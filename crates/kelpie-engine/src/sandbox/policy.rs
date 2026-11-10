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
//!   folder, the harness's own install, and each entry of the person's global
//!   folder their harness linked into the session. At either setting an allow is
//!   kept off every folder at or inside a login or a secret, which would open it,
//!   and a linked entry is kept off one only when it is itself that login or
//!   secret. Every other folder on the machine is read at either setting.
//!   [`Reads::Anywhere`], which the project's *Read anywhere by default* setting
//!   asks for, leaves the home folder open.
//! - **Writes:** the state folder alone, with the log folders crabpot would
//!   otherwise make writable under the home folder denied back, and each entry
//!   linked in from the person's global folder denied by its own name besides, so
//!   a harness cannot change their own file through a link.
//! - **Network:** the harness's model hosts plus the project's allowlist, and
//!   nothing else gets through: a host off the list is refused, not asked about.
//! - **The door:** kelpie's tools door, when a session has one, and that one
//!   address alone: a Unix socket's path on macOS (`allowUnixSockets`), a
//!   `127.0.0.1` address and port on Linux (`allowTcp`). `allowLocalBinding`
//!   stays off.
//!
//! The logins and secrets are closed whatever else is read, and every
//! harness's login is closed to every other harness's session (ADR 0006). A
//! project whose folder is the home folder or a folder above it holds them all,
//! and one at or inside a login or a secret folder is that folder to its own
//! sandbox, so [`Sandbox::start`](crate::sandbox::Sandbox::start) refuses both
//! before the session starts. Folders are compared as crabpot matches them,
//! with their links resolved: a project reaches the home folder through a link
//! as surely as by name.

use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};

use crabpot::{Config, ConfigError};
use serde_json::json;

use crate::session::Harness;
use crate::tools::Door;

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
pub(super) const KEYCHAIN_SERVICES: [&str; 2] =
    ["com.apple.securityd.xpc", "com.apple.SecurityServer"];

/// The folders crabpot makes writable for a tool's own logs wherever nothing
/// closes the home folder over them
///
/// kelpie's writes are the state folder alone, so they are denied whatever the
/// session reads.
pub(super) const TOOL_LOGS: [&str; 2] = ["~/.npm/_logs", "~/.claude/debug"];

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

/// Each entry of [`denied`] as the folder or file it really names
///
/// `~` is the person's home folder, a trailing `/**` is dropped as crabpot drops
/// it, and each path is resolved the way a folder is compared here. An allow at
/// or inside one of these opens it, so no allow lands on one and a project
/// inside one is refused.
#[must_use]
pub fn denied_folders() -> Vec<PathBuf> {
    let Some(home) = home() else {
        return Vec::new();
    };
    denied()
        .into_iter()
        .filter_map(|entry| denied_path(entry, &home))
        .map(|path| real(&path))
        .collect()
}

/// The entry `entry` of [`denied`] as a path under `home`
///
/// None when it names nothing: an empty `home` makes `~/.ssh` the relative
/// `.ssh`, which is a folder of wherever the session was started.
fn denied_path(entry: &str, home: &Path) -> Option<PathBuf> {
    let entry = entry.strip_suffix("/**").unwrap_or(entry);
    let path = match entry.strip_prefix('~') {
        Some(tail) => home.join(tail.strip_prefix('/').unwrap_or(tail)),
        None => PathBuf::from(entry),
    };
    path.is_absolute().then_some(path)
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
    policy_with_door(harness, project, state_dir, None)
}

/// [`policy`] with kelpie's tools `door` named, for a session that reaches them
///
/// The door is the one extra route out: its socket (macOS) or its address
/// (Linux) and nothing beside it, and the harness starts the relay inside the
/// sandbox, so the relay's own binary is read too when the home folder is
/// closed.
///
/// # Errors
///
/// As [`policy`], and the door is one this platform's crabpot cannot grant: a
/// TCP address on macOS, where Seatbelt cannot narrow `allowTcp` to one host.
pub fn policy_with_door(
    harness: &Harness,
    project: &Project<'_>,
    state_dir: &Path,
    door: Option<&Door>,
) -> Result<Config, ConfigError> {
    let mut deny_read: Vec<String> = denied().into_iter().map(str::to_owned).collect();
    // A session's state folder holds its harness's config folder, which can hold
    // a login. Each session reads its own, and no one else's.
    deny_read.push(glob(project.sessions_root));
    // The session's own state folder, and each entry linked in from the person's
    // global folder, are carved back out of what the reads close, so they are
    // named as allows at either setting: goose keeps its own config inside a
    // folder the policy denies as a login folder whatever the project reads.
    let mut allow_read = reads(harness, project, state_dir);
    if project.reads == Reads::Project {
        deny_read.push("~".to_owned());
        // The harness starts the relay inside the sandbox, so its binary is read
        // too when the home folder is closed.
        if door.is_some()
            && let Ok(relay) = env::current_exe()
        {
            allow_read.push(text(&relay));
        }
    }
    let mut network = json!({
        "allowedDomains": hosts(harness, project),
        "deniedDomains": [],
        // A host off the list is refused rather than asked about: the ask is
        // the sandbox request prompt's, not the proxy's
        "strictAllowlist": true,
    });
    if let Some(door) = door {
        if let Some(socket) = door.socket() {
            network["allowUnixSockets"] = json!([text(socket)]);
        }
        if let Some(address) = door.tcp() {
            network["allowTcp"] = json!([address.to_string()]);
        }
    }
    // Only macOS has a keychain to ask
    if cfg!(target_os = "macos") {
        network["denyMachLookup"] = json!(KEYCHAIN_SERVICES);
    }
    let settings = json!({
        "network": network,
        "filesystem": {
            "denyRead": deny_read,
            "allowRead": allow_read,
            "allowWrite": [text(state_dir)],
            // The log folders crabpot would otherwise make writable under the home
            // folder, plus each entry linked in from the person's global folder: a
            // link is a name in the session's folder for a file of the person's own,
            // so it is denied by its own name too, and a harness writing through it
            // is refused whether the OS checks the link or what it points at
            "denyWrite": TOOL_LOGS
                .iter()
                .map(|folder| (*folder).to_owned())
                .chain(harness.global_reads.iter().map(|entry| text(entry)))
                .collect::<Vec<String>>(),
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
/// folder. The refusal names the folder as the person chose it, and both compare
/// the folder it really names, as crabpot matches it.
#[must_use]
pub fn holds_home(project: &Project<'_>) -> Option<PathBuf> {
    home()
        .filter(|home| real(home).starts_with(real(project.folder)))
        .map(|_| project.folder.to_path_buf())
}

/// The project folder and the denied folder it is at or inside, when it is
///
/// A project at `~/.codex/skills/<name>` has Codex's own folder as the folder
/// beside it, so its sandbox would be asked to read the login whole.
/// [`Sandbox::start`](crate::sandbox::Sandbox::start) refuses it before the
/// session starts, and the guard in `reads` keeps any other allow off it. The
/// refusal names the folder as the person chose it, and both compare the folder
/// it really names, as crabpot matches it.
#[must_use]
pub fn in_denied(project: &Project<'_>) -> Option<(PathBuf, PathBuf)> {
    let reached = real(project.folder);
    denied_folders()
        .into_iter()
        .find(|held| reached.starts_with(held))
        .map(|held| (project.folder.to_path_buf(), held))
}

/// The folders opened back inside what the session's reads close
///
/// crabpot masks a denied path again when an allowed folder sits above it, but
/// an allow at or inside a denied folder opens it: a deny whose spelling ends in
/// `/**` is the folder itself, and only a deny strictly under an allow is landed
/// after it. So an allow is kept off every folder at or inside a login or a
/// secret, and off every one at or above a closed folder, which would open what
/// it hides: the logins under the home folder, or the state of every other
/// session. This is the second line: [`holds_home`] and [`in_denied`] refuse the
/// project folder itself.
///
/// An allow is kept off a closed folder by the folder it really names, since
/// crabpot matches real paths, and is named as the session was given it.
///
/// The one allow meant to sit inside a closed folder is an entry of the person's
/// global folder their harness linked into the session: it is named as the single
/// file or folder, never as the global folder whole, which holds their login and
/// every secret. crabpot lands every allow after every deny, so the name carves
/// only itself out of the deny around it.
pub(super) fn reads(harness: &Harness, project: &Project<'_>, state_dir: &Path) -> Vec<String> {
    let closed = closed(project);
    let denied = denied_folders();
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
    let mut allowed: Vec<String> = folders
        .into_iter()
        .filter(|folder| {
            let reached = real(folder);
            !closed.iter().any(|held| held.starts_with(&reached))
                && !denied.iter().any(|held| reached.starts_with(held))
                && seen.insert(reached)
        })
        .map(text)
        .collect();
    // One entry per entry of the person's global folder their harness linked into
    // the session, never the folder whole: that folder holds their login and every
    // secret, which stay denied. Each sits inside a folder the policy denies (goose
    // keeps its config beside its keys), so the #108 filter above would refuse it
    // and it is checked on its own: refused when it is itself a login or a secret,
    // or at or above a closed folder, which it would open.
    allowed.extend(
        harness
            .global_reads
            .iter()
            .filter(|entry| {
                let reached = real(entry);
                !closed.iter().any(|held| held.starts_with(&reached))
                    && !denied.contains(&reached)
                    && seen.insert(reached)
            })
            .map(|entry| text(entry)),
    );
    allowed
}

/// The folders an allow may not sit at or above, each of which holds something
/// the policy denies under it, as crabpot matches them
fn closed(project: &Project<'_>) -> Vec<PathBuf> {
    let mut closed = vec![real(project.sessions_root)];
    closed.extend(home().map(|home| real(&home)));
    closed
}

/// The harness's model hosts, then the project's allowlist, each once
pub(super) fn hosts(harness: &Harness, project: &Project<'_>) -> Vec<String> {
    let mut seen = HashSet::new();
    harness
        .hosts
        .iter()
        .chain(project.network_allowlist)
        .filter(|host| seen.insert(host.as_str()))
        .cloned()
        .collect()
}

/// `path` as a sandbox matches it: every link on the way to it resolved
///
/// A folder reaches a closed folder through a link as surely as by name: `HOME`
/// is `/home/<name>`, `/home` is a link to `/data/home`, and the project is
/// `/data/home/<name>`. crabpot resolves the paths it matches, so a comparison
/// written as the paths are spelled sees no match and lets the allow through.
///
/// A folder that is not there yet is left as written, since it can hold no
/// folder that is. A Windows verbatim prefix is not stripped, as kelpie has no
/// Windows sandbox yet.
fn real(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

/// The person's home folder
pub(super) fn home() -> Option<std::path::PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(Into::into)
}

/// A deny over `path` whole, as SK's `profile::home_denials` spells one
pub(super) fn glob(path: &Path) -> String {
    format!("{}/**", text(path))
}

pub(super) fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
