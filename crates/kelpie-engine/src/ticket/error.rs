//! Why a read of a project's tickets failed

use core::fmt;

use super::TicketId;
use crate::github::{Cooldown, Timestamp};

/// Why a read of a project's tickets failed
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ForgeError {
    /// The source's command line tool could not be started, with the OS's reason
    Spawn(String),
    /// The tool ran and exited unsuccessfully, with what it said on stderr
    Failed(String),
    /// The tool succeeded but its output was not what was asked for
    Unreadable(String),
    /// The forge said its rate limit was used up, so no call is made until it resets then
    Held(Timestamp),
    /// The forge answered that a rate limit is used up, saying so in `said` and
    /// when to ask again in `cooldown`
    Limited {
        /// What to tell the log
        said: String,
        /// When the answer says to ask again
        cooldown: Cooldown,
    },
    /// The ticket is not one this source holds, such as a local ticket asked
    /// of a project whose tickets are GitHub issues
    NoIssue {
        /// The ticket it was asked for
        ticket: TicketId,
    },
}

impl ForgeError {
    /// Whether asking again later may succeed with nothing changed
    ///
    /// A failed call passes when the tool reported the forge's own answer of 5xx
    /// or worse, which it prints as `HTTP 502: Bad Gateway (https://…)` or at the
    /// end of its own line, `(HTTP 502)`.
    #[must_use]
    pub fn passes(&self) -> bool {
        match self {
            Self::Failed(said) => printed_status(said).is_some_and(|status| status >= 500),
            Self::Limited { .. } | Self::Held(_) => true,
            Self::Spawn(_) | Self::Unreadable(_) | Self::NoIssue { .. } => false,
        }
    }

    /// Whether this is the forge's own answer that what was asked for is not there
    ///
    /// That is a 404 the tool printed. A [`ForgeError::NoIssue`] is not one: the
    /// source was never asked.
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Failed(said) if printed_status(said) == Some(404))
    }
}

// The status the tool printed for a failed call's answer, of which the last is
// the one its own line names
fn printed_status(said: &str) -> Option<u16> {
    said.match_indices("HTTP ")
        .filter_map(|(at, mark)| {
            let first = at + mark.len();
            let code = said.get(first..first + 3)?;
            code.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| code.parse().ok())?
        })
        .last()
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(f, "cannot start the ticket source's tool: {error}"),
            Self::Failed(stderr) => write!(f, "the ticket source failed: {}", stderr.trim()),
            Self::Unreadable(output) => {
                write!(
                    f,
                    "unreadable answer from the ticket source: {}",
                    output.trim()
                )
            }
            Self::Held(until) => write!(
                f,
                "not asked: the forge's rate limit is used up until {} seconds after the Unix epoch",
                until.secs()
            ),
            Self::Limited { said, .. } => f.write_str(said),
            Self::NoIssue { ticket } => write!(f, "ticket {ticket} is not held by this source"),
        }
    }
}

impl core::error::Error for ForgeError {}
