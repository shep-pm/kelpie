//! The port a read runs through, and why a read fails

use core::fmt;
use std::path::Path;

use kelpie_protocol::SessionEvent;

use super::Finding;

/// One read: what the reviewer is told, where it reads, and where its session's
/// events go
#[derive(Clone, Copy)]
pub struct Read<'a> {
    /// What the reviewer is asked
    pub prompt: &'a str,
    /// The worktree it reads, which it may not write
    pub worktree: &'a Path,
    /// The one folder it may write, for a failing test of its own, when the
    /// project has a test command
    pub tests: Option<&'a Path>,
    /// Takes each event of the reviewer's session, so the app can show the read
    ///
    /// It runs on the session's own thread and must not block for long.
    pub sink: &'a (dyn Fn(SessionEvent) + Sync),
}

impl fmt::Debug for Read<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Read")
            .field("prompt", &self.prompt)
            .field("worktree", &self.worktree)
            .field("tests", &self.tests)
            .finish_non_exhaustive()
    }
}

/// Runs one read of a worktree's diff
pub trait Reviewer: Send + Sync {
    /// Reads as `read` says, and answers with the findings, or none when the
    /// reviewer found nothing
    ///
    /// # Errors
    ///
    /// [`ReviewerError`] when the read could not be started or did not finish.
    fn read(&self, read: &Read<'_>) -> Result<Vec<Finding>, ReviewerError>;
}

/// Why a read did not produce findings
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReviewerError {
    /// The reviewer could not be started, with the OS's reason
    Start(String),
    /// The reviewer started and ended unsuccessfully, with what it said
    Failed(String),
    /// The read was ended because kelpie is stopping
    Stopped,
    /// The reviewer answered with something that is not a review, with its reply
    Unreadable(String),
    /// The model stopped at its token limit before it finished, or inside its
    /// thinking, with the reason
    CutOff(String),
}

impl fmt::Display for ReviewerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(error) => write!(f, "cannot start the read: {error}"),
            Self::Failed(said) | Self::CutOff(said) => {
                write!(f, "the read failed: {}", said.trim())
            }
            Self::Stopped => f.write_str("the read was stopped with kelpie"),
            Self::Unreadable(reply) => write!(f, "the read's reply is unreadable: {reply}"),
        }
    }
}

impl core::error::Error for ReviewerError {}
