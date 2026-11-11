//! The app's opener for a worker: its session on the harness its settings name
//!
//! [`open`] is what [`kelpie_engine::worker::start`] calls to build a worker's
//! session, as [`pm::open`](crate::pm::open) builds the PM's. The engine hands it
//! the worktree to work in, the variables the worker's harness runs with and the
//! worker's lease socket, and it builds the session on the `claude-acp` bridge
//! inside a sandbox started by [`Sandbox::start_worker`].
//!
//! The engine's own `Opening::sandbox` borrows what the start holds, and a bridge
//! starts a sandbox for each version it moves a session to, long after the start
//! has returned. So the opener keeps the folders itself and starts every
//! version's sandbox from them. A worker never runs without a sandbox: where it
//! cannot start, the worker does not either.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use kelpie_engine::bridge;
use kelpie_engine::harness::claude;
use kelpie_engine::lease::SessionLease;
use kelpie_engine::login::Store;
use kelpie_engine::sandbox::{self, Sandbox};
use kelpie_engine::session::{Session, Started, sandboxed_launcher};
use kelpie_engine::settings::{ConfigDir, Harness};
use kelpie_engine::tools::SessionTools;
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::worker::Opening;
use kelpie_engine::worktree::clone;

use super::WorkerPlan;
use crate::pm;

/// What the app builds a worker's session from, besides what the engine hands the opener
#[derive(Debug, Clone, Copy)]
pub struct OpenOn<'a> {
    /// The app's data folder, which holds the installed bridges
    pub data_dir: &'a Path,

    /// Where the person's harness logins are kept
    pub logins: &'a Store,

    /// The folder settings are read from, which says how each bridge is updated
    pub config: &'a ConfigDir,

    /// The project the worker belongs to
    pub project_id: &'a str,

    /// The project's checkout, which the worker's clone borrows objects from
    pub folder: &'a Path,

    /// What the worker runs on, resolved when this start began
    pub plan: &'a WorkerPlan,
}

/// The worker's session on the harness its binding names
///
/// A binding that names no harness runs on Claude Code, kelpie's own default
/// ([`pm::PM_DEFAULT`]), as the PM does.
///
/// # Errors
///
/// The harness is not wired to workers, the state folder cannot be made, the
/// bridge cannot be installed, or the harness is not installed or has no login.
/// The text says which.
pub fn open(on: &OpenOn<'_>, opening: &Opening<'_>) -> Result<Session, String> {
    let harness = opening
        .binding
        .harness
        .value
        .unwrap_or(pm::PM_DEFAULT.harness);
    // parity: harness.worker
    match harness {
        Harness::ClaudeCode => claude_worker(on, opening),
        // `Harness` grows with each harness's slice, and none but Claude Code is built here
        other => Err(format!(
            "{} is not wired to workers yet",
            pm::describe(other, None)
        )),
    }
}

/// What every version's sandbox is started from
struct Folders {
    tree: Arc<ProcessTree>,
    logins: Store,
    harness_global_folder: bool,
    reach: Reach,
    worktree: PathBuf,
    git_dir: PathBuf,
    git_common_dir: PathBuf,
    state_dir: PathBuf,
    kelpie_home: PathBuf,
    tools: Option<SessionTools>,
    lease: Option<SessionLease>,
    env: Vec<(OsString, OsString)>,
}

/// What the sandbox lets the worker read and reach, as the project's settings say
struct Reach {
    folder: PathBuf,
    network_allowlist: Vec<String>,
    reads: sandbox::Reads,
    sessions_root: PathBuf,
}

fn claude_worker(on: &OpenOn<'_>, opening: &Opening<'_>) -> Result<Session, String> {
    let state_dir = opening.state_dir;
    std::fs::create_dir_all(state_dir)
        .map_err(|error| format!("could not make {}: {error}", state_dir.display()))?;
    // kelpie's own pin is always there to fall back to
    bridge::ensure(on.data_dir, claude::pin()).map_err(|error| error.to_string())?;
    let mut env: Vec<(OsString, OsString)> = opening
        .env
        .iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
    env.extend(opening.lease.as_ref().map(SessionLease::env));
    let folders = Folders {
        tree: Arc::clone(opening.tree),
        logins: on.logins.clone(),
        harness_global_folder: on.plan.harness_global_folder,
        reach: Reach {
            folder: on.folder.to_path_buf(),
            network_allowlist: on.plan.network_allowlist.clone(),
            reads: on.plan.reads,
            sessions_root: on.data_dir.join("projects"),
        },
        worktree: opening.worktree.to_path_buf(),
        git_dir: clone::git_dir_of(opening.worktree),
        git_common_dir: git_common_dir(opening.tree, on.folder)?,
        state_dir: state_dir.to_path_buf(),
        kelpie_home: on.data_dir.to_path_buf(),
        tools: opening.tools.clone(),
        lease: opening.lease.clone(),
        env,
    };
    let bridge = pm::claude_bridge(on.data_dir, on.config, on.project_id, move |cmd| {
        folders.start(cmd)
    });
    Ok(Session::on_bridge(
        bridge,
        opening.worktree.to_path_buf(),
        opening.resume.clone(),
        opening.choice.clone(),
    ))
}

impl Folders {
    /// The worker's harness on the bridge `cmd`, and the sandbox it runs in
    fn start(&self, cmd: PathBuf) -> Result<Started, String> {
        let mut harness = claude::harness(
            cmd,
            &self.state_dir,
            &self.logins,
            self.harness_global_folder,
        )
        .map_err(|error| error.to_string())?;
        harness.env.extend(self.env.iter().cloned());
        let reach = sandbox::Project {
            folder: &self.reach.folder,
            network_allowlist: &self.reach.network_allowlist,
            reads: self.reach.reads,
            sessions_root: &self.reach.sessions_root,
        };
        let worker = sandbox::Worker {
            worktree: &self.worktree,
            git_dir: &self.git_dir,
            git_common_dir: &self.git_common_dir,
            state_dir: &self.state_dir,
            kelpie_home: &self.kelpie_home,
            door: self.tools.as_ref().map(SessionTools::door),
            lease: self.lease.as_ref().map(SessionLease::door),
        };
        let sandbox = Sandbox::start_worker(&harness, &reach, &worker)
            .map_err(|error| format!("could not start the worker's sandbox: {error}"))?;
        Ok(Started {
            launch: sandboxed_launcher(
                Arc::clone(&self.tree),
                harness.clone(),
                sandbox,
                self.worktree.clone(),
            ),
            harness,
        })
    }
}

/// The git dir the project's checkout keeps its objects in, which the clone borrows
///
/// Asked of git through the process tree, which is where the engine asks it for
/// the clone.
fn git_common_dir(tree: &ProcessTree, folder: &Path) -> Result<PathBuf, String> {
    let mut git = Command::new("git");
    git.current_dir(folder)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"]);
    let asked = tree
        .output(&mut git)
        .map_err(|error| format!("could not ask git for the project's git dir: {error}"))?;
    if !asked.status.success() {
        return Err(format!(
            "git could not name the project's git dir: {}",
            String::from_utf8_lossy(&asked.stderr).trim()
        ));
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&asked.stdout).trim().to_owned(),
    ))
}
