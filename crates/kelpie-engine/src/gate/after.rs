//! What a merged ticket's issue comes to, decided on plain values
//!
//! A merge by kelpie or by hand leaves nothing behind (`docs/design/gate-and-merge.md`,
//! After the merge). An issue the pull request names as closed is left to GitHub for
//! [`FORGE_CLOSE_WAIT`]. One still open after that, or that no pull request names, is
//! commented on once and closed, with [`TRIES`] tries. [`Merged`] is what the gate keeps
//! of one such ticket between reads, in `gate.json`, and [`closing`] answers what the
//! next read does about the issue. Lifted from SK's `runner/close_merged.rs`:
//! `docs/lifted.md`.

use serde::{Deserialize, Serialize};

use crate::github::Timestamp;

/// How long GitHub is left to close an issue the pull request names, in seconds
///
/// Never measured: SK's `docs/facts/ci-and-the-merge.md` leaves it to the maintainer.
pub const FORGE_CLOSE_WAIT: u64 = 600;

/// The most times kelpie asks the forge to close one issue before it gives up
pub const TRIES: u32 = 3;

/// What the gate keeps of a ticket whose pull request merged, until it is cleaned up
// wire format: `gate.json` holds this
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merged {
    /// When the gate first read the pull request merged
    pub since: Timestamp,
    /// Whether the pull request names the issue as one it closes
    pub names: bool,
    /// Whether the comment was made, which is once whether or not it went through
    #[serde(default)]
    pub commented: bool,
    /// The closes the forge refused or could not answer
    #[serde(default)]
    pub tries: u32,
    /// Whether the issue is settled: closed, or given up on
    #[serde(default)]
    pub settled: bool,
    /// Whether the worker's session is stopped and its worktree, build folder and branch are gone
    #[serde(default)]
    pub cleaned: bool,
}

impl Merged {
    /// A ticket whose pull request was first read merged at `now`
    ///
    /// `names` is whether that pull request names the ticket's issue.
    #[must_use]
    pub const fn new(now: Timestamp, names: bool) -> Self {
        Self {
            since: now,
            names,
            commented: false,
            tries: 0,
            settled: false,
            cleaned: false,
        }
    }

    /// Records that the comment was made, so a later try does not make it again
    pub const fn commented(&mut self) {
        self.commented = true;
    }

    /// Records a close the forge did not take
    pub const fn close_failed(&mut self) {
        self.tries = self.tries.saturating_add(1);
    }

    /// Records the issue as closed, or given up on
    pub const fn settle(&mut self) {
        self.settled = true;
    }

    /// Records the worker's session, worktree, build folder and branch as gone
    pub const fn clean(&mut self) {
        self.cleaned = true;
    }
}

/// What the next read does about a merged ticket's issue
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Close {
    /// The forge closed it, or it is no longer open: nothing to write
    Closed,
    /// GitHub is still given its time to close it
    Wait,
    /// Comment (unless one was made) and close
    Write {
        /// Whether the comment is still to be made
        comment: bool,
    },
    /// The closes are used up: leave the issue open for the maintainer
    GiveUp,
}

/// What the next read does about the issue, given whether the forge still shows it `open`
#[must_use]
pub fn closing(merged: &Merged, open: bool, now: Timestamp) -> Close {
    if !open {
        return Close::Closed;
    }
    if merged.names && now.secs().saturating_sub(merged.since.secs()) < FORGE_CLOSE_WAIT {
        return Close::Wait;
    }
    if merged.tries >= TRIES {
        return Close::GiveUp;
    }
    Close::Write {
        comment: !merged.commented,
    }
}

/// The words of the comment kelpie leaves on an issue it closes, for pull request `number`
#[must_use]
pub fn comment(number: u64) -> String {
    format!("Closed by kelpie: pull request #{number} merged, and GitHub left this issue open.")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> Timestamp {
        Timestamp::from_secs(secs)
    }

    #[test]
    fn an_issue_the_pull_names_waits_for_github_until_the_wait_is_over() {
        let merged = Merged::new(at(1_000), true);

        assert_eq!(closing(&merged, true, at(1_000)), Close::Wait);
        assert_eq!(
            closing(&merged, true, at(1_000 + FORGE_CLOSE_WAIT - 1)),
            Close::Wait
        );
        assert_eq!(
            closing(&merged, true, at(1_000 + FORGE_CLOSE_WAIT)),
            Close::Write { comment: true }
        );
    }

    #[test]
    fn an_issue_no_pull_names_is_closed_at_once() {
        let merged = Merged::new(at(1_000), false);

        assert_eq!(
            closing(&merged, true, at(1_000)),
            Close::Write { comment: true }
        );
    }

    #[test]
    fn an_issue_the_forge_shows_closed_is_left_alone() {
        let merged = Merged::new(at(1_000), false);

        assert_eq!(closing(&merged, false, at(1_000)), Close::Closed);
    }

    #[test]
    fn a_close_that_failed_comments_no_second_time_and_gives_up_after_three() {
        let mut merged = Merged::new(at(0), false);
        merged.commented();
        merged.close_failed();
        assert_eq!(
            closing(&merged, true, at(1)),
            Close::Write { comment: false }
        );

        merged.close_failed();
        merged.close_failed();

        assert_eq!(closing(&merged, true, at(1)), Close::GiveUp);
    }

    #[test]
    fn what_the_gate_keeps_of_a_merged_ticket_reads_back() {
        let mut merged = Merged::new(at(5), true);
        merged.commented();
        merged.close_failed();

        let json = serde_json::to_string(&merged).unwrap();

        assert_eq!(serde_json::from_str::<Merged>(&json).unwrap(), merged);
    }
}
