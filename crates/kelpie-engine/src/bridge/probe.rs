//! The probe a new bridge version passes before any session uses it
//!
//! Bridges are 0.x and release often, so a version number says nothing about
//! compatibility. [`probe`] starts the bridge through the process tree, sends
//! `initialize` and no prompt, and stops it. It passes when the bridge speaks
//! protocol 1 and keeps every [`Capability`] kelpie uses for its harness
//! (`docs/design/bridge-updates.md`, The probe).
//!
//! The start is the one [`crate::session::options::ask`] uses to list a
//! harness's models, less its `session/new`: no session opens, so no login is
//! needed and nothing is spent.

use std::fmt;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::InitializeResponse;

use crate::session::options::{initialized_within, launcher};
use crate::session::{Harness, Launch, SessionError};
use crate::tree::ProcessTree;

/// How long a bridge is given to answer `initialize` before the probe fails
const ANSWER_WITHIN: Duration = Duration::from_secs(30);

/// Something a bridge's answer to `initialize` says it can do, which a harness's adapter uses
// kelpie-engine is a library: each harness slice adds the capabilities its adapter uses
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Capability {
    /// `session/resume`, or `session/load` where the bridge has no resume
    ResumeSession,
}

impl Capability {
    /// Whether the bridge's answer to `initialize` offers it
    fn offered_by(self, init: &InitializeResponse) -> bool {
        match self {
            Self::ResumeSession => {
                let capabilities = &init.agent_capabilities;
                capabilities.session_capabilities.resume.is_some() || capabilities.load_session
            }
        }
    }

    /// What the bridge was expected to offer, as the protocol names it
    fn name(self) -> &'static str {
        match self {
            Self::ResumeSession => "session/resume or session/load",
        }
    }
}

/// Why a bridge did not pass the probe
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum ProbeError {
    /// The bridge process could not be started
    Start(io::Error),

    /// The bridge ended the connection before it answered, with how it ended
    Ended {
        /// The bridge's own end, as the tree and its stderr can say it
        message: String,
    },

    /// The bridge did not answer within the time kelpie waits, and was stopped
    TimedOut {
        /// How long kelpie waited
        after: Duration,
        /// How the bridge ended once it was stopped, with what it last wrote
        message: String,
    },

    /// The bridge answered `initialize` with an error
    Refused {
        /// The error the bridge answered with
        message: String,
    },

    /// The bridge answered with a protocol version other than 1
    WrongProtocol {
        /// The version it answered with
        answered: String,
    },

    /// The bridge answered without a capability kelpie uses for its harness
    Missing {
        /// The capability the answer lacks
        capability: Capability,
    },
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(error) => write!(f, "kelpie could not start the bridge: {error}"),
            Self::Ended { message } => {
                write!(
                    f,
                    "the bridge ended the connection before it answered: {message}"
                )
            }
            Self::TimedOut { after, message } => {
                write!(
                    f,
                    "the bridge did not answer within {} s and was stopped",
                    after.as_secs()
                )?;
                if message.is_empty() {
                    Ok(())
                } else {
                    write!(f, ": {message}")
                }
            }
            Self::Refused { message } => write!(f, "the bridge refused to initialize: {message}"),
            Self::WrongProtocol { answered } => {
                write!(
                    f,
                    "the bridge speaks protocol {answered}, and kelpie speaks 1"
                )
            }
            Self::Missing { capability } => {
                write!(f, "the bridge no longer offers {}", capability.name())
            }
        }
    }
}

impl core::error::Error for ProbeError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Start(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SessionError> for ProbeError {
    fn from(error: SessionError) -> Self {
        match error {
            SessionError::Start(error) => Self::Start(error),
            SessionError::Ended { message } => Self::Ended { message },
            SessionError::TimedOut { after, message } => Self::TimedOut { after, message },
            other => Self::Refused {
                message: other.to_string(),
            },
        }
    }
}

/// Start `harness` through `tree`, send `initialize` and no prompt, and stop it
///
/// Passes when the answer is protocol 1 and offers every capability in
/// `requires`, and fails when it does not, or when no answer comes within 30 s.
/// This blocks until the bridge has answered or is gone, and it is stopped
/// either way.
///
/// # Errors
///
/// [`ProbeError`] says whether the bridge could not be started, ended the
/// connection, stayed silent, refused, spoke another protocol, or lacks a
/// capability in `requires`.
pub fn probe(
    tree: &Arc<ProcessTree>,
    harness: &Harness,
    requires: &[Capability],
) -> Result<(), ProbeError> {
    let folder = harness.program.parent().unwrap_or(Path::new("."));
    probed_within(
        &mut launcher(tree, harness, folder),
        requires,
        ANSWER_WITHIN,
    )
}

/// [`probe`], on a bridge `launcher` starts instead of one from the process tree, with `deadline` to answer
pub(crate) fn probed_within(
    launcher: &mut dyn Launch,
    requires: &[Capability],
    deadline: Duration,
) -> Result<(), ProbeError> {
    let init = initialized_within(launcher, deadline)?;
    if init.protocol_version != ProtocolVersion::V1 {
        return Err(ProbeError::WrongProtocol {
            answered: init.protocol_version.to_string(),
        });
    }
    match requires
        .iter()
        .find(|capability| !capability.offered_by(&init))
    {
        Some(capability) => Err(ProbeError::Missing {
            capability: *capability,
        }),
        None => Ok(()),
    }
}
