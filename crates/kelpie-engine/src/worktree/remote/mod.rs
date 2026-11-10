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

    /// Fetches the branch, and returns the ref it was fetched to
    pub(super) fn fetch(
        &self,
        tree: &ProcessTree,
        repo: &Path,
        branch: &str,
    ) -> Result<String, WorktreeError> {
        let tracking = self.tracking(branch);
        match self {
            Self::Origin => git(tree, repo, ["fetch", "--quiet", "--", "origin", branch])?,
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
        match self {
            Self::Origin => {
                git(
                    tree,
                    repo,
                    ["fetch", "--quiet", "--", "origin", base, branch],
                )?;
                Ok(self.tracking(branch))
            }
        }
    }
}

#[cfg(test)]
mod tests;
