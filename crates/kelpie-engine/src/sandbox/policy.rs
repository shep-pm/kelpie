//! What a session may read, write and reach, in crabpot's settings shape
//!
//! The settings are srt's (`crabpot::Config`), which crabpot reads and kelpie
//! builds from the session's harness, its project and its state folder (ADR 0005).
//! Reads are wide and writes narrow:
//!
//! - **Reads:** the project, the folders beside it, the session's state folder,
//!   the harness's own install, and the entries of the person's global folder
//!   their harness linked into the session. The rest of the home folder is
//!   closed. With the project's *Read anywhere by default* setting on, only the
//!   logins and secrets are closed.
//! - **Writes:** the state folder alone. Each entry linked in from the person's
//!   global folder is denied by its own name besides, so a harness cannot change
//!   their own file through a link.
//! - **Network:** the harness's model hosts plus the project's allowlist, and
//!   nothing else gets through: a host off the list is refused, not asked about.
//! - **The door:** kelpie's tools door, when a session has one, and that one
//!   address alone: a Unix socket's path on macOS (`allowUnixSockets`), a
//!   `127.0.0.1` address and port on Linux (`allowTcp`). `allowLocalBinding`
//!   stays off.
//!
//! The logins and secrets are closed whatever else is read, and every
//! harness's login is closed to every other harness's session (ADR 0006).

use std::collections::HashSet;
use std::env;
use std::path::Path;

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
const SECRETS: [&str; 10] = [
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
];

/// The macOS services that answer for the keychain, which a session never asks
const KEYCHAIN_SERVICES: [&str; 2] = ["com.apple.securityd.xpc", "com.apple.SecurityServer"];

/// The project a session works in, and what its settings say about the sandbox
#[derive(Debug, Clone, Copy)]
pub struct Project<'a> {
    /// The project's folder, which is read, and read beside
    pub folder: &'a Path,
    /// Domains the project allows besides the harness's model hosts
    /// (`network-allowlist` in the project's settings)
    pub network_allowlist: &'a [String],
    /// Whether the project reads anywhere but its logins and secrets
    /// (`read-anywhere` in the project's settings)
    pub read_anywhere: bool,
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
    // The linked entries are named as allows whatever the project's *Read anywhere
    // by default* setting says: goose keeps its own config inside a folder the policy
    // denies as a login folder, and an allow is what crabpot carves that deny around
    let allow_read: Vec<String> = if project.read_anywhere {
        harness
            .global_reads
            .iter()
            .map(|entry| text(entry))
            .collect()
    } else {
        deny_read.push("~".to_owned());
        let mut folders = reads(harness, project, state_dir);
        // The relay the harness starts inside the sandbox is read too, so the door
        // reaches kelpie's tools with the home folder closed
        if door.is_some()
            && let Ok(relay) = env::current_exe()
        {
            folders.push(text(&relay));
        }
        folders
    };
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
            // A link is a name in the session's folder for a file of the person's own.
            // Each one is denied by its own name too, so a harness writing through it
            // is refused whether the OS checks the link or what it points at
            "denyWrite": harness
                .global_reads
                .iter()
                .map(|entry| text(entry))
                .collect::<Vec<String>>(),
        },
    });
    Config::from_json(&settings.to_string())
}

/// The folders a session reads inside the closed home folder
fn reads(harness: &Harness, project: &Project<'_>, state_dir: &Path) -> Vec<String> {
    let mut folders = vec![text(project.folder)];
    // The folders beside the project: its sibling repos and worktrees. Not when
    // they are the home folder or the root, which would open everything.
    if let Some(beside) = project.folder.parent()
        && beside.parent().is_some()
        && Some(beside) != home().as_deref()
    {
        folders.push(text(beside));
    }
    folders.push(text(state_dir));
    folders.extend(harness.installs.iter().map(|folder| text(folder)));
    // One entry per linked entry of the person's global folder, never the folder
    // whole: it holds their login and secrets, which stay denied
    folders.extend(harness.global_reads.iter().map(|entry| text(entry)));
    let mut seen = HashSet::new();
    folders.retain(|folder| seen.insert(folder.clone()));
    folders
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

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
