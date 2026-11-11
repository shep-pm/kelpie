//! The pull request a worker's hand-over becomes

use std::num::NonZeroU64;

use super::ForgeError;

/// The pull request of a project's repo, opened and edited as the maintainer
///
/// The worker pushes its branch with a token that cannot open a pull request,
/// so kelpie opens it from the worker's hand-over file
/// (`docs/design/tickets.md`, GitHub). The source holds its repo, so no method
/// names one. A title or body that names this machine is refused with
/// [`ForgeError::LocalPath`] before any call is made.
pub trait Pulls: Send + Sync {
    /// Opens a draft pull request of `head` into `base`, and answers its number
    ///
    /// The pull request is always a draft: kelpie marks nothing ready.
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
}
