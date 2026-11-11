//! A ticket as its source holds it, an entry of a list of them, and an open pull request

use std::num::NonZeroU64;

use super::TicketId;
use crate::github::Timestamp;

/// A ticket as its source holds it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Its title
    pub title: String,
    /// Its body, as written
    pub body: String,
    /// Its labels' names
    pub labels: Vec<String>,
    /// Whether it is still open
    pub open: bool,
}

/// One entry of a list of tickets
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// Its id
    pub id: TicketId,
    /// Its title
    pub title: String,
    /// Its labels' names
    pub labels: Vec<String>,
    /// The login of whoever opened it, empty when their account is gone
    pub author: String,
    /// When it was opened
    pub created: Timestamp,
}

/// An open pull request, and the tickets it closes
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPull {
    /// Its number
    pub number: NonZeroU64,
    /// The branch it is opened from
    pub branch: String,
    /// Whether it is still a draft
    pub draft: bool,
    /// The tickets of this repo it closes when it merges, in the order GitHub lists them
    pub closes: Vec<TicketId>,
}
