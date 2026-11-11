use std::path::PathBuf;

use super::tests::{Dir, git, process_tree, write_in};
use super::{
    Rebase, Remote, Start, WorktreeError, abort_merge, clone, head, prepare, rebase, uncommitted,
};
use crate::tree::ProcessTree;

// An origin holding `main` with `shared.txt`, a checkout of it, and a ticket's
// worktree on `kelpie/7` cut from it. A worker's commit is made in the
// worktree and pushed to origin, and main moves in a second clone, as they do
// on a project.
struct Rig {
    _dir: Dir,
    tree: ProcessTree,
    root: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let dir = Dir::new("catch-up");
        let root = dir.path().canonicalize().unwrap();
        git(
            &root,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&root, &["clone", "--quiet", "origin.git", "repo"]);
        let rig = Self {
            tree: process_tree(&root),
            _dir: dir,
            root,
        };
        write_in(&rig.repo(), "shared.txt", "base\n");
        git(&rig.repo(), &["add", "."]);
        git(&rig.repo(), &["commit", "--quiet", "-m", "init"]);
        git(&rig.repo(), &["push", "--quiet", "origin", "main"]);
        prepare(
            &rig.tree,
            &rig.repo(),
            &rig.worktree(),
            "kelpie/7",
            Start::Main,
            "main",
            &Remote::Origin,
            &rig.root.join("build"),
        )
        .unwrap();
        rig
    }

    fn repo(&self) -> PathBuf {
        self.root.join("repo")
    }

    fn worktree(&self) -> PathBuf {
        self.root.join("wt")
    }

    fn in_worktree(&self, args: &[&str]) -> String {
        git(&self.worktree(), args)
    }

    // The worker commits `text` at `file`, and the branch is pushed
    fn work(&self, file: &str, text: &str) -> String {
        write_in(&self.worktree(), file, text);
        self.in_worktree(&["add", file]);
        self.in_worktree(&["commit", "--quiet", "-m", &format!("work on {file}")]);
        self.push()
    }

    fn push(&self) -> String {
        self.in_worktree(&["push", "--quiet", "origin", "HEAD:refs/heads/kelpie/7"]);
        self.in_worktree(&["rev-parse", "HEAD"])
    }

    // Main takes a commit that sets `text` at `file`, and the checkout fetches it
    fn main_moves(&self, file: &str, text: &str) {
        let other = self.root.join("other");
        if !other.exists() {
            git(&self.root, &["clone", "--quiet", "origin.git", "other"]);
        }
        git(&other, &["pull", "--quiet", "origin", "main"]);
        write_in(&other, file, text);
        git(&other, &["add", file]);
        git(
            &other,
            &["commit", "--quiet", "-m", &format!("main sets {file}")],
        );
        git(&other, &["push", "--quiet", "origin", "main"]);
        git(&self.repo(), &["fetch", "--quiet", "origin"]);
    }

    fn origin_main(&self) -> String {
        self.in_origin("main")
    }

    fn in_origin(&self, branch: &str) -> String {
        git(&self.root.join("origin.git"), &["rev-parse", branch])
    }

    fn rebase(&self, head: &str, rewrite: bool) -> Result<Rebase, WorktreeError> {
        rebase(
            &self.tree,
            &self.repo(),
            &self.worktree(),
            "kelpie/7",
            head,
            rewrite,
            &Remote::Origin,
            "main",
        )
    }

    fn pushed(&self, head: &str) -> String {
        match self.rebase(head, true).unwrap() {
            Rebase::Pushed(caught) => caught,
            other => panic!("expected a push, got {other:?}"),
        }
    }

    // The commit `at` names, as the worktree's clone holds it
    fn parents_of(&self, at: &str) -> Vec<String> {
        let line = self.in_worktree(&["rev-list", "--parents", "-n", "1", at]);
        line.split(' ').skip(1).map(str::to_owned).collect()
    }
}

#[test]
fn a_branch_behind_its_base_is_rebased_and_pushed_with_a_lease() {
    let rig = Rig::new();
    let work = rig.work("work.txt", "mine\n");
    rig.main_moves("main.txt", "main\n");

    let caught = rig.pushed(&work);

    assert_eq!(rig.parents_of(&caught), vec![rig.origin_main()]);
    assert_eq!(rig.in_origin("kelpie/7"), caught);
    assert_eq!(
        head(&rig.tree, &rig.repo(), &rig.worktree()).unwrap(),
        caught
    );
    assert!(
        uncommitted(&rig.tree, &rig.repo(), &rig.worktree())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_rebase_never_overwrites_a_branch_that_moved_on_the_remote() {
    let rig = Rig::new();
    let work = rig.work("work.txt", "mine\n");
    rig.main_moves("main.txt", "main\n");
    // Someone else pushes to the branch after the worker's head was read.
    let other = rig.root.join("other");
    git(&other, &["fetch", "--quiet", "origin"]);
    git(
        &other,
        &["checkout", "--quiet", "-b", "kelpie/7", "origin/kelpie/7"],
    );
    write_in(&other, "theirs.txt", "theirs\n");
    git(&other, &["add", "theirs.txt"]);
    git(&other, &["commit", "--quiet", "-m", "someone else"]);
    git(&other, &["push", "--quiet", "origin", "kelpie/7"]);
    let theirs = rig.in_origin("kelpie/7");

    let rebased = rig.rebase(&work, true);

    assert!(rebased.is_err(), "{rebased:?}");
    assert_eq!(rig.in_origin("kelpie/7"), theirs);
    assert_eq!(head(&rig.tree, &rig.repo(), &rig.worktree()).unwrap(), work);
}

#[test]
fn a_branch_holding_a_merge_commit_is_merged_never_rebased() {
    let rig = Rig::new();
    let first = rig.work("work.txt", "mine\n");
    rig.in_worktree(&["checkout", "--quiet", "-b", "side", &format!("{first}^")]);
    write_in(&rig.worktree(), "side.txt", "side\n");
    rig.in_worktree(&["add", "side.txt"]);
    rig.in_worktree(&["commit", "--quiet", "-m", "side"]);
    rig.in_worktree(&["checkout", "--quiet", "kelpie/7"]);
    rig.in_worktree(&["merge", "--quiet", "--no-ff", "-m", "merge side", "side"]);
    let work = rig.push();
    rig.main_moves("main.txt", "main\n");

    let caught = rig.pushed(&work);

    // The head stays in the history, which a rebase would have rewritten.
    assert_eq!(rig.parents_of(&caught), vec![work, rig.origin_main()]);
    assert_eq!(rig.in_origin("kelpie/7"), caught);
}

// Two commits that undo each other leave the branch with `shared.txt` as it
// was on main, so a merge takes main's change where replaying the first
// commit conflicts with it.
#[test]
fn a_rebase_that_conflicts_where_a_merge_would_not_is_merged_and_pushed() {
    let rig = Rig::new();
    rig.work("shared.txt", "mine\n");
    let work = rig.work("shared.txt", "base\n");
    rig.main_moves("shared.txt", "main\n");

    let caught = rig.pushed(&work);

    assert_eq!(rig.parents_of(&caught), vec![work, rig.origin_main()]);
    assert_eq!(
        rig.in_worktree(&["show", &format!("{caught}:shared.txt")]),
        "main"
    );
    assert_eq!(rig.in_origin("kelpie/7"), caught);
    // The rebase that conflicted is not left in progress.
    assert!(
        !clone::git_dir_of(&rig.worktree())
            .join("rebase-merge")
            .exists()
    );
    assert!(
        !clone::git_dir_of(&rig.worktree())
            .join("rebase-apply")
            .exists()
    );
}

#[test]
fn a_merge_that_conflicts_is_left_in_progress_naming_the_files() {
    let rig = Rig::new();
    let work = rig.work("shared.txt", "mine\n");
    rig.main_moves("shared.txt", "theirs\n");
    rig.main_moves("other.txt", "other\n");
    let main = rig.origin_main();

    let rebased = rig.rebase(&work, true).unwrap();

    let Rebase::Conflicts { base, files } = rebased else {
        panic!("expected conflicts, got {rebased:?}");
    };
    assert_eq!(base, main);
    assert_eq!(files, vec!["shared.txt".to_owned()]);
    assert_eq!(rig.in_worktree(&["rev-parse", "MERGE_HEAD"]), main);
    // The base's other files are written and staged for the worker.
    assert_eq!(
        rig.in_worktree(&["diff", "--cached", "--name-only", "--diff-filter=A"]),
        "other.txt"
    );
    assert_eq!(rig.in_origin("kelpie/7"), work);
}

#[test]
fn a_push_the_remote_refuses_puts_the_branch_back_at_its_head() {
    let rig = Rig::new();
    let work = rig.work("work.txt", "mine\n");
    rig.main_moves("main.txt", "main\n");
    let hook = rig.root.join("origin.git/hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho refused >&2\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let rebased = rig.rebase(&work, true);

    assert!(
        matches!(rebased, Err(WorktreeError::Git { .. })),
        "{rebased:?}"
    );
    assert_eq!(head(&rig.tree, &rig.repo(), &rig.worktree()).unwrap(), work);
    assert_eq!(rig.in_origin("kelpie/7"), work);
    assert!(
        uncommitted(&rig.tree, &rig.repo(), &rig.worktree())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_worktree_with_uncommitted_changes_is_refused() {
    let rig = Rig::new();
    let work = rig.work("work.txt", "mine\n");
    rig.main_moves("main.txt", "main\n");
    write_in(&rig.worktree(), "work.txt", "edited\n");

    let rebased = rig.rebase(&work, true).unwrap();

    assert_eq!(
        rebased,
        Rebase::Refused("its worktree has changes that are not committed".into())
    );
    assert_eq!(head(&rig.tree, &rig.repo(), &rig.worktree()).unwrap(), work);
    assert_eq!(rig.in_origin("kelpie/7"), work);
}

#[test]
fn a_worktree_not_at_the_head_or_off_its_branch_is_refused() {
    let rig = Rig::new();
    let work = rig.work("work.txt", "mine\n");
    let behind = rig.parents_of(&work).remove(0);

    let not_at_head = rig.rebase(&behind, true).unwrap();
    rig.in_worktree(&["checkout", "--quiet", "--detach"]);
    let off_branch = rig.rebase(&work, true).unwrap();

    for refused in [not_at_head, off_branch] {
        let Rebase::Refused(why) = refused else {
            panic!("expected a refusal, got {refused:?}");
        };
        assert!(why.contains("is not at the pull request's head"), "{why}");
    }
}

#[test]
fn a_worktree_mid_merge_is_refused() {
    let rig = Rig::new();
    let work = rig.work("shared.txt", "mine\n");
    rig.main_moves("shared.txt", "theirs\n");
    assert!(matches!(
        rig.rebase(&work, true).unwrap(),
        Rebase::Conflicts { .. }
    ));

    let again = rig.rebase(&work, true).unwrap();

    assert_eq!(
        again,
        Rebase::Refused("its worktree is in the middle of a merge".into())
    );
}

// The worker's one identity is on its commits. Kelpie has none of its own to
// put on a rebased commit, and a merge commit takes the same.
#[test]
fn the_caught_up_commits_take_the_heads_committer() {
    let rig = Rig::new();
    write_in(&rig.worktree(), "work.txt", "mine\n");
    rig.in_worktree(&["add", "work.txt"]);
    rig.in_worktree(&[
        "-c",
        "user.name=Worker",
        "-c",
        "user.email=worker@example.invalid",
        "commit",
        "--quiet",
        "-m",
        "work",
    ]);
    let work = rig.push();
    rig.main_moves("main.txt", "main\n");

    let caught = rig.pushed(&work);

    let committer = rig.in_worktree(&["log", "-1", "--format=%cn <%ce>", &caught]);
    assert_eq!(committer, "Worker <worker@example.invalid>");
}

#[test]
fn abort_merge_leaves_the_worktree_at_its_head() {
    let rig = Rig::new();
    let work = rig.work("shared.txt", "mine\n");
    rig.main_moves("shared.txt", "theirs\n");
    assert!(matches!(
        rig.rebase(&work, true).unwrap(),
        Rebase::Conflicts { .. }
    ));

    abort_merge(&rig.tree, &rig.repo(), &rig.worktree()).unwrap();

    assert_eq!(head(&rig.tree, &rig.repo(), &rig.worktree()).unwrap(), work);
    assert!(
        uncommitted(&rig.tree, &rig.repo(), &rig.worktree())
            .unwrap()
            .is_empty()
    );
    assert!(
        !clone::git_dir_of(&rig.worktree())
            .join("MERGE_HEAD")
            .exists()
    );
}
