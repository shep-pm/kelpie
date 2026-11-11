//! The end of a worker's turn: the fence's check of its branch
//!
//! The sandbox's `denyWrite` globs are set when a turn starts, so they miss what
//! the turn makes on its own, and on Linux they are skipped whole. The fence reads
//! the worktree when the turn ends and catches both
//! (`docs/design/workers.md`, The worker's policy, check 3).
//!
//! Nothing here lets a worktree find its own git dirs: the clone's are named
//! ([`worktree::trusted`]), since the `.git` file in the worktree is the worker's
//! to rewrite, and a git that reads it answers from whatever repo the worker points
//! it at.

use std::path::Path;

use crate::sandbox::fence;
use crate::tree::ProcessTree;
use crate::worktree::{self, WorktreeError};

use super::Status;

/// What the fence makes of a turn's branch
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ending {
    /// The branch changed nothing a harness owns: the worker is done
    Done,

    /// The branch holds something only the maintainer can rule on
    Held(Held),
}

impl Ending {
    /// What the ticket's record becomes
    ///
    /// A held branch is [`Status::WaitingOnMaintainer`], which needs no slot, so
    /// holding one frees it.
    #[must_use]
    pub fn status(&self) -> Status {
        match self {
            Self::Done => Status::Ended,
            Self::Held(_) => Status::WaitingOnMaintainer,
        }
    }

    /// Why the ticket waits on the maintainer: the held branch's line, `None` when done
    #[must_use]
    pub fn reason(&self) -> Option<String> {
        match self {
            Self::Done => None,
            Self::Held(held) => Some(held.line()),
        }
    }
}

/// Why a turn's branch holds its ticket for the maintainer
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Held {
    /// The branch changed each of these files, which a harness loads and may run code from
    OwnFiles {
        /// Each file from the worktree's root, as the fence names it
        files: Vec<String>,
    },

    /// The branch carries harness files as `main` left them where it was cut, not as
    /// `main` holds them now, which is what a branch that lagged a change `main` made
    /// looks like
    BehindMain,

    /// The branch could not be read, so nothing is believed about it
    Unreadable(WorktreeError),
}

impl Held {
    /// One line saying what the branch holds, naming each file and the harness that owns it
    #[must_use]
    pub fn line(&self) -> String {
        match self {
            Self::OwnFiles { files } => {
                let named = files
                    .iter()
                    .map(|file| match fence::owner(Path::new(file)) {
                        Some(harness) => format!("{file} ({harness})"),
                        None => file.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("the branch changes a harness's own files: {named}")
            }
            Self::BehindMain => {
                "the branch carries harness files as main left them where it was cut, \
                 not as main holds them now"
                    .to_owned()
            }
            Self::Unreadable(error) => format!("kelpie could not read the branch: {error}"),
        }
    }
}

/// Check a turn's branch with the fence
///
/// [`fence::differ`] names each harness's own file the worktree changed from a
/// commit kelpie trusts: `origin/<base>`, the commit the branch cut it at, or a head
/// the maintainer accepted. Where the clone has commits of its own,
/// [`fence::matches_main`] decides as well: the branch is kelpie's to push, and the
/// next turn's harness loads these files again, so they must be `origin/<base>`'s own
/// and not the ones it was cut at. A branch that kept `main`'s older files cannot be
/// told from one that restored them, so it is held too.
///
/// A branch kelpie cannot read is held as [`Held::Unreadable`], never passed.
pub fn check(tree: &ProcessTree, repo: &Path, worktree: &Path, base: &str) -> Ending {
    match held_for(tree, repo, worktree, base) {
        Ok(Some(held)) => Ending::Held(held),
        Ok(None) => Ending::Done,
        Err(error) => Ending::Held(Held::Unreadable(error)),
    }
}

/// What the fence holds of `worktree`'s branch, and `None` when it passes
fn held_for(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    base: &str,
) -> Result<Option<Held>, WorktreeError> {
    let files = fence::differ(tree, repo, worktree, None, base)?;
    if !files.is_empty() {
        return Ok(Some(Held::OwnFiles { files }));
    }
    if has_commits_of_its_own(tree, repo, worktree, base)?
        && !fence::matches_main(tree, repo, worktree, base)
    {
        return Ok(Some(Held::BehindMain));
    }
    Ok(None)
}

/// Whether `worktree`'s clone holds commits `origin/<base>` lacks
///
/// Its head is its own when it is not part of `origin/<base>`'s history, which is
/// asked of the worktree's git with the clone's dirs named. `origin/<base>` is read
/// in `repo`: a clone's own is the worker's to move.
fn has_commits_of_its_own(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    base: &str,
) -> Result<bool, WorktreeError> {
    let main = worktree::commit_of(tree, repo, &format!("refs/remotes/origin/{base}"))?;
    let head = worktree::head(tree, repo, worktree)?;
    let cut = worktree::trusted(tree, repo, worktree)?(&["merge-base", main.as_str(), "HEAD"])?;
    Ok(cut != head)
}
