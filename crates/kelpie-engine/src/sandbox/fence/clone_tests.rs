//! The fence's check of a clone, whose commits the checkout does not hold

use std::fs;

use super::differ;
use crate::worktree::clone;
use crate::worktree::tests::{Dir, git, process_tree};

#[test]
fn a_clone_with_a_commit_of_its_own_is_checked_against_main() {
    let dir = Dir::new("fence-clone");
    let root = dir.path().canonicalize().unwrap();
    let tree = process_tree(&root);
    let repo = root.join("repo");
    let worktree = root.join("wt");
    git(&root, &["init", "--quiet", "-b", "main", "repo"]);
    fs::create_dir_all(repo.join(".claude")).unwrap();
    fs::write(repo.join(".claude/settings.json"), "{}\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "settings"]);
    git(&repo, &["update-ref", "refs/remotes/origin/main", "main"]);
    clone::make(&tree, &repo, &worktree, "kelpie/7", "origin/main").unwrap();
    fs::write(worktree.join("work.txt"), "work\n").unwrap();
    git(&worktree, &["add", "work.txt"]);
    git(&worktree, &["commit", "--quiet", "-m", "work"]);

    let files = differ(&tree, &repo, &worktree, None, "main").unwrap();

    assert_eq!(files, Vec::<String>::new());
}
