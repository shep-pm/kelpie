//! Harnesses: what each one needs before a session can run on it
//!
//! A [`crate::session::Harness`] says what to run, and each harness's slice builds one
//! from what that harness asks for (ADR 0007): its bridge from the ACP registry,
//! the folder the session keeps its state in, and the person's own login, read
//! from the [`login`](crate::login) store.
//! Nothing here starts a process: [`session`](crate::session) does that through
//! the process tree (ADR 0001).
//!
//! Kelpie ships no harness and no login of its own (ADR 0006). Each slice points
//! the harness at the person's own install and gives the session a config folder
//! of kelpie's own, which keeps the person's global folder out until they turn
//! it on.
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::Path;
//!
//! use kelpie_engine::harness::claude;
//! use kelpie_engine::login::Store;
//!
//! let harness = claude::harness("claude-agent-acp".into(), Path::new("pm"), &Store::keychain())?;
//! println!("run {}", harness.program.display());
//! # Ok::<(), kelpie_engine::harness::HarnessError>(())
//! ```

use std::fmt;

use crate::login::LoginError;

pub mod claude;

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

    /// The login store holds no login for this harness
    NoLogin {
        /// The harness that has no login
        harness: String,

        /// The command that makes a token
        setup: String,
    },

    /// The login store could not be read, so whether the harness has a login is unknown
    Login(LoginError),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled { harness, cmd } => write!(
                f,
                "{harness} is not installed: kelpie found no {cmd} on PATH. \
                 Install {harness} and start kelpie again."
            ),
            Self::NoLogin { harness, setup } => write!(
                f,
                "{harness} has no login: each session keeps a config folder of its own, \
                 which holds none. Run `{setup}`, then paste the token it prints into \
                 {harness}'s login on kelpie's settings page."
            ),
            Self::Login(error) => write!(f, "kelpie could not read the login: {error}"),
        }
    }
}

impl From<LoginError> for HarnessError {
    fn from(error: LoginError) -> Self {
        Self::Login(error)
    }
}

impl core::error::Error for HarnessError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Login(error) => Some(error),
            Self::NotInstalled { .. } | Self::NoLogin { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests;
