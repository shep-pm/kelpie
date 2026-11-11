//! The files a branch changes from the base
//!
//! As SK's `changed_files` (`worktree/changed.rs`). Its `moved_since` stays
//! behind with review.

use std::path::Path;

use super::{WorktreeError, git};
use crate::tree::ProcessTree;

/// The files `head` changes from `origin/<base>`, as the pull request shows them
///
/// Three dots, so these are the branch's own changes from the commit it left
/// the base at, and not what the base gained since. [`super::base_of`] has just
/// fetched `origin/<base>`.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn changed_files(
    tree: &ProcessTree,
    repo: &Path,
    head: &str,
    base: &str,
) -> Result<Vec<String>, WorktreeError> {
    let range = format!("origin/{base}...{head}");
    let names = git(
        tree,
        repo,
        ["diff", "--name-only", "--no-renames", "-z", &range],
    )?;
    Ok(names
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::tests::{Dir, git, process_tree, write_in};

    #[test]
    fn the_changed_files_are_those_between_the_base_and_the_head() {
        let dir = Dir::new("changed");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet", "-b", "main"]);
        write_in(&repo, "base.rs", "fn base() {}\n");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "first"]);
        git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(&repo, &["checkout", "--quiet", "-b", "work"]);
        write_in(&repo, "a.rs", "1\n");
        write_in(&repo, "base.rs", "fn base() { 2 }\n");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "work"]);
        let work = git(&repo, &["rev-parse", "HEAD"]);
        // The base moves on after the branch left it, which is not the branch's change
        git(&repo, &["checkout", "--quiet", "main"]);
        write_in(&repo, "main.rs", "moved\n");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "main moves"]);
        git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);

        let files = changed_files(&process_tree(dir.path()), &repo, &work, "main").unwrap();

        assert_eq!(files, ["a.rs", "base.rs"]);
    }
}
