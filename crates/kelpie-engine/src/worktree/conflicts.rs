//! Whether a head merges into the base without conflicts

use std::path::Path;

use super::{WorktreeError, in_repo};
use crate::tree::ProcessTree;

/// Whether merging `head` into `origin/<base>` has conflicts, which git says
/// without touching the checkout. [`super::base_of`] has just fetched both.
///
/// # Errors
///
/// [`WorktreeError`] when git could not judge: it failed to run or to merge,
/// or said nothing of `head`.
pub fn merge_conflicts(
    tree: &ProcessTree,
    repo: &Path,
    head: &str,
    base: &str,
) -> Result<bool, WorktreeError> {
    let tip = format!("origin/{base}");
    let args = ["merge-tree", "--write-tree", "--no-messages", &tip, head];
    let output = tree
        .output(in_repo(repo).arg("--no-replace-objects").args(args))
        .map_err(|e| WorktreeError::Spawn(e.to_string()))?;
    let failed = |why: String| WorktreeError::Git {
        args: args.join(" "),
        stderr: why,
    };
    // Exit 1 with a tree first is a merge with conflicts. A missing commit
    // also exits 1, with nothing on stdout, and git before 2.38 fails otherwise.
    match output.status.code() {
        Some(0) => Ok(false),
        Some(1) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let tree = stdout.lines().next().unwrap_or_default();
            if tree.is_empty() || !tree.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(failed(format!("git said nothing of {head}")));
            }
            Ok(true)
        }
        _ => Err(failed(String::from_utf8_lossy(&output.stderr).into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::tests::{Dir, git, process_tree, write_in};

    // A repo whose `origin/main` holds `shared.txt`, as just fetched, and whose
    // `branch` sets it to `text` on top of that
    fn repo_with_branch(dir: &Dir, text: &str) -> (std::path::PathBuf, String) {
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet", "-b", "main"]);
        write_in(&repo, "shared.txt", "base\n");
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "first"]);
        git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(&repo, &["checkout", "--quiet", "-b", "work"]);
        write_in(
            &repo,
            if text.is_empty() {
                "other.txt"
            } else {
                "shared.txt"
            },
            &format!("{text}\n"),
        );
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "work"]);
        let work = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["checkout", "--quiet", "main"]);
        (repo, work)
    }

    // Main moves on and `origin/main` follows.
    fn main_sets_shared(repo: &Path, text: &str) {
        write_in(repo, "shared.txt", &format!("{text}\n"));
        git(repo, &["add", "."]);
        git(repo, &["commit", "--quiet", "-m", "main moves"]);
        git(repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    }

    #[test]
    fn a_head_that_merges_cleanly_has_no_conflicts() {
        let dir = Dir::new("merges-clean");
        let (repo, work) = repo_with_branch(&dir, "");
        main_sets_shared(&repo, "theirs");

        assert_eq!(
            merge_conflicts(&process_tree(dir.path()), &repo, &work, "main"),
            Ok(false)
        );
    }

    #[test]
    fn a_head_that_conflicts_with_the_base_says_so() {
        let dir = Dir::new("merges-conflict");
        let (repo, work) = repo_with_branch(&dir, "mine");
        main_sets_shared(&repo, "theirs");

        assert_eq!(
            merge_conflicts(&process_tree(dir.path()), &repo, &work, "main"),
            Ok(true)
        );
    }

    #[test]
    fn a_head_git_does_not_know_is_an_error() {
        let dir = Dir::new("merges-unknown");
        let (repo, _) = repo_with_branch(&dir, "mine");

        let judged = merge_conflicts(&process_tree(dir.path()), &repo, &"0".repeat(40), "main");

        assert!(judged.is_err(), "{judged:?}");
    }
}
