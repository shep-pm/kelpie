//! A stand-in ticket source, for code that reads the port
//!
//! [`FakeTickets`] answers each method from what a test scripted and records
//! what it was asked, so a reader of [`Tickets`] needs no source behind it.

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Mutex;

use super::{
    ForgeError, GithubTickets, Issue, Listed, OpenPull, PullState, Pulls, TicketId, Tickets,
};
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
    /// [`GithubTickets::open_pulls`]
    OpenPulls,
}

type Listing = Result<Vec<Listed>, ForgeError>;
type PullListing = Result<Vec<OpenPull>, ForgeError>;

#[derive(Debug, Default)]
struct Script {
    issues: BTreeMap<TicketId, Result<Issue, ForgeError>>,
    ready: Option<Listing>,
    open: Option<Listing>,
    closed: Option<Listing>,
    pulls: Option<PullListing>,
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

    /// What [`GithubTickets::open_pulls`] answers
    pub fn script_pulls(&self, answer: PullListing) {
        self.script().pulls = Some(answer);
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

    fn open_pulls(&self) -> Result<Vec<OpenPull>, ForgeError> {
        let mut script = self.script();
        script.asked.push(Asked::OpenPulls);
        script.pulls.clone().unwrap_or_else(|| Ok(Vec::new()))
    }
}

/// What [`FakePulls`] was asked, in order
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PullCall {
    /// [`Pulls::create_draft`], with its `head`, `base`, `title` and `body`
    Create {
        /// The branch to open from
        head: String,
        /// The branch to open into
        base: String,
        /// The pull request's title
        title: String,
        /// The pull request's body
        body: String,
    },
    /// [`Pulls::branch_head`], and the branch
    BranchHead(String),
    /// [`Pulls::edit`], with the number, title and body
    Edit {
        /// The pull request
        number: NonZeroU64,
        /// Its new title
        title: String,
        /// Its new body
        body: String,
    },
    /// [`Pulls::state`], and the number
    State(NonZeroU64),
    /// [`Pulls::mark_ready`], and the number
    MarkReady(NonZeroU64),
    /// [`Pulls::merge`], with the number and the head it was pinned to
    Merge {
        /// The pull request
        number: NonZeroU64,
        /// The head the merge was pinned to
        head: String,
    },
    /// [`Pulls::disable_auto_merge`], and the number
    DisableAutoMerge(NonZeroU64),
}

#[derive(Debug, Default)]
struct PullScript {
    created: Option<Result<NonZeroU64, ForgeError>>,
    heads: BTreeMap<String, Result<Option<String>, ForgeError>>,
    edited: Option<Result<(), ForgeError>>,
    states: BTreeMap<NonZeroU64, Result<PullState, ForgeError>>,
    ready: Option<Result<(), ForgeError>>,
    merged: Option<Result<(), ForgeError>>,
    disarmed: Option<Result<(), ForgeError>>,
    asked: Vec<PullCall>,
}

/// A pull request source that answers from a script and records each call
///
/// With no answer scripted, a call succeeds: a draft is pull request 1, a branch
/// has no head, an edit, a ready mark, a merge and a disarm change nothing, and a
/// pull request is open.
#[derive(Debug, Default)]
pub struct FakePulls {
    script: Mutex<PullScript>,
}

impl FakePulls {
    /// A source with nothing scripted
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What [`Pulls::create_draft`] answers
    pub fn script_create(&self, answer: Result<NonZeroU64, ForgeError>) {
        self.script().created = Some(answer);
    }

    /// What [`Pulls::branch_head`] answers for `branch`
    pub fn script_branch_head(&self, branch: &str, answer: Result<Option<String>, ForgeError>) {
        self.script().heads.insert(branch.to_owned(), answer);
    }

    /// What [`Pulls::edit`] answers
    pub fn script_edit(&self, answer: Result<(), ForgeError>) {
        self.script().edited = Some(answer);
    }

    /// What [`Pulls::state`] answers for `number`
    pub fn script_state(&self, number: NonZeroU64, answer: Result<PullState, ForgeError>) {
        self.script().states.insert(number, answer);
    }

    /// What [`Pulls::mark_ready`] answers
    pub fn script_mark_ready(&self, answer: Result<(), ForgeError>) {
        self.script().ready = Some(answer);
    }

    /// What [`Pulls::merge`] answers, whatever head it is pinned to
    pub fn script_merge(&self, answer: Result<(), ForgeError>) {
        self.script().merged = Some(answer);
    }

    /// What [`Pulls::disable_auto_merge`] answers
    pub fn script_disable_auto_merge(&self, answer: Result<(), ForgeError>) {
        self.script().disarmed = Some(answer);
    }

    /// What it was asked, in order
    #[must_use]
    pub fn calls(&self) -> Vec<PullCall> {
        self.script().asked.clone()
    }

    fn script(&self) -> std::sync::MutexGuard<'_, PullScript> {
        self.script
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Pulls for FakePulls {
    fn create_draft(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<NonZeroU64, ForgeError> {
        let mut script = self.script();
        script.asked.push(PullCall::Create {
            head: head.to_owned(),
            base: base.to_owned(),
            title: title.to_owned(),
            body: body.to_owned(),
        });
        script.created.clone().unwrap_or(Ok(NonZeroU64::MIN))
    }

    fn branch_head(&self, branch: &str) -> Result<Option<String>, ForgeError> {
        let mut script = self.script();
        script.asked.push(PullCall::BranchHead(branch.to_owned()));
        script.heads.get(branch).cloned().unwrap_or(Ok(None))
    }

    fn edit(&self, number: NonZeroU64, title: &str, body: &str) -> Result<(), ForgeError> {
        let mut script = self.script();
        script.asked.push(PullCall::Edit {
            number,
            title: title.to_owned(),
            body: body.to_owned(),
        });
        script.edited.clone().unwrap_or(Ok(()))
    }

    fn state(&self, number: NonZeroU64) -> Result<PullState, ForgeError> {
        let mut script = self.script();
        script.asked.push(PullCall::State(number));
        script
            .states
            .get(&number)
            .cloned()
            .unwrap_or(Ok(PullState::Open))
    }

    fn mark_ready(&self, number: NonZeroU64) -> Result<(), ForgeError> {
        let mut script = self.script();
        script.asked.push(PullCall::MarkReady(number));
        script.ready.clone().unwrap_or(Ok(()))
    }

    fn merge(&self, number: NonZeroU64, head: &str) -> Result<(), ForgeError> {
        let mut script = self.script();
        script.asked.push(PullCall::Merge {
            number,
            head: head.to_owned(),
        });
        script.merged.clone().unwrap_or(Ok(()))
    }

    fn disable_auto_merge(&self, number: NonZeroU64) -> Result<(), ForgeError> {
        let mut script = self.script();
        script.asked.push(PullCall::DisableAutoMerge(number));
        script.disarmed.clone().unwrap_or(Ok(()))
    }
}
