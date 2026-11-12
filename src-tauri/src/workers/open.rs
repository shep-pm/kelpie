//! The app's opener for a worker: its session on the harness its settings name
//!
//! [`open`] is what [`kelpie_engine::worker::start`] calls to build a worker's
//! session, as [`pm::open`](crate::pm::open) builds the PM's. The engine hands it
//! the worktree to work in, the variables the worker's harness runs with and the
//! worker's lease socket, and it builds the session inside a sandbox started by
//! [`Sandbox::start_worker`]: on Claude Code that means the `claude-acp` bridge,
//! and on goose the person's own `goose acp`, which speaks ACP itself and needs
//! no bridge (ADR 0007).
//!
//! The engine's own `Opening::sandbox` borrows what the start holds, and a bridge
//! starts a sandbox for each version it moves a session to, long after the start
//! has returned. So the opener keeps the folders itself and starts every
//! version's sandbox from them. A goose worker swaps no version, so its sandbox
//! comes from `Opening::sandbox` and covers its whole session. A worker never runs
//! without a sandbox: where it cannot start, the worker does not either.
//!
//! Starting a sandbox only builds crabpot's settings, and bwrap's refusal first
//! shows when something runs inside it. So each start probes the sandbox its
//! session runs in ([`confine`]) before the session is built, and a sandbox bwrap
//! refuses parks the worker Paused with the refusal's words
//! (`docs/design/sandbox-setup.md`, Running without a sandbox).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use kelpie_engine::bridge;
use kelpie_engine::harness::{HarnessError, claude, goose};
use kelpie_engine::lease::SessionLease;
use kelpie_engine::login::Store;
use kelpie_engine::sandbox::{self, Sandbox, SandboxError};
use kelpie_engine::session::{Harness as SessionHarness, Session, Started, sandboxed_launcher};
use kelpie_engine::settings::{ConfigDir, Harness};
use kelpie_engine::tools::SessionTools;
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::worker::Opening;
use kelpie_engine::worktree::clone;

use super::WorkerPlan;
use crate::pm;
use crate::sandbox_setup::proven;

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
/// bridge cannot be installed, a goose worker has no provider set, the harness is
/// not installed or has no login, or the worker's sandbox cannot start. The text
/// says which.
pub fn open(on: &OpenOn<'_>, opening: &Opening<'_>) -> Result<Session, String> {
    open_with(on, opening, goose::harness, goose_session)
}

/// The opener with the goose arm's two steps as `build` and `session`
///
/// `build` is what finds the person's own goose and builds its harness, and
/// `session` is what starts the worker's sandbox around it and opens the session
/// in the worktree. The app hands in [`goose::harness`] and [`goose_session`],
/// which search `PATH` and start crabpot; a test hands in stand-ins, since a unit
/// test has no goose on `PATH` and starts no sandbox.
///
/// # Errors
///
/// As [`open`].
pub(super) fn open_with(
    on: &OpenOn<'_>,
    opening: &Opening<'_>,
    build: impl FnOnce(&Path, goose::Provider, bool) -> Result<SessionHarness, HarnessError>,
    session: impl FnOnce(&Opening<'_>, SessionHarness) -> Result<Session, String>,
) -> Result<Session, String> {
    let harness = opening
        .binding
        .harness
        .value
        .unwrap_or(pm::PM_DEFAULT.harness);
    // parity: harness.worker
    match harness {
        Harness::ClaudeCode => claude_worker(on, opening),
        Harness::Goose => goose_worker(on, opening, build, session),
        // `Harness` grows with each harness's slice, and none but these two are built here
        other => Err(format!(
            "{} is not wired to workers yet",
            pm::describe(other, None)
        )),
    }
}

/// The worker's goose harness, from the plan's provider, carrying the worker's own variables
///
/// goose keeps its config in a folder of kelpie's own under the worker's state
/// folder, so each worker has a goose root of its own and the person's own goose
/// config stays out until they turn *Harness global folder* on.
///
/// # Errors
///
/// The settings name no provider, the state folder cannot be made, or the
/// person's goose is not on `PATH`. The text says which.
fn goose_worker(
    on: &OpenOn<'_>,
    opening: &Opening<'_>,
    build: impl FnOnce(&Path, goose::Provider, bool) -> Result<SessionHarness, HarnessError>,
    session: impl FnOnce(&Opening<'_>, SessionHarness) -> Result<Session, String>,
) -> Result<Session, String> {
    let provider = pm::goose_provider(on.plan.provider.as_deref(), on.plan.host.as_deref())?;
    std::fs::create_dir_all(opening.state_dir)
        .map_err(|error| format!("could not make {}: {error}", opening.state_dir.display()))?;
    let mut harness =
        build(opening.state_dir, provider, on.plan.harness_global_folder).map_err(pm::no_goose)?;
    harness.env.extend(
        opening
            .env
            .iter()
            .map(|(key, value)| (key.into(), value.into())),
    );
    harness
        .env
        .extend(opening.lease.as_ref().map(SessionLease::env));
    session(opening, harness)
}

/// The worker's goose session, inside the sandbox the engine starts for it
///
/// `Opening::sandbox` is [`Sandbox::start_worker`] with the worker's worktree,
/// git dirs, state folder and lease door, which is the policy every worker's
/// sandbox gets whatever its harness is. goose swaps no version, so this one
/// sandbox covers its whole session. It is probed once, before the session is
/// built.
///
/// # Errors
///
/// The worker's sandbox cannot start, or bwrap refuses it. The text says why.
pub(super) fn goose_session(
    opening: &Opening<'_>,
    harness: SessionHarness,
) -> Result<Session, String> {
    goose_session_with(opening, harness, |sandbox| sandbox.ready(opening.tree))
}

/// [`goose_session`] with `ready` as the probe of the sandbox
///
/// The app hands in [`Sandbox::ready`], which runs a command in bwrap. A test
/// hands in its own, since no unit test runs bwrap.
///
/// # Errors
///
/// As [`goose_session`].
pub(super) fn goose_session_with(
    opening: &Opening<'_>,
    harness: SessionHarness,
    ready: impl FnOnce(&Sandbox) -> Result<(), SandboxError>,
) -> Result<Session, String> {
    let sandbox = confine(opening, (opening.sandbox)(&harness), ready)?;
    Ok(Session::sandboxed(
        Arc::clone(opening.tree),
        harness,
        sandbox,
        opening.worktree.to_path_buf(),
        opening.resume.clone(),
        opening.choice.clone(),
    ))
}

/// The started sandbox, once `ready` has run in it and passed
///
/// A refusal is named to `opening`, which is what parks the worker Paused and
/// holds no slot for it ([`Opening::refuse`]). Any other error fails the start
/// and gives the slot back.
///
/// # Errors
///
/// The words of the refusal, or of the error the start or `ready` gave.
pub(super) fn confine<T>(
    opening: &Opening<'_>,
    started: Result<T, SandboxError>,
    ready: impl FnOnce(&T) -> Result<(), SandboxError>,
) -> Result<T, String> {
    proven(started, ready).map_err(|error| match error {
        SandboxError::Refused(refusal) => opening.refuse(refusal),
        other => format!("could not start the worker's sandbox: {other}"),
    })
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
    let pinned = bridge::ensure(on.data_dir, claude::pin()).map_err(|error| error.to_string())?;
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
    // One probe per start, in the sandbox the session runs in. A bridge version
    // swap later starts its sandbox unprobed.
    let probed = folders.harness(pinned)?;
    confine(opening, folders.sandbox(&probed), |sandbox| {
        sandbox.ready(opening.tree)
    })?;
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
    /// The worker's harness on the bridge `cmd`
    fn harness(&self, cmd: PathBuf) -> Result<SessionHarness, String> {
        let mut harness = claude::harness(
            cmd,
            &self.state_dir,
            &self.logins,
            self.harness_global_folder,
        )
        .map_err(|error| error.to_string())?;
        harness.env.extend(self.env.iter().cloned());
        Ok(harness)
    }

    /// The sandbox `harness` runs in, as the worker's policy has it
    fn sandbox(&self, harness: &SessionHarness) -> Result<Sandbox, SandboxError> {
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
        Sandbox::start_worker(harness, &reach, &worker)
    }

    /// The worker's harness on the bridge `cmd`, and the sandbox it runs in
    fn start(&self, cmd: PathBuf) -> Result<Started, String> {
        let harness = self.harness(cmd)?;
        let sandbox = self
            .sandbox(&harness)
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
pub(crate) fn git_common_dir(tree: &ProcessTree, folder: &Path) -> Result<PathBuf, String> {
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
