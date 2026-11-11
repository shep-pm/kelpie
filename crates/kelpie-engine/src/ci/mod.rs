//! Where CI stands on a head, and the port the gate waits on it through
//!
//! The gate asks one port, so another CI system can stand behind it later
//! (`docs/design/gate-and-merge.md`, Ports). [`Ci`] reads a head's checks into a
//! [`HeadStatus`], reruns one run, and reads the logs of the runs that failed.
//! Nothing here talks to GitHub: the CI that does is
//! `github::gh::GithubActions`, which also holds what is GitHub Actions' alone,
//! such as the six hours a job may run.
//!
//! `fake::FakeCi` (under the `testing` feature) answers each method from a
//! script for the code that reads the port.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::ci::HeadStatus;
//!
//! assert_ne!(HeadStatus::Passed, HeadStatus::Skipped);
//! ```

use crate::ticket::ForgeError;

#[cfg(any(test, feature = "testing"))]
pub mod fake;
#[cfg(test)]
mod tests;

/// Where CI stands on one head
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadStatus {
    /// No check has reported on the head
    None,
    /// At least one check is pending, including a workflow held for approval
    Pending(Vec<Queued>),
    /// Every check finished, and none failed
    Passed,
    /// Every check finished and was skipped, as a workflow whose jobs all skip a
    /// draft leaves on one: nothing ran, and it counts as passed once the pull
    /// request is ready
    Skipped,
    /// Every check finished, and these failed
    Failed(Vec<FailedCheck>),
}

/// A pending check, or a workflow held for approval
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    /// The check's name
    pub name: String,
    /// The run it belongs to, when its link names one
    pub run: Option<u64>,
    /// It started and has not finished, so it ends at its own timeout. A queued
    /// one waits for a runner that may never come.
    pub running: bool,
    /// The CI holds the workflow until a maintainer approves its run
    pub held: bool,
}

/// A check that finished red
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedCheck {
    /// The check's name
    pub name: String,
    /// Where its page is, when it has one
    pub url: Option<String>,
}

/// The end of the log of one run that failed
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedLog {
    /// The check the run belongs to, by its workflow's name
    pub check: String,
    /// The run
    pub run: u64,
    /// Where the run's page is
    pub url: String,
    /// The end of its failed log, where it says why
    pub tail: String,
}

/// The CI of one project's repo
///
/// The CI holds its repo, so no method names one. A head is a commit's hash.
pub trait Ci: Send + Sync {
    /// Where CI stands on `head`
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the CI cannot be asked or its answer read.
    fn head_status(&self, head: &str) -> Result<HeadStatus, ForgeError>;

    /// Starts run `run` again, every job of it
    ///
    /// A run held in the queue has jobs that never started, which a rerun of only
    /// its failed jobs would leave out.
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the CI cannot be asked or refuses the rerun.
    fn rerun(&self, run: u64) -> Result<(), ForgeError>;

    /// The logs of the runs of `head` that failed, at most three, newest first
    ///
    /// With `merge_group`, the runs are those of the merge queue instead, which
    /// name no head.
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the CI cannot be asked or its answer read.
    fn logs(&self, head: &str, merge_group: bool) -> Result<Vec<FailedLog>, ForgeError>;
}
