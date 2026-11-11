//! The CI of a GitHub repo: GitHub Actions, read and driven through `gh`
//!
//! [`GithubActions`] is the [`Ci`] a project's repo has when its forge is
//! GitHub. Every call goes through the [`Hold`], so a used-up rate limit holds
//! its reads like any other (ADR 0001: `gh` is a child of the process tree).
//! What belongs to GitHub Actions alone sits here and not behind the port: the
//! rollup and suites in `checks`, and the log tails and the rerun in `runs`.

use super::hold::Hold;
use super::{checks, runs};
use crate::ci::{Ci, FailedLog, HeadStatus};
use crate::github::ForgeSlug;
use crate::ticket::ForgeError;

/// One GitHub repo's Actions, as the CI port reads them
#[derive(Debug)]
pub struct GithubActions {
    hold: Hold,
    repo: ForgeSlug,
}

impl GithubActions {
    /// The Actions of `repo`, asked through `hold`
    #[must_use]
    pub fn new(hold: Hold, repo: ForgeSlug) -> Self {
        Self { hold, repo }
    }
}

impl Ci for GithubActions {
    fn head_status(&self, head: &str) -> Result<HeadStatus, ForgeError> {
        checks::head_status(&self.hold, &self.repo, head)
    }

    fn rerun(&self, run: u64) -> Result<(), ForgeError> {
        runs::rerun(&self.hold, &self.repo, run)
    }

    fn logs(&self, head: &str, merge_group: bool) -> Result<Vec<FailedLog>, ForgeError> {
        runs::failed_logs(&self.hold, &self.repo, head, merge_group)
    }
}
