//! A ref read from its loose file, which spares starting git where one is kept
//!
//! Git reads a ref's loose file before `packed-refs`, so where the file is,
//! it is the ref. Starting git costs tens of milliseconds on Windows, and the
//! board reads every ticket's branch on each write.

use std::path::{Component, Path};

use super::{WorktreeError, git, in_process};
use crate::tree::ProcessTree;

/// The commit the loose ref `full_ref` in the git dir `git_dir` names, none where git must answer
///
/// A ref that is packed, symbolic, in a reftable, behind a link or absent
/// has no such file. A branch or remote-tracking ref names a commit, so the
/// id is not peeled.
pub fn loose_commit(git_dir: &Path, full_ref: &str) -> Option<String> {
    let path = Path::new(full_ref);
    let plain =
        path.starts_with("refs") && path.components().all(|c| matches!(c, Component::Normal(_)));
    if !plain {
        return None;
    }
    full_id(&file_text(&git_dir.join(path))?)
}

/// The commit a worktree's `HEAD` in its git dir `own` names, through its
/// branch in the common dir `common`, none where git must answer
pub fn loose_head(own: &Path, common: &Path) -> Option<String> {
    let text = file_text(&own.join("HEAD"))?;
    match text.strip_prefix("ref: ") {
        Some(branch) => loose_commit(common, branch.strip_suffix('\n')?),
        None => full_id(&text),
    }
}

// The refs git keeps in each worktree's own git dir, so the common dir holds
// another worktree's answer for them.
const PER_WORKTREE: [&str; 3] = ["refs/bisect/", "refs/worktree/", "refs/rewritten/"];

// What the regular file at `path` holds, none for a link or a file that is not there.
fn file_text(path: &Path) -> Option<String> {
    if !std::fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

// The object id a ref file holds, on a line of its own.
fn full_id(text: &str) -> Option<String> {
    let id = text.strip_suffix('\n')?;
    let full =
        matches!(id.len(), 40 | 64) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    full.then(|| id.to_owned())
}

/// The commit a worktree's `full_ref` names in `repo`, from its loose file in
/// the common dir its branches and remote-tracking refs are kept in, none
/// where there is no such file, the ref is one git keeps in each worktree's
/// own git dir, or the common dir is one git must name
pub fn loose_commit_in(repo: &Path, full_ref: &str) -> Option<String> {
    if PER_WORKTREE.iter().any(|own| full_ref.starts_with(own)) {
        return None;
    }
    loose_commit(&in_process::common_dir(repo)?, full_ref)
}

/// The commit `full_ref` names in `repo`, from its loose file where it has one, else read in
/// process, else as git reads it
///
/// # Errors
///
/// [`WorktreeError`] when git cannot answer, as it cannot for an absent ref.
pub fn commit_of(tree: &ProcessTree, repo: &Path, full_ref: &str) -> Result<String, WorktreeError> {
    if let Some(commit) =
        loose_commit_in(repo, full_ref).or_else(|| in_process::commit_of(repo, full_ref))
    {
        return Ok(commit);
    }
    git(tree, repo, ["rev-parse", "--verify", "--quiet", full_ref])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::tests::{Dir, git, git_code, process_tree};

    fn repo() -> (Dir, String) {
        let dir = Dir::new("loose");
        git(dir.path(), &["init", "--quiet", "-b", "main"]);
        git(
            dir.path(),
            &["commit", "--quiet", "--allow-empty", "-m", "first"],
        );
        let head = git(dir.path(), &["rev-parse", "HEAD"]);
        (dir, head)
    }

    // A repo gix cannot read, so a ref read there has to come from git.
    fn sha256_repo() -> (Dir, String) {
        let dir = Dir::new("loose-sha256");
        git(
            dir.path(),
            &["init", "--quiet", "-b", "main", "--object-format=sha256"],
        );
        git(
            dir.path(),
            &["commit", "--quiet", "--allow-empty", "-m", "first"],
        );
        let head = git(dir.path(), &["rev-parse", "HEAD"]);
        (dir, head)
    }

    #[test]
    fn a_loose_branch_reads_as_git_reads_it() {
        let (dir, head) = repo();
        git(
            dir.path(),
            &["update-ref", "refs/remotes/origin/main", &head],
        );

        assert_eq!(
            loose_commit_in(dir.path(), "refs/heads/main"),
            Some(head.clone())
        );
        assert_eq!(
            loose_commit_in(dir.path(), "refs/remotes/origin/main"),
            Some(head)
        );
    }

    #[test]
    fn a_packed_symbolic_or_absent_ref_is_left_to_git() {
        let (dir, _) = repo();
        git(
            dir.path(),
            &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
        );
        git(dir.path(), &["pack-refs", "--all"]);

        assert_eq!(loose_commit_in(dir.path(), "refs/heads/main"), None);
        assert_eq!(loose_commit_in(dir.path(), "refs/heads/alias"), None);
        assert_eq!(loose_commit_in(dir.path(), "refs/heads/gone"), None);
    }

    #[test]
    fn a_ref_reads_loose_and_asks_git_for_a_packed_or_absent_one() {
        let (dir, head) = repo();
        git(dir.path(), &["branch", "packed"]);
        git(dir.path(), &["pack-refs", "--all"]);
        git(dir.path(), &["branch", "loose"]);
        let tree = process_tree(dir.path());

        assert_eq!(
            commit_of(&tree, dir.path(), "refs/heads/loose"),
            Ok(head.clone())
        );
        assert_eq!(commit_of(&tree, dir.path(), "refs/heads/packed"), Ok(head));
        let absent = commit_of(&tree, dir.path(), "refs/heads/gone").unwrap_err();
        assert!(
            matches!(&absent, WorktreeError::Git { args, .. } if args.ends_with("refs/heads/gone")),
            "{absent:?}"
        );
    }

    #[test]
    fn a_head_on_a_branch_or_detached_reads_as_git_reads_it() {
        let (dir, head) = repo();
        let git_dir = dir.path().join(".git");

        assert_eq!(loose_head(&git_dir, &git_dir), Some(head.clone()));
        git(dir.path(), &["checkout", "--quiet", "--detach"]);
        assert_eq!(loose_head(&git_dir, &git_dir), Some(head));
    }

    #[test]
    fn a_ref_reaching_out_of_the_git_dir_is_left_to_git() {
        let (dir, head) = repo();
        std::fs::write(dir.path().join(".git/outside"), format!("{head}\n")).unwrap();

        assert_eq!(loose_commit_in(dir.path(), "refs/../outside"), None);
        assert_eq!(loose_commit_in(dir.path(), "HEAD"), None);
    }

    #[test]
    fn a_ref_git_keeps_in_each_worktrees_own_dir_is_left_to_git() {
        let (dir, head) = repo();
        let linked = dir.path().join("linked");
        git(
            dir.path(),
            &[
                "worktree",
                "add",
                "--quiet",
                "--detach",
                linked.to_str().unwrap(),
            ],
        );
        git(dir.path(), &["update-ref", "refs/bisect/bad", &head]);

        // git reads the linked worktree's own ref, which it does not have.
        assert_eq!(
            git_code(
                &linked,
                &["rev-parse", "--verify", "--quiet", "refs/bisect/bad"]
            ),
            Some(1),
            "git answers for its own worktree"
        );
        for name in [
            "refs/bisect/bad",
            "refs/worktree/keep",
            "refs/rewritten/main",
        ] {
            assert_eq!(loose_commit_in(&linked, name), None, "{name}");
            assert_eq!(loose_commit_in(dir.path(), name), None, "{name}");
        }
        assert_eq!(loose_commit_in(dir.path(), "refs/heads/main"), Some(head));
    }

    #[test]
    fn a_git_dir_left_in_kelpies_own_environment_is_not_read() {
        let (dir, head) = sha256_repo();
        let (other, other_head) = repo();
        assert_ne!(head, other_head);

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "worktree::loose::tests::child_reads_a_ref_with_a_foreign_git_dir",
                "--nocapture",
            ])
            .env(CHILD_REPO, dir.path())
            .env("GIT_DIR", other.path().join(".git"))
            .env("GIT_WORK_TREE", other.path())
            .env("GIT_COMMON_DIR", other.path().join(".git"))
            .env("GIT_NAMESPACE", "ns")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{stdout}");
        assert!(stdout.contains(&format!("child read {head}")), "{stdout}");
        assert!(
            !stdout.contains(&other_head),
            "git read the other repo: {stdout}"
        );
    }

    // The repo a child reads with another repo's git dirs in its environment.
    const CHILD_REPO: &str = "KELPIE_LOOSE_CHILD_REPO";

    // Run by `a_git_dir_left_in_kelpies_own_environment_is_not_read` in a process
    // of its own, and a no-op anywhere else.
    #[test]
    fn child_reads_a_ref_with_a_foreign_git_dir() {
        let Some(repo) = std::env::var_os(CHILD_REPO) else {
            return;
        };
        let repo = Path::new(&repo);
        let tree = process_tree(repo);
        let read = crate::worktree::commit_of(&tree, repo, "refs/heads/main");
        println!("child read {}", read.expect("git answers the ref"));
    }
}
