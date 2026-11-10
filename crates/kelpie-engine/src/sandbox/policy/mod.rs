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

use std::env;
use std::path::{Path, PathBuf};

use crabpot::{Config, ConfigError};
use serde_json::json;

use self::network::network;
use crate::session::Harness;
use crate::tools::Door;

mod network;
mod reads;
mod secrets;

pub(super) use self::network::{KEYCHAIN_SERVICES, hosts};
pub use self::reads::Reads;
pub(super) use self::reads::reads;
pub use self::secrets::{denied, holds_home, in_denied};

/// The folders crabpot makes writable for a tool's own logs wherever nothing
/// closes the home folder over them
///
/// kelpie's writes are the state folder alone, so they are denied whatever the
/// session reads.
pub(super) const TOOL_LOGS: [&str; 2] = ["~/.npm/_logs", "~/.claude/debug"];

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
    let network = network(harness, project, door);
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
