//! The repo a checkout's `origin` names

use std::path::Path;
use std::process::Command;

use crate::tree::ProcessTree;
use crate::worktree;

use super::{ForgeSlug, forge_of};

/// The project's repo as GitHub sees it, from the `origin` of its checkout,
/// which is what each ticket's clone pushes to
///
/// Read in process where gix can say, as [`worktree::clone`] reads the same key
/// when it copies it, and asked of git where that read cannot answer. A checkout
/// with no `origin`, or one that is not GitHub, names no repo.
#[must_use]
pub fn origin_of(tree: &ProcessTree, folder: &Path) -> Option<ForgeSlug> {
    const URL: &str = "remote.origin.url";
    let url = worktree::local_config(folder, |key| key == URL)
        .and_then(|found| {
            found
                .into_iter()
                .find(|(key, _)| key == URL)
                .map(|(_, url)| url)
        })
        .or_else(|| {
            let mut git = Command::new("git");
            git.current_dir(folder).args(["config", "--get", URL]);
            let asked = tree.output(&mut git).ok()?;
            asked
                .status
                .success()
                .then(|| String::from_utf8_lossy(&asked.stdout).trim().to_owned())
        })?;
    forge_of(&url)
}
