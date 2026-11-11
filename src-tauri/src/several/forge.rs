//! One stand-in GitHub for the cases to share
//!
//! [`Github`] is the project's [`Feed`], its [`GithubTickets`] and its [`Pulls`] at
//! once, over one state. A draft opened through [`Pulls::create_draft`] is what the
//! next [`GithubTickets::open_pulls`] lists, as it is on GitHub, so a board read
//! after a hand-over sees the pull request the hand-over opened. No call here needs
//! `gh` or the network.
//!
//! A merge of a draft, of an ended pull request, or pinned to a head the branch no
//! longer holds is refused, as GitHub refuses it, and a merged pull request leaves
//! the open ones. Later slices grow
//! it with checks and closes.

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::{Mutex, MutexGuard, PoisonError};

use kelpie_engine::github::{ForgeSlug, Timestamp};
use kelpie_engine::ticket::{
    ForgeError, GithubTickets, Issue, Listed, OpenPull, PullState, Pulls, TicketId, Tickets,
};

use crate::board::poll::Feed;

/// The login the stand-in's `gh` holds
pub(super) const VIEWER: &str = "octocat";

/// An issue as the stand-in keeps it
#[derive(Debug, Clone)]
struct Stored {
    listed: Listed,
    body: String,
    open: bool,
}

/// A pull request as the stand-in keeps it
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pull {
    /// Its number
    pub number: NonZeroU64,

    /// The branch it is opened from
    pub branch: String,

    /// The base branch it is opened against
    pub base: String,

    /// Its title
    pub title: String,

    /// Its body
    pub body: String,

    /// Whether it is still a draft
    pub draft: bool,

    /// The tickets its body closes
    pub closes: Vec<TicketId>,

    /// Whether it is open, merged or closed
    pub state: PullState,
}

#[derive(Debug, Default)]
struct State {
    issues: BTreeMap<TicketId, Stored>,
    pulls: Vec<Pull>,

    /// The commit each branch holds on the forge, which is what a push leaves there
    heads: BTreeMap<String, String>,
}

impl State {
    fn pull_mut(&mut self, number: NonZeroU64) -> Result<&mut Pull, ForgeError> {
        self.pulls
            .iter_mut()
            .find(|pull| pull.number == number)
            .ok_or_else(|| ForgeError::Failed(format!("no pull request {number}")))
    }
}

/// The project's GitHub, stood in
#[derive(Debug)]
pub(super) struct Github {
    repo: ForgeSlug,
    state: Mutex<State>,
}

impl Github {
    /// A GitHub holding no issue and no pull request
    pub(super) fn new() -> Self {
        Self {
            repo: ForgeSlug::try_from("shep-pm/kelpie".to_owned()).unwrap(),
            state: Mutex::default(),
        }
    }

    /// Open issue `number`, which `author` wrote, with `labels`
    pub(super) fn open_issue(
        &self,
        number: u64,
        title: &str,
        body: &str,
        labels: &[&str],
        author: &str,
    ) {
        let id = TicketId::from_issue(number).unwrap();
        self.state().issues.insert(
            id,
            Stored {
                listed: Listed {
                    id,
                    title: title.to_owned(),
                    labels: labels.iter().map(|label| (*label).to_owned()).collect(),
                    author: author.to_owned(),
                    created: Timestamp::from_secs(1_700_000_000 + number),
                },
                body: body.to_owned(),
                open: true,
            },
        );
    }

    /// What a push of `branch` leaves on GitHub: the branch holds `head`
    pub(super) fn push(&self, branch: &str, head: &str) {
        self.state()
            .heads
            .insert(branch.to_owned(), head.to_owned());
    }

    /// Every pull request, oldest first
    pub(super) fn pulls(&self) -> Vec<Pull> {
        self.state().pulls.clone()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The tickets `body` closes, as GitHub reads a `Resolves #n` line
fn closes(body: &str) -> Vec<TicketId> {
    body.lines()
        .filter_map(|line| line.strip_prefix("Resolves #"))
        .filter_map(|number| number.trim().parse().ok())
        .filter_map(TicketId::from_issue)
        .collect()
}

impl Tickets for Github {
    fn issue(&self, id: &TicketId) -> Result<Issue, ForgeError> {
        self.state()
            .issues
            .get(id)
            .map(|stored| Issue {
                title: stored.listed.title.clone(),
                body: stored.body.clone(),
                labels: stored.listed.labels.clone(),
                open: stored.open,
            })
            .ok_or(ForgeError::NoIssue { ticket: *id })
    }

    fn ready_issues(&self, label: &str) -> Result<Vec<Listed>, ForgeError> {
        Ok(self
            .state()
            .issues
            .values()
            .filter(|stored| stored.open && stored.listed.labels.iter().any(|l| l == label))
            .map(|stored| stored.listed.clone())
            .collect())
    }
}

impl GithubTickets for Github {
    fn open_issues(&self) -> Result<Vec<Listed>, ForgeError> {
        Ok(self
            .state()
            .issues
            .values()
            .filter(|stored| stored.open)
            .map(|stored| stored.listed.clone())
            .collect())
    }

    fn closed_issues(&self, _since: Timestamp) -> Result<Vec<Listed>, ForgeError> {
        Ok(self
            .state()
            .issues
            .values()
            .filter(|stored| !stored.open)
            .map(|stored| stored.listed.clone())
            .collect())
    }

    fn open_pulls(&self) -> Result<Vec<OpenPull>, ForgeError> {
        Ok(self
            .state()
            .pulls
            .iter()
            .filter(|pull| pull.state == PullState::Open)
            .map(|pull| OpenPull {
                number: pull.number,
                branch: pull.branch.clone(),
                draft: pull.draft,
                closes: pull.closes.clone(),
            })
            .collect())
    }
}

impl Feed for Github {
    fn repo(&self) -> &ForgeSlug {
        &self.repo
    }

    fn viewer(&self) -> Result<String, ForgeError> {
        Ok(VIEWER.to_owned())
    }

    fn lifts_at(&self) -> Option<Timestamp> {
        None
    }
}

impl Pulls for Github {
    fn create_draft(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<NonZeroU64, ForgeError> {
        let mut state = self.state();
        let number = NonZeroU64::new(state.pulls.len() as u64 + 1).unwrap();
        state.pulls.push(Pull {
            number,
            branch: head.to_owned(),
            base: base.to_owned(),
            title: title.to_owned(),
            body: body.to_owned(),
            draft: true,
            closes: closes(body),
            state: PullState::Open,
        });
        Ok(number)
    }

    fn branch_head(&self, branch: &str) -> Result<Option<String>, ForgeError> {
        Ok(self.state().heads.get(branch).cloned())
    }

    fn edit(&self, number: NonZeroU64, title: &str, body: &str) -> Result<(), ForgeError> {
        let mut state = self.state();
        let pull = state
            .pulls
            .iter_mut()
            .find(|pull| pull.number == number)
            .ok_or_else(|| ForgeError::Failed(format!("no pull request {number}")))?;
        pull.title = title.to_owned();
        pull.body = body.to_owned();
        pull.closes = closes(body);
        Ok(())
    }

    fn state(&self, number: NonZeroU64) -> Result<PullState, ForgeError> {
        self.state().pull_mut(number).map(|pull| pull.state)
    }

    fn mark_ready(&self, number: NonZeroU64) -> Result<(), ForgeError> {
        self.state().pull_mut(number)?.draft = false;
        Ok(())
    }

    fn merge(&self, number: NonZeroU64, head: &str) -> Result<(), ForgeError> {
        let mut state = self.state();
        let pull = state.pull_mut(number)?;
        if pull.draft || pull.state != PullState::Open {
            return Err(ForgeError::Failed(
                "Pull request is not mergeable: it is a draft or already ended.".to_owned(),
            ));
        }
        let branch = pull.branch.clone();
        if state.heads.get(&branch).map(String::as_str) != Some(head) {
            return Err(ForgeError::Failed(
                "Head branch was modified. Review and try the merge again.".to_owned(),
            ));
        }
        state.pull_mut(number)?.state = PullState::Merged;
        Ok(())
    }

    fn disable_auto_merge(&self, number: NonZeroU64) -> Result<(), ForgeError> {
        self.state().pull_mut(number).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft_on_main() -> (Github, NonZeroU64) {
        let github = Github::new();
        github.push("kelpie/7", "abc");
        let number = github
            .create_draft("kelpie/7", "main", "feat: x", "Resolves #7")
            .unwrap();
        (github, number)
    }

    #[test]
    fn a_draft_or_an_ended_pull_request_is_refused_a_merge_as_github_refuses_it() {
        let (github, number) = draft_on_main();

        let draft = github.merge(number, "abc");
        github.mark_ready(number).unwrap();
        let ready = github.merge(number, "abc");
        let again = github.merge(number, "abc");

        assert!(matches!(draft, Err(ForgeError::Failed(_))), "{draft:?}");
        assert_eq!(ready, Ok(()));
        assert!(matches!(again, Err(ForgeError::Failed(_))), "{again:?}");
        assert_eq!(Pulls::state(&github, number), Ok(PullState::Merged));
    }
}
