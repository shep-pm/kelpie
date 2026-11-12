//! The gate between a worker's pull request and the maintainer's yes
//!
//! Each pass reads the pull request once and stops at the first step that has
//! something to do (`docs/design/gate-and-merge.md`, The order). The order is
//! decided here, apart from git and GitHub: [`pass`] takes what a pass read as
//! plain values and answers what to do about it, so each step is tested on
//! values. Nothing runs a pass yet. The pass that does reads the world into a
//! [`Look`], acts on the [`Step`], and saves the facts it changed in [`Book`].
//!
//! [`red`] and [`removed`] decide what a red run or a merge queue removal comes
//! to, [`conflicted`] what a conflict on catch-up comes to, [`yes`], [`refused`]
//! and [`queue`] what the maintainer's yes and the merge it sends come to, and
//! [`Ruling`] holds
//! what the gate will not settle itself. [`Kept`] is what the gate remembers of
//! a ticket between passes. Lifted from SK's `runner/gate/` (`check_ci`, `lag.rs`,
//! `catch_up.rs`, `fix_attempts.rs`), `runner/gate/queued/` and `rerun/`,
//! `runner/rework/`, and `runner/merge/`: `docs/lifted.md`.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::gate::Kept;
//!
//! let mut kept = Kept::default();
//! kept.pushed("0123456789abcdef");
//! assert_eq!(kept.pushed, ["0123456789abcdef"]);
//! ```

use crate::ci::{FailedCheck, HeadStatus, Queued};
use crate::github::Timestamp;
use crate::settings::{CatchUp, CiWait, FixAttempts, Resolved};
use crate::ticket::OpenPull;
use crate::worktree::Base;

mod kept;
mod merge;
mod prompt;
mod red;
mod ruling;
#[cfg(test)]
mod tests;

pub use kept::{Book, Conflict, FILE, Kept, Lag, QueuedReruns, Seen, Sent, Wait};
pub use merge::{Merge, Queue, Refusal, disarm, queue, refused, yes};
pub(crate) use prompt::KEEP_BODY_TRUE;
pub use red::{Logs, Red, red, removed};
pub use ruling::Ruling;

/// How long the forge's head may differ from the branch before the line says so
pub const LAG_LIMIT: u64 = 15 * 60;

/// The most conflict turns one ticket gets before a ruling, so a base that
/// moves before every check cannot loop the worker
pub const CONFLICT_TURNS: u32 = 3;

/// How long a push or a pull request marked ready is given to register its checks
///
/// A check set registers one check at a time (`docs/facts/ci-and-the-merge.md`
/// in SK), so a verdict waits until it has had time to register whole.
pub const CHECKS_SETTLE: u64 = 120;

/// How long a running check, or one with no run, is waited on past the CI wait
/// before a ruling, in seconds
///
/// GitHub lets a hosted job run six hours (SK's `JOB_TIMEOUT`). A running job
/// ends at its own timeout, so it is ruled on only past the longest one runs.
pub const JOB_TIMEOUT: u64 = 6 * 60 * 60;

/// The pull request as one pass read it
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pull {
    /// Still open, with what the listing carries
    Open(OpenPull),
    /// Merged, which counts as the maintainer's yes
    Merged,
    /// Closed without a merge
    Closed,
}

/// What one pass read, from git and GitHub
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    /// The pull request
    pub pull: Pull,
    /// Where its head stands against the base branch, just fetched
    pub base: Base,
    /// The commit the ticket's worktree has checked out, which the worker's last push left
    pub head: String,
    /// The harnesses' own files the diff holds
    pub agents_files: Vec<String>,
    /// Whether merging the head into the base branch is clean. Only read when the
    /// branch is behind and the project catches up on a conflict alone, so a caller
    /// with no answer says `false` and the branch is caught up, which finds the conflict
    pub merges_clean: bool,
    /// Whether the base branch holds a workflow that runs on a pull request
    pub base_runs_pull_workflow: bool,
}

/// The project's settings the gate reads (`docs/design/gate-and-merge.md`, Settings)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Whether CI holds the merge: off reads no checks and asks for the merge once the branch is current
    pub ci_holds_merge: bool,
    /// The fix turns red runs get for one ticket before a ruling, no cap when none
    pub fix_attempts: Option<FixAttempts>,
    /// How long a check may sit queued before a rerun, and a head with no check waits before a ruling
    pub ci_wait: CiWait,
    /// When a branch behind its base is caught up: always, or only on a conflict
    pub catch_up: CatchUp,
}

impl From<&Resolved> for Limits {
    fn from(resolved: &Resolved) -> Self {
        Self {
            ci_holds_merge: resolved.ci_holds_merge.value,
            fix_attempts: resolved.fix_attempts.value,
            ci_wait: resolved.ci_wait.value,
            catch_up: resolved.catch_up.value,
        }
    }
}

/// What a pass found to do
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// The maintainer merged it by hand: the ticket is Done
    Done,
    /// The gate will not settle it itself
    Rule(Ruling),
    /// Nothing to do yet, and the ticket's line says why
    Wait(Line),
    /// The branch is behind its base: rebase and push. A conflict goes to [`conflicted`].
    CatchUp,
    /// A conflict is the worker's next turn, with this prompt
    Conflict(String),
    /// Start these runs again. For a stuck queue, the first the forge accepts ends it
    Rerun(Vec<u64>),
    /// Red: read the failed runs' logs, then decide with [`red`]
    Red(Vec<FailedCheck>),
    /// Everything holds: ask the maintainer for the merge
    Yes,
}

impl Step {
    /// The line a Review ticket shows while this is where the gate is, none for [`Step::Done`]
    #[must_use]
    pub fn line(&self) -> Option<Line> {
        Some(match self {
            Self::Done => return None,
            Self::Rule(ruling) => Line::of(ruling),
            Self::Wait(line) => line.clone(),
            Self::CatchUp | Self::Conflict(_) => Line::CatchingUp,
            Self::Rerun(_) => Line::WaitingForCi,
            Self::Red(checks) => Line::FixingRed {
                link: checks.iter().find_map(|check| check.url.clone()),
            },
            Self::Yes => Line::ReadyToMerge,
        })
    }
}

/// The one status line a Review ticket shows
/// (`docs/design/gate-and-merge.md`, The Review column)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// Catching up with the base branch
    CatchingUp,
    /// Waiting for CI
    WaitingForCi,
    /// Fixing red CI, linking the failing run where it has a page
    FixingRed {
        /// The failing run's page
        link: Option<String>,
    },
    /// Ready to merge, on the maintainer's yes
    ReadyToMerge,
    /// In the merge queue
    InMergeQueue,
    /// The forge's head has not matched the branch for [`LAG_LIMIT`]
    BaseLagging {
        /// The head the forge shows
        head: String,
    },
    /// A ruling's reason, linking the failing run where it has one
    Ruling {
        /// The reason, the needs-you line
        reason: String,
        /// The failing run's page
        link: Option<String>,
    },
}

impl Line {
    /// The line of `ruling`
    #[must_use]
    pub fn of(ruling: &Ruling) -> Self {
        Self::Ruling {
            reason: ruling.reason(),
            link: ruling.link().map(str::to_owned),
        }
    }

    /// The line as the ticket shows it
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::CatchingUp => "Catching up with the base branch".to_owned(),
            Self::WaitingForCi => "Waiting for CI".to_owned(),
            Self::FixingRed { .. } => "Fixing red CI".to_owned(),
            Self::ReadyToMerge => "Ready to merge".to_owned(),
            Self::InMergeQueue => "In the merge queue".to_owned(),
            Self::BaseLagging { head } => format!(
                "GitHub has shown {} for {} minutes, but the branch is elsewhere. Reopen the \
                 pull request or push again.",
                short(head),
                LAG_LIMIT / 60
            ),
            Self::Ruling { reason, .. } => reason.clone(),
        }
    }

    /// The failing run's page, for the line that has one
    #[must_use]
    pub fn link(&self) -> Option<&str> {
        match self {
            Self::FixingRed { link } | Self::Ruling { link, .. } => link.as_deref(),
            _ => None,
        }
    }
}

/// One pass over a pull request: what to do about it, stopping at the first step with something to do
///
/// It changes `kept` only by what the pass itself saw: the labels and draft
/// state first seen, the head the wait is on, and the lag. What an act comes to
/// is recorded by the caller, with the [`Kept`] methods, once it has been done.
pub fn pass(look: &Look, kept: &mut Kept, limits: &Limits, now: Timestamp) -> Step {
    let pull = match &look.pull {
        // The maintainer merged it by hand, which is their own yes
        Pull::Merged => return Step::Done,
        Pull::Closed => return Step::Rule(Ruling::Closed),
        Pull::Open(pull) => pull,
    };
    let since = waiting_since(kept, &pull.head, now);
    if look.base == Base::Lagging {
        return lagging(&pull.head, kept, now);
    }
    kept.lag = None;
    let seen = kept
        .seen
        .get_or_insert_with(|| Seen {
            labels: pull.labels.clone(),
            draft: pull.draft,
        })
        .clone();
    if let Some(what) = foreign_change(&seen, kept, look, pull) {
        return Step::Rule(Ruling::ForeignChange { what });
    }
    if !look.agents_files.is_empty() {
        return Step::Rule(Ruling::AgentsFiles {
            files: look.agents_files.clone(),
        });
    }
    // Under `conflicts`, a branch behind its base that merges cleanly goes on
    // unrebased, to the checks below.
    if look.base == Base::Behind && (limits.catch_up == CatchUp::Always || !look.merges_clean) {
        return Step::CatchUp;
    }
    if !limits.ci_holds_merge {
        return Step::Yes;
    }
    if !settled(since, now) {
        return Step::Wait(Line::WaitingForCi);
    }
    match &pull.checks {
        HeadStatus::None => without_checks(look, &pull.head, since, limits, now),
        HeadStatus::Pending(queued) => pending(
            &pull.head,
            queued,
            kept,
            since,
            limits.ci_wait.duration().as_secs(),
            now,
        ),
        HeadStatus::Passed | HeadStatus::Skipped => Step::Yes,
        HeadStatus::Failed(checks) => Step::Red(checks.clone()),
    }
}

/// What a conflict on catch-up comes to: the worker's next turn, or a ruling
///
/// `head` is the head that conflicted with `base`, the base's commit, in
/// `files`. A conflict the worker was already sent for at this head or this
/// base is one it did not resolve, and a ticket with [`CONFLICT_TURNS`] of them
/// has had enough: those are a ruling. The caller aborts the merge it started,
/// and otherwise records the turn with [`Kept::conflict_sent`] once sent.
#[must_use]
pub fn conflicted(kept: &Kept, head: &str, base: &str, files: &[String]) -> Step {
    let tried = kept
        .conflict
        .as_ref()
        .is_some_and(|sent| sent.head == head || sent.base == base || sent.turns >= CONFLICT_TURNS);
    if tried {
        return Step::Rule(Ruling::Conflict {
            files: files.to_vec(),
        });
    }
    Step::Conflict(prompt::conflict(files))
}

// When the wait for `head`'s checks began, starting it at `now` for a head that has none
fn waiting_since(kept: &mut Kept, head: &str, now: Timestamp) -> Timestamp {
    match &kept.wait {
        Some(wait) if wait.head == head => wait.since,
        _ => {
            kept.wait = Some(Wait {
                head: head.to_owned(),
                since: now,
            });
            now
        }
    }
}

/// Whether a check set has had its time to register since `since`
#[must_use]
pub fn settled(since: Timestamp, now: Timestamp) -> bool {
    now.secs().saturating_sub(since.secs()) >= CHECKS_SETTLE
}

/// The first seven characters of a commit hash, as git shows it
#[must_use]
pub fn short(head: &str) -> &str {
    head.get(..7).unwrap_or(head)
}

// The forge's head lags a push by a moment, so a pass waits until the two
// match. A forge that stops updating never lets them, so the line names it past
// the limit. A lag belongs to the head it began on.
fn lagging(forged: &str, kept: &mut Kept, now: Timestamp) -> Step {
    let since = match &kept.lag {
        Some(lag) if lag.head == forged => lag.since,
        _ => {
            kept.lag = Some(Lag {
                head: forged.to_owned(),
                since: now,
            });
            now
        }
    };
    if now.secs().saturating_sub(since.secs()) < LAG_LIMIT {
        return Step::Wait(Line::WaitingForCi);
    }
    Step::Wait(Line::BaseLagging {
        head: forged.to_owned(),
    })
}

// Whether the labels, the draft state or the head differ from what the gate
// saw first and what its workers pushed, and what changed, named plainly enough
// to answer from a phone
fn foreign_change(seen: &Seen, kept: &Kept, look: &Look, pull: &OpenPull) -> Option<String> {
    let added: Vec<&str> = (pull.labels.iter())
        .filter(|label| !seen.labels.contains(label))
        .map(String::as_str)
        .collect();
    let removed: Vec<&str> = (seen.labels.iter())
        .filter(|label| !pull.labels.contains(label))
        .map(String::as_str)
        .collect();
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!(
            "the `{}` {} added",
            added.join("`, `"),
            label_or_labels(added.len())
        ));
    }
    if !removed.is_empty() {
        parts.push(format!(
            "the `{}` {} removed",
            removed.join("`, `"),
            label_or_labels(removed.len())
        ));
    }
    if !pull.draft && seen.draft {
        parts.push("it was marked ready for review".to_owned());
    } else if pull.draft && !seen.draft {
        parts.push("it was marked a draft again".to_owned());
    }
    let own = pull.head == look.head || kept.pushed.contains(&pull.head);
    if !own {
        parts.push(format!(
            "its head moved to {}, a commit the worker did not push",
            short(&pull.head)
        ));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

fn label_or_labels(n: usize) -> &'static str {
    if n == 1 { "label was" } else { "labels were" }
}

// A head the forge reports no check on: with no workflow on the base that runs
// on a pull request, no check will ever appear, so it goes on as skipped. With
// one, a check may still be starting, so the head waits the CI wait.
fn without_checks(
    look: &Look,
    head: &str,
    since: Timestamp,
    limits: &Limits,
    now: Timestamp,
) -> Step {
    if !look.base_runs_pull_workflow {
        return Step::Yes;
    }
    if now.secs().saturating_sub(since.secs()) < limits.ci_wait.duration().as_secs() {
        return Step::Wait(Line::WaitingForCi);
    }
    Step::Rule(Ruling::NoChecks {
        head: head.to_owned(),
    })
}

// Checks held for approval, or stuck on a finished run. GitHub sometimes leaves
// one job of a finished run queued, and the rollup cannot tell a running job
// from a stuck one, so both are waited on past the CI wait. Past that the run is
// rerun whole, once per head, and still stuck it is a ruling, unless every check
// left is running. Held runs are never rerun, and a head held entirely is ruled
// on once its checks have had time to register.
fn pending(
    head: &str,
    queued: &[Queued],
    kept: &Kept,
    since: Timestamp,
    cap: u64,
    now: Timestamp,
) -> Step {
    let waited = now.secs().saturating_sub(since.secs());
    if !queued.is_empty() && queued.iter().all(|check| check.held) {
        return Step::Rule(Ruling::CheckQueued {
            head: head.to_owned(),
            checks: queued.iter().map(|check| check.name.clone()).collect(),
            held: true,
            run: None,
        });
    }
    let active: Vec<&Queued> = queued.iter().filter(|check| !check.held).collect();
    if waited < cap || active.is_empty() {
        return Step::Wait(Line::WaitingForCi);
    }
    let rerun = |run: u64| {
        kept.queued
            .as_ref()
            .is_some_and(|done| done.head == head && done.runs.contains(&run))
    };
    let mut runs: Vec<u64> = Vec::new();
    for run in active.iter().filter_map(|check| check.run) {
        if !runs.contains(&run) && !rerun(run) {
            runs.push(run);
        }
    }
    if !runs.is_empty() {
        return Step::Rerun(runs);
    }
    // A job that is running ends at its own timeout, so one running check is
    // waited on to that limit, and a check with no run, which cannot be rerun,
    // is waited on to the same limit.
    let stuck: Vec<&Queued> = active
        .iter()
        .copied()
        .filter(|check| !check.running)
        .collect();
    let first = stuck.iter().find_map(|check| check.run);
    let running = stuck.len() < active.len();
    if (running || first.is_none()) && waited < JOB_TIMEOUT {
        return Step::Wait(Line::WaitingForCi);
    }
    // The ruling names the checks that are stuck, not the ones running fine. Past
    // the timeout a running job is stuck too.
    let named = if stuck.is_empty() { &active } else { &stuck };
    let run = named.iter().find_map(|check| check.run);
    Step::Rule(Ruling::CheckQueued {
        head: head.to_owned(),
        checks: named
            .iter()
            .filter(|check| check.run == run)
            .map(|check| check.name.clone())
            .collect(),
        held: false,
        run,
    })
}
