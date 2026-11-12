//! A sandbox for each session: crabpot around the harness (ADR 0005)
//!
//! Every agent session kelpie starts runs inside crabpot (shep-pm/crabpot), the
//! PM and its workers alike. [`Sandbox::start`] builds the session's
//! [`policy`] (a worker's is [`worker::policy`], through [`Sandbox::start_worker`], and a
//! reviewer's is [`reviewer::policy`], through [`Sandbox::start_reviewer`])
//! and starts crabpot's sandbox from it, and [`Sandbox::command`] wraps the
//! harness's command into one that runs inside it. The process tree
//! spawns the wrapped command as it spawns anything else, so the run's mark
//! reaches it and the sweep finds it (ADR 0001). A project whose folder is the
//! home folder or a folder above it, or is at or inside a login or a secret
//! folder, by name or through a link, is refused at [`Sandbox::start`], at
//! [`Sandbox::start_worker`] and at [`Sandbox::start_reviewer`], since such a
//! folder holds the logins and secrets the policy denies.
//!
//! [`Sandbox::ready`] runs `sh -c true` in a sandbox and tells bwrap's two
//! refusals apart by its own words ([`Refusal`]), and [`check_project`] asks
//! the project-folder question without starting a sandbox.
//!
//! crabpot's proxy lives in kelpie until the [`Sandbox`] drops, so it stops with
//! the session, and with kelpie. On Linux and Windows crabpot runs the host's
//! own binary again as its helper, so the host's `main` calls
//! [`run_from_args`] before anything else.
//!
//! macOS and Linux only for now: Windows waits on ADR 0005's open risk, which is
//! whether the sandbox user's processes stop when kelpie dies.
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::Path;
//!
//! use kelpie_engine::sandbox::{Project, Reads, Sandbox, run_from_args};
//! use kelpie_engine::session::Harness;
//!
//! // The first thing the host's `main` does
//! run_from_args();
//!
//! let harness = Harness::new("claude-agent-acp");
//! let project = Project {
//!     folder: Path::new("/work/kelpie"),
//!     network_allowlist: &[],
//!     reads: Reads::Project,
//!     sessions_root: Path::new("/state/projects"),
//! };
//! let sandbox = Sandbox::start(&harness, &project, Path::new("/state/pm"))?;
//! let _wrapped = sandbox.command(&std::process::Command::new("claude-agent-acp"))?;
//! # Ok::<(), kelpie_engine::sandbox::SandboxError>(())
//! ```

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crabpot::{Config, ConfigError, StartError, WrapError};
use serde_json::json;

use crate::session::Harness;
use crate::tools::{Door, Reach};
use crate::tree::ProcessTree;

pub mod fence;
mod hop;
mod policy;
pub mod refusal;
pub mod reviewer;
pub mod worker;

pub use hop::profile_loaded;
pub use policy::{Project, Reads, denied, policy, policy_with_door};
pub use refusal::Refusal;
pub use reviewer::Reviewer;
pub use worker::Worker;

/// Takes over the process when crabpot ran it again as its helper
///
/// A host calls this first thing in `main`, before anything of its own,
/// since on Linux and Windows the sandbox runs the host's binary again inside
/// itself, and a `main` that skipped this would run there in place of the
/// command. A run started by [`Sandbox::command`]'s profile hop execs its
/// command first. Any other run of the binary comes back at once.
pub fn run_from_args() {
    hop::run_from_args();
    crabpot::run_from_args();
}

/// Why a sandbox could not start, or could not wrap a command
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum SandboxError {
    /// The project folder is the home folder or a folder above it, by name or
    /// through a link, which holds every harness's login and every secret the
    /// sandbox denies
    ProjectAtHome {
        /// The project's folder, which holds the home folder, as it was given
        folder: PathBuf,
    },

    /// The project folder is at or inside a folder the sandbox denies its reads,
    /// a harness's login folder or a folder of secrets, by name or through a link
    ProjectInDenied {
        /// The project's folder, which sits inside the denied one, as it was given
        folder: PathBuf,

        /// The denied folder it is at or inside, as the policy really names it
        denied: PathBuf,
    },

    /// The settings crabpot was asked for were refused
    Policy(ConfigError),

    /// The session's state folder could not be made
    StateDir(io::Error),

    /// crabpot could not start: no `bwrap` on Linux, a proxy that would not
    /// listen, a path that could not be made absolute
    Start(StartError),

    /// crabpot could not put the command inside the sandbox
    Wrap(WrapError),

    /// bwrap refused to start the sandbox, for the reason the [`Refusal`] names
    Refused(Refusal),

    /// The probe command could not run in the sandbox, or failed for a reason
    /// that is no [`Refusal`]: the error is its stderr, or the spawn error
    Probe(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProjectAtHome { folder } => {
                let folder = folder.display();
                write!(
                    f,
                    "the project folder {folder} is the home folder or a folder above it, so a sandbox there would read the logins and secrets under it along with the project: move the project into a folder of its own"
                )
            }
            Self::ProjectInDenied { folder, denied } => {
                let (folder, denied) = (folder.display(), denied.display());
                write!(
                    f,
                    "the project folder {folder} is at or inside the denied folder {denied}, which holds a harness's login or a secret the sandbox denies: move the project out of {denied}"
                )
            }
            Self::Policy(error) => write!(f, "the sandbox's settings were refused: {error}"),
            Self::StateDir(error) => {
                write!(f, "the session's state folder could not be made: {error}")
            }
            Self::Start(error) => write!(f, "the sandbox could not start: {error}"),
            Self::Wrap(error) => write!(f, "the command could not go in the sandbox: {error}"),
            Self::Refused(refusal) => write!(f, "the sandbox cannot start: {refusal}"),
            Self::Probe(detail) => write!(f, "the sandbox's probe command failed: {detail}"),
        }
    }
}

impl core::error::Error for SandboxError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::ProjectAtHome { .. } | Self::ProjectInDenied { .. } => None,
            Self::Policy(error) => Some(error),
            Self::StateDir(error) => Some(error),
            Self::Start(error) => Some(error),
            Self::Wrap(error) => Some(error),
            Self::Refused(refusal) => Some(refusal),
            Self::Probe(_) => None,
        }
    }
}

impl From<SandboxError> for io::Error {
    fn from(error: SandboxError) -> Self {
        Self::other(error)
    }
}

/// One session's sandbox, held until the session ends
///
/// Dropping it revokes what crabpot granted and stops its proxy. Hold it as
/// long as any command wrapped by it may run.
#[derive(Debug)]
pub struct Sandbox {
    inner: Arc<crabpot::Sandbox>,
    /// The network lists the sandbox runs under now
    config: Arc<Mutex<Config>>,
}

impl Sandbox {
    /// Start the sandbox of a session on `harness`, in `project`, keeping its
    /// files in `state_dir`
    ///
    /// Makes `state_dir` when it is missing, since the sandbox may write there
    /// and nowhere else. See [`policy`] for what the session may reach, and
    /// [`Reads`] for how far it reads.
    ///
    /// A project whose folder is the home folder or a folder above it, or is at
    /// or inside a login or a secret folder, is refused before anything is made
    /// or started, whether it reaches it by name or through a link: such a folder
    /// holds the logins and secrets the sandbox denies, so a sandbox over it
    /// would read them with the project.
    ///
    /// # Errors
    ///
    /// [`SandboxError`]: the project folder holds the home folder or sits inside
    /// a denied one, the settings are refused, `state_dir` cannot be made, or
    /// crabpot cannot start.
    pub fn start(
        harness: &Harness,
        project: &Project<'_>,
        state_dir: &Path,
    ) -> Result<Self, SandboxError> {
        Self::start_with_door(harness, project, state_dir, None)
    }

    /// [`Sandbox::start`] for a session that reaches kelpie's tools through
    /// `door`, the one extra route out the policy names
    ///
    /// # Errors
    ///
    /// As [`Sandbox::start`], and `door` is one this platform cannot grant.
    pub fn start_with_door(
        harness: &Harness,
        project: &Project<'_>,
        state_dir: &Path,
        door: Option<&Door>,
    ) -> Result<Self, SandboxError> {
        check_project(project)?;
        let config =
            policy_with_door(harness, project, state_dir, door).map_err(SandboxError::Policy)?;
        fs::create_dir_all(state_dir).map_err(SandboxError::StateDir)?;
        Self::from_config(config)
    }

    /// Start crabpot on `config`, and keep its network lists for [`Sandbox::network`]
    fn from_config(config: Config) -> Result<Self, SandboxError> {
        let inner = crabpot::Sandbox::start(&config).map_err(SandboxError::Start)?;
        Ok(Self {
            inner: Arc::new(inner),
            config: Arc::new(Mutex::new(config)),
        })
    }

    /// A handle on this sandbox's network lists, to widen them while it runs
    ///
    /// The handle does not keep the sandbox alive: once the sandbox drops, it
    /// lets nothing through.
    #[must_use]
    pub fn network(&self) -> Network {
        Network {
            inner: Arc::downgrade(&self.inner),
            config: Arc::clone(&self.config),
        }
    }

    /// Start the sandbox of a worker on `harness`, in `project`
    ///
    /// Makes the worker's state folder when it is missing, since the sandbox may
    /// write there, in its worktree and in its clone's git dir, and nowhere else.
    /// See [`worker::policy`] for what the worker may reach. The project folder is
    /// refused as it is at [`Sandbox::start`], before the settings are built or
    /// the state folder made.
    ///
    /// # Errors
    ///
    /// [`SandboxError`]: the project folder holds the home folder or sits inside
    /// a denied one, the settings are refused, the state folder cannot be made,
    /// or crabpot cannot start.
    pub fn start_worker(
        harness: &Harness,
        project: &Project<'_>,
        worker: &Worker<'_>,
    ) -> Result<Self, SandboxError> {
        check_project(project)?;
        let config = worker::policy(harness, project, worker).map_err(SandboxError::Policy)?;
        fs::create_dir_all(worker.state_dir).map_err(SandboxError::StateDir)?;
        Self::from_config(config)
    }

    /// Start the sandbox `config` describes, a policy already composed
    ///
    /// For a caller that holds a session's policy and runs one more command under it, as
    /// the review pass runs a reviewer's test under the worker's own ([`worker::policy`]).
    /// The folders `config` lets be written must exist. No check of the project's folder
    /// is made here: the caller made one when it composed the policy.
    ///
    /// # Errors
    ///
    /// [`SandboxError::Start`] when crabpot cannot start.
    pub fn start_policy(config: Config) -> Result<Self, SandboxError> {
        Self::from_config(config)
    }

    /// Start the sandbox of a reviewer on `harness`, in `project`
    ///
    /// Makes the reviewer's state folder when it is missing, since the sandbox may
    /// write there and in its tests folder, and nowhere else. The tests folder is
    /// the caller's to make. See [`reviewer::policy`] for what the reviewer may
    /// reach. The project folder is refused as it is at [`Sandbox::start`], before
    /// the settings are built or the state folder made.
    ///
    /// # Errors
    ///
    /// [`SandboxError`]: the project folder holds the home folder or sits inside
    /// a denied one, the settings are refused, the state folder cannot be made,
    /// or crabpot cannot start.
    pub fn start_reviewer(
        harness: &Harness,
        project: &Project<'_>,
        reviewer: &Reviewer<'_>,
    ) -> Result<Self, SandboxError> {
        check_project(project)?;
        let config = reviewer::policy(harness, project, reviewer).map_err(SandboxError::Policy)?;
        fs::create_dir_all(reviewer.state_dir).map_err(SandboxError::StateDir)?;
        Self::from_config(config)
    }

    /// `command`, wrapped to run inside this sandbox
    ///
    /// Keeps its program, arguments, working directory and environment changes,
    /// and none of its standard streams: set those on the wrapped command.
    /// Spawn the result through the process tree (ADR 0001).
    ///
    /// On Linux, a kelpie that runs `unconfined` while the `kelpie` AppArmor
    /// profile is loaded, as after Install, wraps the command once more in a
    /// fresh run of its own binary (`--sandbox-hop`), which the profile
    /// confines, so bwrap starts without a relaunch. Any other kelpie wraps it
    /// as crabpot does.
    ///
    /// # Errors
    ///
    /// [`SandboxError::Wrap`] when the command cannot be put in the sandbox.
    pub fn command(&self, command: &Command) -> Result<Command, SandboxError> {
        let wrapped = self.inner.command(command).map_err(SandboxError::Wrap)?;
        Ok(hop::through_profile(&wrapped).unwrap_or(wrapped))
    }

    /// Run `sh -c true` in this sandbox, to learn whether it can start a command
    ///
    /// Runs through `tree` (ADR 0001), so a stop of the tree ends it.
    ///
    /// # Errors
    ///
    /// [`SandboxError::Refused`] when bwrap names one of its [`Refusal`]s, and
    /// [`SandboxError::Probe`] when the command cannot be wrapped or spawned, or
    /// fails for any other reason.
    pub fn ready(&self, tree: &ProcessTree) -> Result<(), SandboxError> {
        let mut probe = Command::new("sh");
        probe.args(["-c", "true"]);
        let mut wrapped = self.command(&probe)?;
        let output = tree
            .output(&mut wrapped)
            .map_err(|error| SandboxError::Probe(error.to_string()))?;
        refusal::judge(&output)
    }
}

/// Refuse a project whose own folder would bring a denied folder into the
/// sandbox's reads, whichever policy the session runs under
///
/// The home folder or a folder above it holds every login and secret, and a
/// folder at or inside one of them is that folder to its own sandbox, since the
/// folder beside it is opened back for the project.
///
/// [`Sandbox::start`], [`Sandbox::start_worker`] and [`Sandbox::start_reviewer`] call it first, so a caller
/// can ask the same question without starting a sandbox.
///
/// # Errors
///
/// [`SandboxError::ProjectAtHome`] or [`SandboxError::ProjectInDenied`].
pub fn check_project(project: &Project<'_>) -> Result<(), SandboxError> {
    if let Some(folder) = policy::holds_home(project) {
        return Err(SandboxError::ProjectAtHome { folder });
    }
    if let Some((folder, denied)) = policy::in_denied(project) {
        return Err(SandboxError::ProjectInDenied { folder, denied });
    }
    Ok(())
}

/// The hosts a running sandbox lets through, which can grow while it runs
///
/// What `request_network` widens when the maintainer allows a host (ADR 0005).
/// Made by [`Sandbox::network`].
#[derive(Debug, Clone)]
pub struct Network {
    inner: Weak<crabpot::Sandbox>,
    config: Arc<Mutex<Config>>,
}

/// The port a request is judged on: a host the agent asks for is one it dials over TLS
const HTTPS: u16 = 443;

impl Reach for Network {
    fn allows(&self, host: &str) -> bool {
        let config = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        config
            .network
            .allowed_domains
            .iter()
            .any(|entry| entry.matches(host, HTTPS))
    }

    /// Adds `host` to the allowed domains, which crabpot's proxy follows from its next request on
    ///
    /// Says `false`, and changes nothing, when the sandbox has dropped or crabpot cannot list `host`
    fn allow(&self, host: &str) -> bool {
        let Some(sandbox) = self.inner.upgrade() else {
            return false;
        };
        let mut config = self.config.lock().unwrap_or_else(PoisonError::into_inner);
        let mut allowed: Vec<&str> = config
            .network
            .allowed_domains
            .iter()
            .map(|entry| entry.as_str())
            .collect();
        allowed.push(host);
        let lists = json!({
            "network": {"allowedDomains": allowed, "deniedDomains": []},
            "filesystem": {"denyRead": [], "allowWrite": [], "denyWrite": []},
        });
        let Ok(widened) = Config::from_json(&lists.to_string()) else {
            return false;
        };
        sandbox.replace_network_lists(&widened);
        config.network.allowed_domains = widened.network.allowed_domains;
        true
    }
}

// The escapes SK tried against a sandbox, tried here against the PM's policy,
// are `tests/sandbox/escapes.rs`, not a module of this one: on Linux crabpot
// runs the test binary again as its helper, which this library's `libtest`
// `main` would refuse.
#[cfg(test)]
mod tests;
