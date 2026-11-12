//! A reviewer's policy: what its sandbox writes, reads and reaches
//!
//! The worker's shape (see [`super::worker`]), cut down to a session that runs no
//! command and changes no file of the work (`docs/design/review.md`, Reviewers):
//!
//! - **Writes:** the state folder, which holds the harness's config folder, and
//!   the tests folder when the project has a Test command. Nothing else: the
//!   worktree, its clone's git dir and the checkout's git dir are read only.
//! - **Reads:** the worker's, less kelpie's own binary and the toolchain homes,
//!   which only a command runs from, plus the tests folder. A toolchain home's
//!   cargo credentials stay denied, as for the worker.
//! - **Network:** the harness's own hosts and the project's allowlist. GitHub is
//!   not named, since a reviewer pushes nothing and asks the forge nothing.
//! - **Sockets:** none. A reviewer has no tools door and no lease socket.
//!
//! Every harness's login, the secrets any tool keeps in the home folder and the
//! keychain are denied as for the worker, and kelpie's own data folder is closed
//! but for the state folder.

use std::path::Path;

use crabpot::{Config, ConfigError};
use serde_json::json;

use super::policy::{KEYCHAIN_SERVICES, Project, TOOL_LOGS, home, hosts, text};
use super::worker::{Env, Folders, ProcessEnv, read_rules};
use crate::session::Harness;
use crate::worktree::clone::git_dir_of;

/// One reviewer's folders
#[derive(Debug, Clone, Copy)]
pub struct Reviewer<'a> {
    /// The work item's worktree, which the reviewer reads
    pub worktree: &'a Path,
    /// The project checkout's git dir, whose objects the worktree's clone borrows.
    /// The clone's own git dir is the worktree's sibling ([`git_dir_of`]).
    pub git_common_dir: &'a Path,
    /// The reviewer's state folder, which holds its harness's config folder
    pub state_dir: &'a Path,
    /// The one folder the reviewer writes tests in, which a project with a Test
    /// command gives it and a project without one does not
    pub tests: Option<&'a Path>,
    /// Kelpie's data folder, which holds every project's state and is closed but
    /// for the folders above
    pub kelpie_home: &'a Path,
}

/// The settings a reviewer on `harness` runs under, in `project`
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
    reviewer: &Reviewer<'_>,
) -> Result<Config, ConfigError> {
    compose(harness, project, reviewer, home().as_deref(), &ProcessEnv)
}

/// [`policy`] for a person whose home folder is `home` and whose tools read `env`
fn compose(
    harness: &Harness,
    project: &Project<'_>,
    reviewer: &Reviewer<'_>,
    home: Option<&Path>,
    env: &dyn Env,
) -> Result<Config, ConfigError> {
    let clone_git_dir = git_dir_of(reviewer.worktree);
    let mut read = vec![
        reviewer.worktree,
        clone_git_dir.as_path(),
        reviewer.git_common_dir,
    ];
    read.extend(reviewer.tests);
    let folders = Folders {
        read: &read,
        state_dir: reviewer.state_dir,
        kelpie_home: reviewer.kelpie_home,
    };
    let rules = read_rules(harness, project, &folders, home, env, false);

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
    let write: Vec<String> = [reviewer.state_dir]
        .into_iter()
        .chain(reviewer.tests)
        .map(text)
        .collect();
    let no_write: Vec<String> = TOOL_LOGS
        .iter()
        .map(|path| (*path).to_owned())
        // A link is a name in the reviewer's config folder for a file of the person's
        // own, so it is denied by that name as well as by what it points at
        .chain(harness.global_reads.iter().map(|entry| text(entry)))
        .collect();
    let settings = json!({
        "network": network,
        "filesystem": {
            "denyRead": rules.deny,
            "allowRead": rules.allow,
            "allowWrite": write,
            "denyWrite": no_write,
        },
    });
    Config::from_json(&settings.to_string())
}

#[cfg(test)]
mod tests;
