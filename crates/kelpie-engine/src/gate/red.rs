//! What a red run, or a merge queue removal, comes to
//!
//! As SK's `ci_failed` (`runner/gate/mod.rs`), `rerun_if_unrelated`
//! (`runner/gate/rerun/mod.rs`) and `back_to_worker` (`runner/rework/`), in
//! the note's order (`docs/design/gate-and-merge.md`, Red CI). The caller reads
//! the failed runs' logs and the files the branch changed, and acts on the
//! answer: it records the act with [`Kept::fix_turn_sent`] or
//! [`Kept::reran_failed`] once the act is done.

use super::kept::Kept;
use super::ruling::Ruling;
use super::{Limits, prompt};
use crate::ci::{FailedCheck, FailedLog};

/// What kelpie found in the failed runs' logs of a head
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Logs {
    /// The end of each failed run's log, empty where kelpie found no failed run to read
    Read(Vec<FailedLog>),
    /// The CI refused the read, with its reason
    Refused(String),
}

/// What a red run comes to
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Red {
    /// The gate will not settle it itself
    Rule(Ruling),
    /// The failure is likely not the worker's: start these failed runs again, all of them
    Rerun(Vec<u64>),
    /// The worker's next turn, with this prompt
    FixTurn(String),
}

/// What the failed `checks` at `head` come to, given their `logs` and the files the branch `changed`
///
/// 1. A head that was red before with nothing pushed since is a ruling: still red.
/// 2. Logs that name no file the branch changed rerun the runs once per head,
///    since the failure is likely not the worker's.
/// 3. Past the project's fix attempts, a ruling: still red. 0 rules on the first red run.
/// 4. Otherwise a fix turn: the check's name and its log, starting from `origin`'s head.
#[must_use]
pub fn red(
    head: &str,
    checks: &[FailedCheck],
    logs: &Logs,
    changed: &[String],
    kept: &Kept,
    limits: &Limits,
) -> Red {
    let still_red = |fix_turns| {
        Red::Rule(Ruling::StillRed {
            head: head.to_owned(),
            checks: checks.iter().map(|check| check.name.clone()).collect(),
            fix_turns,
            link: checks.iter().find_map(|check| check.url.clone()),
        })
    };
    if kept.red.iter().any(|red| red == head) {
        return still_red(None);
    }
    let rerun = kept.reran.iter().any(|reran| reran == head);
    if !rerun && let Some(runs) = unrelated(logs, changed) {
        return Red::Rerun(runs);
    }
    let turns = kept.fix_turns;
    if limits.fix_attempts.is_some_and(|cap| turns >= cap.count()) {
        return still_red(Some(turns));
    }
    Red::FixTurn(prompt::red(head, checks, logs, rerun))
}

/// What a merge queue removal of `head` for `reason` comes to
///
/// It goes back to the worker like red CI, uncounted, with "merge queue" as the
/// check's name. A second removal at the same head is a ruling: merge refused.
#[must_use]
pub fn removed(kept: &Kept, head: &str, reason: &str) -> Red {
    if kept.red.iter().any(|red| red == head) {
        return Red::Rule(Ruling::MergeRefused {
            head: head.to_owned(),
            why: reason.to_owned(),
        });
    }
    Red::FixTurn(prompt::queue(head, reason))
}

// The runs to rerun when the logs name no file the branch changed, none when
// they name one, or could not be read. A log that is empty, or a branch with no
// changed file read, leaves the run to the worker.
fn unrelated(logs: &Logs, changed: &[String]) -> Option<Vec<u64>> {
    let Logs::Read(logs) = logs else {
        return None;
    };
    let text: String = logs.iter().map(|log| log.tail.as_str()).collect();
    let related = text.trim().is_empty()
        || changed.is_empty()
        || changed.iter().any(|file| text.contains(file));
    if related {
        return None;
    }
    let mut runs: Vec<u64> = Vec::new();
    for log in logs {
        if !runs.contains(&log.run) {
            runs.push(log.run);
        }
    }
    Some(runs)
}
