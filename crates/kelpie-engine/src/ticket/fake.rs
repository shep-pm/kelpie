//! A stand-in ticket source, for code that reads the port
//!
//! [`FakeTickets`] answers each method from what a test scripted and records
//! what it was asked, so a reader of [`Tickets`] needs no source behind it.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::{ForgeError, GithubTickets, Issue, Listed, TicketId, Tickets};
use crate::github::Timestamp;

/// What the stand-in was asked, in order
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// [`Tickets::issue`], and the ticket
    Issue(TicketId),
    /// [`Tickets::ready_issues`], and the label
    Ready(String),
    /// [`GithubTickets::open_issues`]
    Open,
    /// [`GithubTickets::closed_issues`], and its `since`
    Closed(Timestamp),
}

type Listing = Result<Vec<Listed>, ForgeError>;

#[derive(Debug, Default)]
struct Script {
    issues: BTreeMap<TicketId, Result<Issue, ForgeError>>,
    ready: Option<Listing>,
    open: Option<Listing>,
    closed: Option<Listing>,
    asked: Vec<Asked>,
}

/// A ticket source that answers from a script
///
/// A ticket with no scripted answer is one the source does not hold, and a list
/// with none is empty.
#[derive(Debug, Default)]
pub struct FakeTickets {
    script: Mutex<Script>,
}

impl FakeTickets {
    /// A source with nothing scripted
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What [`Tickets::issue`] answers for `id`
    pub fn script_issue(&self, id: TicketId, answer: Result<Issue, ForgeError>) {
        self.script().issues.insert(id, answer);
    }

    /// What [`Tickets::ready_issues`] answers, whatever label it is asked for
    pub fn script_ready(&self, answer: Listing) {
        self.script().ready = Some(answer);
    }

    /// What [`GithubTickets::open_issues`] answers
    pub fn script_open(&self, answer: Listing) {
        self.script().open = Some(answer);
    }

    /// What [`GithubTickets::closed_issues`] answers, whatever `since` it is asked for
    pub fn script_closed(&self, answer: Listing) {
        self.script().closed = Some(answer);
    }

    /// What it was asked, in order
    #[must_use]
    pub fn asked(&self) -> Vec<Asked> {
        self.script().asked.clone()
    }

    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.script
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Tickets for FakeTickets {
    fn issue(&self, id: &TicketId) -> Result<Issue, ForgeError> {
        let mut script = self.script();
        script.asked.push(Asked::Issue(*id));
        script
            .issues
            .get(id)
            .cloned()
            .unwrap_or(Err(ForgeError::NoIssue { ticket: *id }))
    }

    fn ready_issues(&self, label: &str) -> Result<Vec<Listed>, ForgeError> {
        let mut script = self.script();
        script.asked.push(Asked::Ready(label.to_owned()));
        script.ready.clone().unwrap_or_else(|| Ok(Vec::new()))
    }
}

impl GithubTickets for FakeTickets {
    fn open_issues(&self) -> Result<Vec<Listed>, ForgeError> {
        let mut script = self.script();
        script.asked.push(Asked::Open);
        script.open.clone().unwrap_or_else(|| Ok(Vec::new()))
    }

    fn closed_issues(&self, since: Timestamp) -> Result<Vec<Listed>, ForgeError> {
        let mut script = self.script();
        script.asked.push(Asked::Closed(since));
        script.closed.clone().unwrap_or_else(|| Ok(Vec::new()))
    }
}
