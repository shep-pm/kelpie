//! Harnesses: what each one needs before a session can run on it
//!
//! A [`crate::session::Harness`] says what to run, and each harness's slice builds one
//! from what that harness asks for (ADR 0007): its bridge from the ACP registry,
//! the folder the session keeps its state in, and the person's own login.
//! Nothing here starts a process: [`session`](crate::session) does that through
//! the process tree (ADR 0001).
//!
//! Kelpie ships no harness and no login of its own (ADR 0006). Each slice points
//! the harness at the person's own install and gives the session a config folder
//! of kelpie's own, which keeps the person's global folder out until they turn
//! it on. A harness that speaks ACP itself, such as goose, is run directly; one
//! that does not is run through its bridge from the registry.
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::Path;
//!
//! use kelpie_engine::harness::{Login, claude, goose};
//!
//! let login = Login::from_env(claude::LOGIN_ENV);
//! let harness = claude::harness("claude-agent-acp".into(), Path::new("pm"), login)?;
//! println!("run {}", harness.program.display());
//!
//! let ollama = goose::Provider {
//!     name: "ollama".to_owned(),
//!     host: None,
//! };
//! let local = goose::harness(Path::new("pm"), ollama)?;
//! let acp = local.args.first().expect("goose is run with its subcommand");
//! println!("or run {} {}", local.program.display(), acp.to_string_lossy());
//! # Ok::<(), kelpie_engine::harness::HarnessError>(())
//! ```

use std::env::{self, consts::EXE_SUFFIX};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub mod claude;
pub mod goose;

/// The login a harness session runs with
///
/// Kelpie holds no login of its own (ADR 0007). Until the settings page holds
/// one, it is a token in kelpie's own environment, made once with the harness's
/// own command.
pub struct Login {
    token: Option<String>,
}

impl Login {
    /// No login: a session built from this says how to make one
    #[must_use]
    pub const fn none() -> Self {
        Self { token: None }
    }

    /// `token` as a login, `none` when it is blank
    #[must_use]
    pub fn from_token(token: impl Into<String>) -> Self {
        let token = token.into();
        Self {
            token: (!token.trim().is_empty()).then(|| token.trim().to_owned()),
        }
    }

    /// The token kelpie's own environment holds under `name`, `none` when it holds none
    #[must_use]
    pub fn from_env(name: &str) -> Self {
        match std::env::var(name) {
            Ok(token) => Self::from_token(token),
            Err(_) => Self::none(),
        }
    }

    /// The token, `None` when there is no login
    #[must_use]
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }
}

/// A login is a secret, so `Debug` leaves its value out
impl fmt::Debug for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Login")
            .field("token", &self.token.is_some())
            .finish()
    }
}

/// Why a harness could not be built
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HarnessError {
    /// The harness's own command is not on `PATH`, so the person has not installed it
    NotInstalled {
        /// The harness kelpie cannot run, as it names it to the person
        harness: String,

        /// The command kelpie looked for on `PATH`
        cmd: String,
    },

    /// The login this harness needs is not in kelpie's environment
    NoLogin {
        /// The harness that has no login
        harness: String,

        /// The variable the token belongs in
        env: String,

        /// The command that makes a token
        setup: String,
    },
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled { harness, cmd } => write!(
                f,
                "{harness} is not installed: kelpie found no {cmd} on PATH. \
                 Install {harness} and start kelpie again."
            ),
            Self::NoLogin {
                harness,
                env,
                setup,
            } => write!(
                f,
                "{env} is not set in kelpie's environment, so {harness} has no login: \
                 each session keeps a config folder of its own, which holds no login. \
                 Run `{setup}` and put the token it prints in kelpie's environment as {env}."
            ),
        }
    }
}

impl core::error::Error for HarnessError {}

/// The first `cmd` on the `PATH` text `path`, as an absolute path
///
/// Each harness runs the person's own install, found here rather than trusted to
/// whatever `PATH` the harness would search itself.
pub(super) fn on_path(cmd: &str, path: &OsStr) -> Option<PathBuf> {
    let name = format!("{cmd}{EXE_SUFFIX}");
    env::split_paths(path).find_map(|dir| {
        let found = dir.join(&name);
        is_program(&found).then(|| std::path::absolute(&found).unwrap_or(found))
    })
}

/// Whether `path` is a file this OS can run
fn is_program(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && runnable(&metadata)
}

/// Whether the OS says a file may be run
#[cfg(unix)]
fn runnable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o111 != 0
}

/// Windows runs a file by its extension, which `name` already carries
#[cfg(not(unix))]
fn runnable(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests;
