//! The remote a ticket's branch lives on
//!
//! A branch kelpie opened lives on `origin`. A contributor's fork is the
//! second kind in SK, and it arrives with the repos the maintainer does not own.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{WorktreeError, git};
use crate::tree::ProcessTree;

/// Where a ticket's branch is pushed, and fetched from
// wire format: a ticket saves it, so changing this is a breaking change to
// the state file. A fork variant is added later without one, as the enum is
// non-exhaustive.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Remote {
    /// The project's `origin`, under the ticket's own branch name
    #[default]
    Origin,
}

impl Remote {
    /// Whether this is the project's `origin`
    pub fn is_origin(&self) -> bool {
        *self == Self::Origin
    }

    /// The ref the remote's branch is fetched to
    pub(super) fn tracking(&self, branch: &str) -> String {
        match self {
            Self::Origin => format!("refs/remotes/origin/{branch}"),
        }
    }

    // Where git is told to fetch from and push to.
    fn target(&self) -> &str {
        match self {
            Self::Origin => "origin",
        }
    }

    // The branch's full name on this remote, `branch` being the ticket's.
    fn branch_ref(&self, branch: &str) -> String {
        match self {
            Self::Origin => format!("refs/heads/{branch}"),
        }
    }

    /// Deletes `branch` here when it is here
    ///
    /// The forge may delete a merged branch itself between the check below and
    /// this push, and git's error for that varies, so a failed delete is judged
    /// by whether the ref is still there. The repo's own pre-push hook has no
    /// say over a branch kelpie deletes.
    pub(super) fn delete(
        &self,
        tree: &ProcessTree,
        repo: &Path,
        branch: &str,
    ) -> Result<(), WorktreeError> {
        let full_ref = self.branch_ref(branch);
        if !self.has_branch(tree, repo, &full_ref)? {
            return Ok(());
        }
        let target = self.target();
        match git(
            tree,
            repo,
            [
                "push",
                "--quiet",
                "--no-verify",
                "--delete",
                "--",
                target,
                &full_ref,
            ],
        ) {
            Err(e) if self.has_branch(tree, repo, &full_ref)? => Err(e),
            _ => Ok(()),
        }
    }

    // Whether `full_ref` is a branch here right now, asked of the remote.
    fn has_branch(
        &self,
        tree: &ProcessTree,
        repo: &Path,
        full_ref: &str,
    ) -> Result<bool, WorktreeError> {
        let target = self.target();
        let asked = git(tree, repo, ["ls-remote", "--heads", "--", target, full_ref])?;
        Ok(!asked.is_empty())
    }

    /// Fetches the branch, and returns the ref it was fetched to
    pub(super) fn fetch(
        &self,
        tree: &ProcessTree,
        repo: &Path,
        branch: &str,
    ) -> Result<String, WorktreeError> {
        let tracking = self.tracking(branch);
        let target = self.target();
        match self {
            Self::Origin => git(tree, repo, ["fetch", "--quiet", "--", target, branch])?,
        };
        Ok(tracking)
    }

    /// Fetches `origin/main` and the branch together, and returns the branch's ref
    pub(super) fn fetch_with_main(
        &self,
        tree: &ProcessTree,
        repo: &Path,
        branch: &str,
        base: &str,
    ) -> Result<String, WorktreeError> {
        let target = self.target();
        match self {
            Self::Origin => {
                git(tree, repo, ["fetch", "--quiet", "--", target, base, branch])?;
                Ok(self.tracking(branch))
            }
        }
    }
}

#[cfg(test)]
mod tests;
