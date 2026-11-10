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

use crabpot::{ConfigError, StartError, WrapError};

use crate::session::Harness;
use crate::tools::Door;

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
    inner: crabpot::Sandbox,
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
        Ok(Self { inner })
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

// The escapes SK tried against a sandbox, tried here against the PM's policy,
// are `tests/sandbox/escapes.rs`, not a module of this one: on Linux crabpot
// runs the test binary again as its helper, which this library's `libtest`
// `main` would refuse.
#[cfg(test)]
mod tests;
