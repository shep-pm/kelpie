//! A worker's policy: what its sandbox writes, reads and reaches
//!
//! The PM's shape (see [`super::policy`]), composed as SK's `WorkerProfile::reach`
//! composed it (`docs/design/workers.md`, The worker's policy):
//!
//! - **Writes:** the worktree, its clone's git dir less `CLONE_DENY`, and the
//!   state folder, which holds the harness's config folder. The harnesses' own
//!   files in the worktree are denied inside those writes ([`fence::deny_write`]).
//! - **Reads:** the PM's, plus the worktree and its git dirs, kelpie's own binary
//!   and the toolchain homes a build runs from, with cargo's credentials denied.
//! - **Network:** the PM's, plus GitHub, which a push reaches.
//! - **Sockets:** kelpie's tools door, and the lease socket in the state folder.
//!
//! Every harness's login, the secrets any tool keeps in the home folder and the
//! keychain are denied as for the PM, and kelpie's own data folder is closed but
//! for the folders the worker works in.

use std::collections::HashSet;
use std::env;
use std::net::SocketAddr;
use std::path::Path;

use crabpot::{Config, ConfigError};
use serde_json::json;

use self::toolchain::{Env, ProcessEnv, cargo_credentials, toolchain_homes};
use super::fence;
use super::policy::{
    KEYCHAIN_SERVICES, Project, Reads, TOOL_LOGS, denied, home, hosts, reads, text,
};
use crate::session::Harness;
use crate::tools::Door;

mod toolchain;

/// One worker's folders, and the door it reaches kelpie's tools through
#[derive(Debug, Clone, Copy)]
pub struct Worker<'a> {
    /// The work item's worktree, which the worker writes
    pub worktree: &'a Path,
    /// The clone's own git dir, which the worker writes less `CLONE_DENY`
    pub git_dir: &'a Path,
    /// The project checkout's git dir, whose objects the clone borrows. The worker
    /// reads it and never writes it.
    pub git_common_dir: &'a Path,
    /// The worker's state folder, which holds its harness's config folder and the
    /// lease socket
    pub state_dir: &'a Path,
    /// Kelpie's data folder, which holds every project's state and is closed but
    /// for the folders above
    pub kelpie_home: &'a Path,
    /// Kelpie's tools door
    pub door: Option<&'a Door>,
    /// The loopback address the lease is served on, for a platform whose sandbox
    /// reaches no Unix socket (Linux). Elsewhere the lease is [`LEASE_SOCKET`]
    /// and this is ignored.
    pub lease_address: Option<SocketAddr>,
}

/// What a clone's own git dir denies the worker: the parts that run code outside
/// the sandbox when changed, and the alternates that lend it another repo's objects
const CLONE_DENY: [&str; 5] = ["config", "hooks", "info", "modules", "objects/info"];

/// What the worker reaches besides its harness's model hosts and the project's
/// allowlist: what `git push` and `gh` talk to
const GITHUB: [&str; 2] = ["github.com", "api.github.com"];

/// The socket in the state folder that `kelpie lease run` reaches kelpie through,
/// so a worker's `cargo test` waits its turn behind the others'
///
/// The name is short: a Unix socket's path is limited to about a hundred bytes.
/// Linux cannot reach a Unix socket from the sandbox, so there the lease is
/// [`Worker::lease_address`] instead.
pub const LEASE_SOCKET: &str = "lease.sock";

/// What SK's `CREDENTIALS` holds beyond the secrets the PM's policy denies: the
/// files of SK's old home, which the maintainer's disk still holds a project's
/// state and settings and the authenticator secret in
const SK_FILES: [&str; 3] = [
    "~/.kelpie/projects/**",
    "~/.kelpie/settings.toml",
    "~/.kelpie/totp/**",
];

/// The settings a worker on `harness` runs under, in `project`
///
/// Paths are taken as UTF-8: a part of one that is not is replaced as the OS
/// shows it.
///
/// # Errors
///
/// A domain in the project's allowlist, or a host of the harness's, that crabpot
/// cannot read as one, or a path it cannot use.
pub fn policy(
    harness: &Harness,
    project: &Project<'_>,
    worker: &Worker<'_>,
) -> Result<Config, ConfigError> {
    compose(harness, project, worker, home().as_deref(), &ProcessEnv)
}

/// [`policy`] for a person whose home folder is `home` and whose tools read `env`
fn compose(
    harness: &Harness,
    project: &Project<'_>,
    worker: &Worker<'_>,
    home: Option<&Path>,
    env: &dyn Env,
) -> Result<Config, ConfigError> {
    // A tool home is read back whole, so each cargo home's own credential files
    // are named with it: `SECRETS` spells the default home alone, and `CARGO_HOME`
    // may point anywhere the maintainer keeps it.
    let tools = home.map_or_else(Vec::new, |home| toolchain_homes(home, env));
    let closed = project.reads == Reads::Project;
    // Kelpie's data folder holds every project's state and settings, and the
    // sessions' folder every session's login, so both are closed whatever the
    // project reads, and the home folder with them unless the project reads
    // anywhere. What the worker works in is allowed back below.
    let closed_home = home.filter(|_| closed);
    let mut deny_read: Vec<String> = denied()
        .into_iter()
        .chain(SK_FILES)
        .map(str::to_owned)
        .chain(cargo_credentials(&tools))
        .chain(home_denials(
            closed_home
                .into_iter()
                .chain([worker.kelpie_home, project.sessions_root]),
        ))
        .collect();
    if closed && home.is_none() {
        // crabpot refuses `~` where no home folder is set, rather than leave it open
        deny_read.push("~".to_owned());
    }
    let mut allow_read: Vec<String> = [worker.worktree, worker.git_dir, worker.git_common_dir]
        .into_iter()
        .map(text)
        .collect();
    allow_read.extend(reads(harness, project, worker.state_dir));
    if closed {
        allow_read.extend(env::current_exe().ok().map(|kelpie| text(&kelpie)));
        allow_read.extend(tools.iter().map(|tool| text(tool)));
    }
    let mut seen = HashSet::new();
    allow_read.retain(|folder| seen.insert(folder.clone()));

    // Inside the writes: the clone's own git dir less what runs code, and each
    // harness's files in the worktree
    let no_write: Vec<String> = CLONE_DENY
        .iter()
        .map(|part| text(&worker.git_dir.join(part)))
        .chain(
            fence::deny_write(worker.worktree)
                .iter()
                .map(|path| text(path)),
        )
        // crabpot makes two tools' log folders writable under a home it does not close
        .chain(TOOL_LOGS.iter().map(|path| (*path).to_owned()))
        .collect();

    let mut hosts = hosts(harness, project);
    for host in GITHUB {
        if !hosts.iter().any(|held| held == host) {
            hosts.push(host.to_owned());
        }
    }
    let mut network = json!({
        "allowedDomains": hosts,
        "deniedDomains": [],
        // A host off the list is refused rather than asked about: the ask is
        // the sandbox request prompt's, not the proxy's
        "strictAllowlist": true,
    });
    if cfg!(target_os = "linux") {
        // Linux turns every Unix socket off whatever `allowUnixSockets` lists
        // (`tests/sandbox/escapes.rs`), so the door and the lease are both
        // loopback addresses, as the PM's door is
        let addresses: Vec<String> = worker
            .door
            .and_then(Door::tcp)
            .into_iter()
            .chain(worker.lease_address)
            .map(|address| address.to_string())
            .collect();
        if !addresses.is_empty() {
            network["allowTcp"] = json!(addresses);
        }
    } else {
        let mut sockets = vec![text(&worker.state_dir.join(LEASE_SOCKET))];
        sockets.extend(worker.door.and_then(Door::socket).map(text));
        network["allowUnixSockets"] = json!(sockets);
    }
    // Only macOS has a keychain to ask
    if cfg!(target_os = "macos") {
        network["denyMachLookup"] = json!(KEYCHAIN_SERVICES);
    }
    let write = [worker.worktree, worker.git_dir, worker.state_dir].map(text);
    let settings = json!({
        "network": network,
        "filesystem": {
            "denyRead": deny_read,
            "allowRead": allow_read,
            "allowWrite": write,
            "denyWrite": no_write,
        },
    });
    Config::from_json(&settings.to_string())
}

/// Each of `homes` as a deny over the folder whole, less one another already
/// covers: denying `/k` needs no rule for `/k/projects`
fn home_denials<'a>(homes: impl IntoIterator<Item = &'a Path>) -> Vec<String> {
    let mut kept: Vec<&Path> = Vec::new();
    for home in homes {
        if kept.iter().any(|held| home.starts_with(held)) {
            continue;
        }
        kept.retain(|held| !held.starts_with(home));
        kept.push(home);
    }
    kept.into_iter()
        .map(|home| format!("{}/**", home.display()))
        .collect()
}

#[cfg(test)]
mod tests;
