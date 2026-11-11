//! What the gate rules on, and the reason each puts on the ticket's needs-you line
//!
//! A ruling is something the worker will not fix on its own (`GLOSSARY.md`,
//! Ruling). The maintainer answers it with a Note or by acting on GitHub, which
//! the next pass reads (`docs/design/gate-and-merge.md`, Rulings in this slice),
//! so a reason names what happened and asks for no command.

use super::short;

/// What the gate ruled on, with what its reason names
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ruling {
    /// The pull request was closed without a merge
    Closed,
    /// Its labels, draft state or head changed by someone else
    ForeignChange {
        /// What changed, named plainly
        what: String,
    },
    /// The diff holds a harness's own file
    AgentsFiles {
        /// The files, as the diff names them
        files: Vec<String>,
    },
    /// Catch-up still conflicts after the worker's fix turns
    Conflict {
        /// The files that conflict
        files: Vec<String>,
    },
    /// No check on the head after the wait, with a workflow on the base that runs on a pull request
    NoChecks {
        /// The head
        head: String,
    },
    /// A check still queued after one rerun, held for approval, or never finished
    CheckQueued {
        /// The head
        head: String,
        /// The checks still queued
        checks: Vec<String>,
        /// The workflow runs wait for a maintainer's approval
        held: bool,
        /// The run they belong to, none for an app's check
        run: Option<u64>,
    },
    /// Red with nothing pushed, or past the fix attempts
    StillRed {
        /// The head
        head: String,
        /// The checks that failed
        checks: Vec<String>,
        /// The fix turns the worker had, when the project's fix attempts are what
        /// raised it, and none when the worker pushed no fix
        fix_turns: Option<u32>,
        /// The failing run's page, when a check has one
        link: Option<String>,
    },
    /// Catch-up left the branch as it was, for a reason only the maintainer can settle
    CatchUpRefused {
        /// Why, as the catch-up said
        why: String,
    },
    /// Refused twice, or removed from the merge queue twice at one head
    MergeRefused {
        /// The head
        head: String,
        /// The forge's reason
        why: String,
    },
}

impl Ruling {
    /// The reason, which becomes the ticket's needs-you line
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::Closed => "The pull request was closed without merging.".to_owned(),
            Self::ForeignChange { what } => format!(
                "Someone else changed the pull request: {what}. A Note tells the worker what \
                 to do, or put it back on GitHub."
            ),
            Self::AgentsFiles { files } => format!(
                "The branch changes a harness's own files: {}.",
                files.join(", ")
            ),
            Self::Conflict { files } => format!(
                "The branch still conflicts with its base in {} after the worker's fix turns.",
                files.join(", ")
            ),
            Self::NoChecks { head } => format!(
                "No check has appeared at {} in the CI wait, though a workflow runs on pull \
                 requests.",
                short(head)
            ),
            Self::CheckQueued {
                head,
                checks,
                held: true,
                ..
            } => format!(
                "{} at {} wait for your approval on GitHub. Approving runs the pull request's \
                 code on GitHub's runners, not on this machine.",
                checks.join(", "),
                short(head)
            ),
            Self::CheckQueued {
                head,
                checks,
                run: None,
                ..
            } => format!(
                "A check from an app never finished at {}: {}. Kelpie cannot rerun it, but it \
                 can be retried where it was posted.",
                short(head),
                checks.join(", ")
            ),
            Self::CheckQueued {
                head, checks, run, ..
            } => format!(
                "{} stayed queued at {} after its run{} was rerun.",
                checks.join(", "),
                short(head),
                run.map_or_else(String::new, |run| format!(" ({run})"))
            ),
            Self::StillRed {
                head,
                checks,
                fix_turns: None,
                ..
            } => format!(
                "CI failed again at {}, and the worker pushed no fix: {}.",
                short(head),
                checks.join(", ")
            ),
            Self::StillRed {
                head,
                checks,
                fix_turns: Some(turns),
                ..
            } => format!(
                "CI failed at {}: {}. {}, and the project's fix attempts are used up.",
                short(head),
                checks.join(", "),
                fix_turns_had(*turns)
            ),
            Self::CatchUpRefused { why } => {
                format!("Kelpie could not catch the branch up with its base: {why}.")
            }
            Self::MergeRefused { head, why } => {
                format!("Kelpie could not merge at {}: {why}.", short(head))
            }
        }
    }

    /// The failing run's page, for the ruling that has one
    #[must_use]
    pub fn link(&self) -> Option<&str> {
        match self {
            Self::StillRed { link, .. } => link.as_deref(),
            _ => None,
        }
    }
}

fn fix_turns_had(turns: u32) -> String {
    match turns {
        0 => "The worker has had no fix turn on red runs".to_owned(),
        1 => "The worker has had 1 fix turn on red runs".to_owned(),
        n => format!("The worker has had {n} fix turns on red runs"),
    }
}
