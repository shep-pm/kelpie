//! What a session may read, write and reach, in crabpot's settings shape
//!
//! The settings are srt's (`crabpot::Config`), which crabpot reads and kelpie
//! builds from the session's harness, its project and its state folder (ADR 0005).
//! Reads are wide and writes narrow:
//!
//! - **Reads:** crabpot allows every read the policy does not deny, so a policy
//!   sets its reads by its deny list. [`Reads::Project`], the default, closes the
//!   home folder whole and the folder holding every session's state, and opens
//!   back inside them the project, the folders beside it, the session's own state
//!   folder and the harness's own install. Every other folder on the machine is
//!   read at either setting. [`Reads::Anywhere`], which the project's *Read
//!   anywhere by default* setting asks for, leaves the home folder open.
//! - **Writes:** the state folder alone, with the log folders crabpot would
//!   otherwise make writable under the home folder denied back.
//! - **Network:** the harness's model hosts plus the project's allowlist, and
//!   nothing else gets through: a host off the list is refused, not asked about.
//!
//! The logins and secrets are closed whatever else is read, and every
//! harness's login is closed to every other harness's session (ADR 0006). A
//! project whose folder is the home folder or a folder above it holds them all,
//! so [`Sandbox::start`](crate::sandbox::Sandbox::start) refuses it before the
//! session starts.

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crabpot::{Config, ConfigError};
use serde_json::json;

use crate::session::Harness;

/// The files in the home folder that hold each harness's login, and whose
/// session may not read any of them: a harness gets its own login through its
/// environment instead
///
/// Lifted from SK's `OwnFiles::credentials`, with each harness named as SK
/// named it.
const LOGINS: [(&str, &[&str]); 4] = [
    (
        "Claude Code",
        // Linux keeps the login in the file; `.claude.json` holds MCP servers' secrets
        &["~/.claude/.credentials.json", "~/.claude.json"],
    ),
    ("Codex", &["~/.codex/**"]),
    ("pi", &["~/.pi/**"]),
    (
        "goose",
        // Its config, secrets and session database sit in four places, by platform
        &[
            "~/.config/goose/**",
            "~/.local/share/goose/**",
            "~/.local/state/goose/**",
            "~/Library/Application Support/Block/goose/**",
        ],
    ),
];

/// The secrets any tool keeps in the home folder
///
/// Lifted from SK's `profile::CREDENTIALS`, less the entries for SK's own old
/// home. `~/.config/gh` is added: the PM never holds a GitHub token (ADR 0005).
/// So is macOS's keychain folder, where `gh` and git's helper keep a login, and
/// where kelpie's own logins are stored (ADR 0008).
const SECRETS: [&str; 11] = [
    "~/.ssh/**",
    "~/.aws/**",
    "~/.gnupg/**",
    "~/.docker/**",
    "~/.config/gh/**",
    "~/.netrc",
    "~/.git-credentials",
    "~/.npmrc",
    "~/.cargo/credentials",
    "~/.cargo/credentials.toml",
    "~/Library/Keychains/**",
];

/// The macOS services that answer for the keychain, which a session never asks
const KEYCHAIN_SERVICES: [&str; 2] = ["com.apple.securityd.xpc", "com.apple.SecurityServer"];

/// The folders crabpot makes writable for a tool's own logs wherever nothing
/// closes the home folder over them
///
/// kelpie's writes are the state folder alone, so they are denied whatever the
/// session reads.
const TOOL_LOGS: [&str; 2] = ["~/.npm/_logs", "~/.claude/debug"];

/// How far a session reads, from the project's `read-anywhere` setting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    /// The project, the folders beside it, the session's state folder and the
    /// harness's own install, with the rest of the home folder closed
    Project,

    /// The whole file system, less every harness's login, every secret and every
    /// other session's state
    Anywhere,
}

impl Reads {
    /// The reads the project's `read-anywhere` setting asks for
    ///
    /// A change lands at the session's next start, since crabpot widens no
    /// running sandbox.
    #[must_use]
    pub const fn from_setting(read_anywhere: bool) -> Self {
        if read_anywhere {
            Self::Anywhere
        } else {
            Self::Project
        }
    }
}

/// The project a session works in, and what its settings say about the sandbox
#[derive(Debug, Clone, Copy)]
pub struct Project<'a> {
    /// The project's folder, which is read, and read beside
    pub folder: &'a Path,

    /// Domains the project allows besides the harness's model hosts
    /// (`network-allowlist` in the project's settings)
    pub network_allowlist: &'a [String],

    /// How far the session reads (`read-anywhere` in the project's settings)
    pub reads: Reads,

    /// The folder that holds every session's state folder, such as
    /// `<data_dir>/projects`: it stays closed whatever the session reads
    pub sessions_root: &'a Path,
}

/// Every harness's login and every secret, as paths and globs under `~/`
///
/// The same list for a session on any harness: the session's own harness gets
/// its login through its environment, so none of these is one it needs.
#[must_use]
pub fn denied() -> Vec<&'static str> {
    LOGINS
        .iter()
        .flat_map(|(_, paths)| paths.iter().copied())
        .chain(SECRETS)
        .collect()
}

/// The settings a session on `harness` runs under, in `project`, keeping its
/// files in `state_dir`
///
/// Paths are taken as UTF-8: a part of one that is not is replaced as the OS
/// shows it.
///
/// # Errors
///
/// A domain in the project's allowlist, or a host of the harness's, that
/// crabpot cannot read as one, or a path it cannot use.
pub fn policy(
    harness: &Harness,
    project: &Project<'_>,
    state_dir: &Path,
) -> Result<Config, ConfigError> {
    let mut deny_read: Vec<String> = denied().into_iter().map(str::to_owned).collect();
    // A session's state folder holds its harness's config folder, which can hold
    // a login. Each session reads its own, and no one else's.
    deny_read.push(glob(project.sessions_root));
    if project.reads == Reads::Project {
        deny_read.push("~".to_owned());
    }
    let mut network = json!({
        "allowedDomains": hosts(harness, project),
        "deniedDomains": [],
        // A host off the list is refused rather than asked about: the ask is
        // the sandbox request prompt's, not the proxy's
        "strictAllowlist": true,
    });
    // Only macOS has a keychain to ask
    if cfg!(target_os = "macos") {
        network["denyMachLookup"] = json!(KEYCHAIN_SERVICES);
    }
    let settings = json!({
        "network": network,
        "filesystem": {
            "denyRead": deny_read,
            "allowRead": reads(harness, project, state_dir),
            "allowWrite": [text(state_dir)],
            "denyWrite": TOOL_LOGS,
        },
    });
    Config::from_json(&settings.to_string())
}

/// The project folder when it is the home folder or a folder above it
///
/// Such a folder holds the logins and secrets the policy denies under the home
/// folder, so a sandbox over it would read them along with the project.
/// [`Sandbox::start`](crate::sandbox::Sandbox::start) refuses one before the
/// session starts, and the guard in `reads` keeps any other allow off the home
/// folder.
#[must_use]
pub fn holds_home(project: &Project<'_>) -> Option<PathBuf> {
    home()
        .filter(|home| home.starts_with(project.folder))
        .map(|_| project.folder.to_path_buf())
}

/// The folders opened back inside what the session's reads close
///
/// crabpot lands every allow after every deny, so a folder at or above a closed
/// one would open what it hides: the logins under the home folder, or the state
/// of every other session. This is the second line: [`holds_home`] refuses the
/// project folder itself when it holds the home folder.
fn reads(harness: &Harness, project: &Project<'_>, state_dir: &Path) -> Vec<String> {
    let closed = closed(project);
    let mut folders = vec![project.folder];
    // The folders beside the project: its sibling repos and worktrees. Named
    // only where the reads are closed to the project, and never when they are
    // the root, which would open everything.
    if project.reads == Reads::Project
        && let Some(beside) = project.folder.parent()
        && beside.parent().is_some()
    {
        folders.push(beside);
    }
    folders.push(state_dir);
    folders.extend(harness.installs.iter().map(PathBuf::as_path));
    let mut seen = HashSet::new();
    folders
        .into_iter()
        .filter(|folder| !closed.iter().any(|held| held.starts_with(folder)))
        .filter(|folder| seen.insert(folder.to_path_buf()))
        .map(text)
        .collect()
}

/// The folders an allow may not sit at or above, each of which holds something
/// the policy denies under it
fn closed(project: &Project<'_>) -> Vec<PathBuf> {
    let mut closed = vec![project.sessions_root.to_path_buf()];
    closed.extend(home());
    closed
}

/// The harness's model hosts, then the project's allowlist, each once
fn hosts(harness: &Harness, project: &Project<'_>) -> Vec<String> {
    let mut seen = HashSet::new();
    harness
        .hosts
        .iter()
        .chain(project.network_allowlist)
        .filter(|host| seen.insert(host.as_str()))
        .cloned()
        .collect()
}

/// The person's home folder
fn home() -> Option<std::path::PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(Into::into)
}

/// A deny over `path` whole, as SK's `profile::home_denials` spells one
fn glob(path: &Path) -> String {
    format!("{}/**", text(path))
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
