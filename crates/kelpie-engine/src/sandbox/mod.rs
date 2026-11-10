//! A sandbox for each session: crabpot around the harness (ADR 0005)
//!
//! Every agent session kelpie starts runs inside crabpot (shep-pm/crabpot), the
//! PM and its workers alike. [`Sandbox::start`] builds the session's
//! [`policy`] and starts crabpot's sandbox from it, and [`Sandbox::command`]
//! wraps the harness's command into one that runs inside it. The process tree
//! spawns the wrapped command as it spawns anything else, so the run's mark
//! reaches it and the sweep finds it (ADR 0001).
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
//! use kelpie_engine::sandbox::{Project, Sandbox, run_from_args};
//! use kelpie_engine::session::Harness;
//!
//! // The first thing the host's `main` does
//! run_from_args();
//!
//! let harness = Harness::new("claude-agent-acp");
//! let project = Project {
//!     folder: Path::new("/work/kelpie"),
//!     network_allowlist: &[],
//!     read_anywhere: false,
//! };
//! let sandbox = Sandbox::start(&harness, &project, Path::new("/state/pm"))?;
//! let _wrapped = sandbox.command(&std::process::Command::new("claude-agent-acp"))?;
//! # Ok::<(), kelpie_engine::sandbox::SandboxError>(())
//! ```

use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use crabpot::{Config, ConfigError, StartError, WrapError};
use serde_json::json;

use crate::session::Harness;
use crate::tools::{Door, Reach};

pub mod fence;
mod policy;

pub use policy::{Project, denied, policy, policy_with_door};

/// Takes over the process when crabpot ran it again as its helper
///
/// A host calls this first thing in `main`, before anything of its own,
/// since on Linux and Windows the sandbox runs the host's binary again inside
/// itself, and a `main` that skipped this would run there in place of the
/// command. Any other run of the binary comes back at once.
pub fn run_from_args() {
    crabpot::run_from_args();
}

/// Why a sandbox could not start, or could not wrap a command
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum SandboxError {
    /// The settings crabpot was asked for were refused
    Policy(ConfigError),

    /// The session's state folder could not be made
    StateDir(io::Error),

    /// crabpot could not start: no `bwrap` on Linux, a proxy that would not
    /// listen, a path that could not be made absolute
    Start(StartError),

    /// crabpot could not put the command inside the sandbox
    Wrap(WrapError),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Policy(error) => write!(f, "the sandbox's settings were refused: {error}"),
            Self::StateDir(error) => {
                write!(f, "the session's state folder could not be made: {error}")
            }
            Self::Start(error) => write!(f, "the sandbox could not start: {error}"),
            Self::Wrap(error) => write!(f, "the command could not go in the sandbox: {error}"),
        }
    }
}

impl core::error::Error for SandboxError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Policy(error) => Some(error),
            Self::StateDir(error) => Some(error),
            Self::Start(error) => Some(error),
            Self::Wrap(error) => Some(error),
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
    /// and nowhere else. See [`policy`] for what the session may reach.
    ///
    /// # Errors
    ///
    /// [`SandboxError`]: the settings are refused, `state_dir` cannot be made,
    /// or crabpot cannot start.
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
        let config =
            policy_with_door(harness, project, state_dir, door).map_err(SandboxError::Policy)?;
        fs::create_dir_all(state_dir).map_err(SandboxError::StateDir)?;
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

    /// `command`, wrapped to run inside this sandbox
    ///
    /// Keeps its program, arguments, working directory and environment changes,
    /// and none of its standard streams: set those on the wrapped command.
    /// Spawn the result through the process tree (ADR 0001).
    ///
    /// # Errors
    ///
    /// [`SandboxError::Wrap`] when the command cannot be put in the sandbox.
    pub fn command(&self, command: &Command) -> Result<Command, SandboxError> {
        self.inner.command(command).map_err(SandboxError::Wrap)
    }
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
