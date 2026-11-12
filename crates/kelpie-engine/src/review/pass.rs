//! Where a pull request's review pass stands: its round, that round's stage and what it has run
//!
//! One pass runs the project's reviewers once each, in order
//! (`docs/design/review.md`, The pass). A round that finds anything, nits
//! included, sends the worker all of its findings for one fix turn before the
//! next reviewer reads. Nothing here runs a round: [`Review`] is the state a
//! round is stepped from, and it round-trips through JSON as a work item's
//! state file keeps it.

use serde::{Deserialize, Serialize};

use super::Finding;

/// How many calls of one reviewer in a row may fail before it is passed over for the pass
///
/// A reply it cannot give would otherwise stall the review for good.
pub const ROUND_FAILURES: u32 = 3;

/// Where a review pass stands
///
/// A pass runs the project's reviewers once each, in order. After the last, or
/// once it has its reads, the pull request goes back to the gate.
// wire format: changing this is a breaking change to the state file
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    /// The round now running or about to run, 1-indexed from the start of its pass
    pub round: u32,

    /// Where this round stands
    pub stage: ReviewStage,

    /// Who reviews this round, once it has started
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewer: Option<String>,

    /// The reviewers this pass has run, oldest first, as the project lists them
    ///
    /// The next round goes to the first listed reviewer not among them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ran: Vec<String>,

    /// The calls of this round's reviewer that failed in a row
    #[serde(default, skip_serializing_if = "is_zero")]
    pub failures: u32,

    /// Whether no reviewer of this pass has read the pull request yet: none has come back with
    /// findings or clean
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unread: bool,

    /// The head this round's reviewer reads: the pushed head, which the worktree held, clean, as
    /// the round started
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading: Option<String>,
}

/// Whether `n` is zero, for a field that is left out of the state file while it is
// serde's `skip_serializing_if` hands the field by reference
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Review {
    /// The first round of a pass, about to run
    #[must_use]
    pub fn first() -> Self {
        Self {
            round: 1,
            stage: ReviewStage::Round,
            reviewer: None,
            ran: Vec::new(),
            failures: 0,
            unread: true,
            reading: None,
        }
    }

    /// Counts one more failed call of this round's reviewer, and says whether it is now passed
    /// over: it has failed [`ROUND_FAILURES`] times in a row
    ///
    /// While it is not, the call stays due.
    pub fn fail(&mut self) -> bool {
        self.failures += 1;
        self.failures >= ROUND_FAILURES
    }

    /// The next round of the same pass, once this one's reviewer is done
    ///
    /// The reviewer just done joins `ran` once, so none runs twice in a pass. The stage, the
    /// failures and the head read start over.
    #[must_use]
    pub fn next_round(self) -> Self {
        let mut ran = self.ran;
        ran.extend(self.reviewer.filter(|name| !ran.contains(name)));
        Self {
            round: self.round + 1,
            stage: ReviewStage::Round,
            reviewer: None,
            ran,
            failures: 0,
            unread: self.unread,
            reading: None,
        }
    }
}

/// Where one review round stands
// wire format: changing this is a breaking change to the state file
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ReviewStage {
    /// About to run this round's reviewer
    Round,

    /// The round's findings, about to go to the worker
    Found {
        /// What the reviewer found
        findings: Vec<Finding>,
    },

    /// The round's findings were sent to the worker; waiting for its fix
    Fixing {
        /// The pull request's head when the findings were sent, which a fix moves
        head: String,

        /// The findings sent
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        sent: Vec<Finding>,

        /// The deferred findings file's lines as the findings were sent, so only a deferral
        /// written since counts against them
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        deferred_before: Vec<Finding>,
    },

    /// The worktree was not the head on `origin` as this round was about to run, so the worker
    /// has a turn to push or discard
    ///
    /// Once it ends, the round runs on a worktree at the pushed head, or the ticket parks.
    Pushing,

    /// A reviewer with a second look has read once, and a fresh session of it is about to read
    /// again, shown what the first found
    ///
    /// Both lists then go on as `Found`.
    SecondLook {
        /// What the first read found
        first: Vec<Finding>,
    },
}

/// What both looks found, each finding once, the worst first
///
/// The first look's finding stands where both say the same one, and a stable sort keeps each
/// look's own order within a severity.
#[must_use]
pub fn both_looks(first: Vec<Finding>, more: Vec<Finding>) -> Vec<Finding> {
    let mut all = first;
    for f in more {
        if !all.iter().any(|k| k.is_same_as(&f)) {
            all.push(f);
        }
    }
    all.sort_by_key(|f| std::cmp::Reverse(f.severity));
    all
}
