//! The port a project's tickets are read through

use super::{ForgeError, Issue, Listed, OpenPull, TicketId};
use crate::github::Timestamp;

/// The tickets of one project, as their source holds them
///
/// The source holds its repo, so no method names one. A ticket's id is opaque
/// to the caller: a GitHub source turns a [`TicketId::Github`] back into the
/// number its call needs, and answers [`ForgeError::NoIssue`] for a local one.
pub trait Tickets: Send + Sync {
    /// The ticket `id`
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the source cannot be asked, has no such ticket, or
    /// its answer cannot be read.
    fn issue(&self, id: &TicketId) -> Result<Issue, ForgeError>;

    /// The open tickets carrying `label`
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the source cannot be asked or its answer read.
    fn ready_issues(&self, label: &str) -> Result<Vec<Listed>, ForgeError>;
}

/// The reads only a GitHub source answers
pub trait GithubTickets: Tickets {
    /// Every open issue, ready or not
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when GitHub cannot be asked or its answer read.
    fn open_issues(&self) -> Result<Vec<Listed>, ForgeError>;

    /// The issues closed at or after `since`
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when GitHub cannot be asked or its answer read.
    fn closed_issues(&self, since: Timestamp) -> Result<Vec<Listed>, ForgeError>;

    /// Every open pull request, with the tickets of this repo each closes
    ///
    /// A ticket is in Review while a pull request closing it is open.
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when GitHub cannot be asked or its answer read.
    fn open_pulls(&self) -> Result<Vec<OpenPull>, ForgeError>;
}
