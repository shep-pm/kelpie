//! The pull request a worker's hand-over becomes

use std::num::NonZeroU64;

use super::ForgeError;

/// Where a pull request stands on the forge
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullState {
    /// Still open, as a draft or ready for review
    Open,
    /// Merged into its base branch
    Merged,
    /// Closed without a merge
    Closed,
}

/// The pull request of a project's repo, opened, edited and merged as the maintainer
///
/// The worker pushes its branch with a token that cannot open a pull request,
/// so kelpie opens it from the worker's hand-over file
/// (`docs/design/tickets.md`, GitHub). The source holds its repo, so no method
/// names one. A title or body that names this machine is refused with
/// [`ForgeError::LocalPath`] before any call is made.
pub trait Pulls: Send + Sync {
    /// Opens a draft pull request of `head` into `base`, and answers its number
    ///
    /// The pull request is always a draft until [`Pulls::mark_ready`].
    ///
    /// # Errors
    ///
    /// [`ForgeError::LocalPath`] when `title` or `body` names this machine, and
    /// otherwise [`ForgeError`] when the forge cannot be asked or its answer
    /// read.
    fn create_draft(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<NonZeroU64, ForgeError>;

    /// The commit `branch` has on the forge, or `None` when it has no such branch
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the forge cannot be asked or its answer read.
    fn branch_head(&self, branch: &str) -> Result<Option<String>, ForgeError>;

    /// Changes the title and body of pull request `number`, and nothing else
    ///
    /// # Errors
    ///
    /// As [`Pulls::create_draft`].
    fn edit(&self, number: NonZeroU64, title: &str, body: &str) -> Result<(), ForgeError>;

    /// Where pull request `number` stands
    ///
    /// The gate reads it for a pull request gone from the open ones.
    ///
    /// # Errors
    ///
    /// [`ForgeError::Unreadable`] when the forge names a state this does not,
    /// and otherwise as [`Pulls::create_draft`].
    fn state(&self, number: NonZeroU64) -> Result<PullState, ForgeError>;

    /// Takes pull request `number` out of draft
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the forge refuses or cannot be asked.
    fn mark_ready(&self, number: NonZeroU64) -> Result<(), ForgeError>;

    /// Merges pull request `number` with a merge commit, only while its head is `head`
    ///
    /// The merge commit's subject is the pull request's title, and a head that
    /// has moved since is refused. The call goes through `gh pr merge`, since the
    /// REST merge does not join a merge queue.
    ///
    /// # Errors
    ///
    /// [`ForgeError::Failed`] carrying the forge's words when it refuses, and
    /// otherwise as [`Pulls::create_draft`].
    fn merge(&self, number: NonZeroU64, head: &str) -> Result<(), ForgeError>;

    /// Disarms the auto-merge of pull request `number`
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when the forge refuses or cannot be asked.
    fn disable_auto_merge(&self, number: NonZeroU64) -> Result<(), ForgeError>;
}
