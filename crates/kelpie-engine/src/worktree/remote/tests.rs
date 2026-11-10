use std::path::{Path, PathBuf};

use super::Remote;
use crate::tree::ProcessTree;
use crate::worktree::tests::{Dir, git, process_tree};
use crate::worktree::{
    Base, Start, Worktree, WorktreeError, base_of, change_start, clone, head, head_with_main,
    holder_of, holds_work, origin_head, prepare, remove,
};

// An origin holding `main`, a clone of it, and a second bare repository whose
// `contrib` branch holds a commit origin lacks.
struct Rig {
    _dir: Dir,
    tree: ProcessTree,
    root: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let dir = Dir::new("remote");
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
        let repo = rig.repo();
        git(&repo, &["commit", "--quiet", "--allow-empty", "-m", "init"]);
        git(&repo, &["push", "--quiet", "origin", "main"]);
        git(&repo, &["checkout", "--quiet", "-b", "contrib"]);
        git(&repo, &["commit", "--quiet", "--allow-empty", "-m", "work"]);
        git(&repo, &["checkout", "--quiet", "main"]);
        rig
    }

    fn repo(&self) -> PathBuf {
        self.root.join("repo")
    }

    fn worktree(&self) -> PathBuf {
        self.root.join("wt")
    }

    fn build(&self) -> PathBuf {
        self.root.join("build")
    }

    fn prepare(&self, branch: &str, start: Start, remote: &Remote) {
        self.prepared(branch, start, remote);
    }

    fn rev(&self, repo: &str, reference: &str) -> String {
        git(&self.root.join(repo), &["rev-parse", reference])
    }

    fn at_worktree(&self, args: &[&str]) -> String {
        git(&self.worktree(), args)
    }

    // A ref as the worktree's clone holds it
    fn in_clone(&self, reference: &str) -> String {
        git(
            &clone::git_dir_of(&self.worktree()),
            &["rev-parse", reference],
        )
    }

    // What the checkout holds under `refs/heads/kelpie/`
    fn checkout_branches(&self) -> String {
        git(&self.repo(), &["for-each-ref", "refs/heads/kelpie"])
    }

    // Take `main` off origin, which is what a fetch of it fails against
    fn drop_main(&self) {
        git(
            &self.root.join("origin.git"),
            &["update-ref", "-d", "refs/heads/main"],
        );
    }

    fn prepared(&self, branch: &str, start: Start, remote: &Remote) -> Worktree {
        prepare(
            &self.tree,
            &self.repo(),
            &self.worktree(),
            branch,
            start,
            "main",
            remote,
            &self.build(),
        )
        .unwrap()
    }
}

#[test]
fn prepare_on_origin_starts_and_reuses_a_worktree_as_before() {
    let rig = Rig::new();
    let main = rig.rev("origin.git", "main");

    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    assert_eq!(rig.at_worktree(&["rev-parse", "HEAD"]), main);
    assert_eq!(rig.in_clone("refs/heads/kelpie/7"), main);
    assert_eq!(
        rig.at_worktree(&["symbolic-ref", "HEAD"]),
        "refs/heads/kelpie/7"
    );
    assert_eq!(rig.checkout_branches(), "");
    assert!(rig.build().is_dir());

    rig.at_worktree(&["commit", "--quiet", "--allow-empty", "-m", "work"]);
    let worked = rig.at_worktree(&["rev-parse", "HEAD"]);
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    assert_eq!(rig.at_worktree(&["rev-parse", "HEAD"]), worked);
    assert_eq!(rig.in_clone("refs/heads/kelpie/7"), worked);
    assert_eq!(rig.checkout_branches(), "");
}

// A worker is told to merge `origin/main` and to run no fetch of its own, so
// kelpie brings main to its worktree at the start of each of its turns.
#[test]
fn prepare_fetches_main_into_a_worktree_that_already_exists() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    let stale = rig.in_clone("refs/remotes/origin/main");
    git(&rig.root, &["clone", "--quiet", "origin.git", "other"]);
    git(
        &rig.root.join("other"),
        &["commit", "--quiet", "--allow-empty", "-m", "main moves"],
    );
    git(
        &rig.root.join("other"),
        &["push", "--quiet", "origin", "main"],
    );
    assert_eq!(rig.in_clone("refs/remotes/origin/main"), stale);

    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);

    assert_eq!(
        rig.in_clone("refs/remotes/origin/main"),
        rig.rev("origin.git", "main")
    );
}

// A rework's worker merges `main` too, and its worktree is cut from the pull
// request's branch rather than from `origin/main`.
#[test]
fn prepare_on_a_pushed_branch_fetches_main_as_well() {
    let rig = Rig::new();
    git(
        &rig.repo(),
        &["push", "--quiet", "origin", "contrib:kelpie/7"],
    );
    git(&rig.root, &["clone", "--quiet", "origin.git", "other"]);
    git(
        &rig.root.join("other"),
        &["commit", "--quiet", "--allow-empty", "-m", "main moves"],
    );
    git(
        &rig.root.join("other"),
        &["push", "--quiet", "origin", "main"],
    );
    assert_ne!(
        rig.rev("repo", "refs/remotes/origin/main"),
        rig.rev("origin.git", "main")
    );

    rig.prepare("kelpie/7", Start::Pushed, &Remote::Origin);

    assert_eq!(
        rig.in_clone("refs/remotes/origin/main"),
        rig.rev("origin.git", "main")
    );
}

#[test]
fn prepare_on_origin_starts_a_pushed_branch_where_origin_holds_it() {
    let rig = Rig::new();
    git(
        &rig.repo(),
        &["push", "--quiet", "origin", "contrib:kelpie/7"],
    );

    rig.prepare("kelpie/7", Start::Pushed, &Remote::Origin);

    assert_eq!(
        rig.at_worktree(&["rev-parse", "HEAD"]),
        rig.rev("origin.git", "kelpie/7")
    );
}

// The worker runs no fetch of its own, so a fetch of main that fails is told
// to the maintainer rather than left as a ref the worker merges blind.
#[test]
fn a_main_fetch_that_fails_is_reported_and_the_worktree_is_kept() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    rig.at_worktree(&["commit", "--quiet", "--allow-empty", "-m", "work"]);
    let worked = rig.at_worktree(&["rev-parse", "HEAD"]);
    rig.drop_main();

    let prepared = rig.prepared("kelpie/7", Start::Main, &Remote::Origin);

    let why = prepared.main_fetch_failed.expect("the failed fetch");
    assert!(why.contains("git fetch"), "{why}");
    assert!(why.contains("main"), "{why}");
    assert_eq!(rig.at_worktree(&["rev-parse", "HEAD"]), worked);
    assert!(rig.build().is_dir());
}

// A branch that is already pushed needs main only for the merge its worker
// runs, so its worktree is made even when that fetch fails.
#[test]
fn a_pushed_branch_starts_and_reports_a_main_fetch_that_failed() {
    let rig = Rig::new();
    git(
        &rig.repo(),
        &["push", "--quiet", "origin", "contrib:kelpie/7"],
    );
    rig.drop_main();

    let prepared = rig.prepared("kelpie/7", Start::Pushed, &Remote::Origin);

    assert!(prepared.main_fetch_failed.is_some());
    assert_eq!(
        rig.at_worktree(&["rev-parse", "HEAD"]),
        rig.rev("origin.git", "kelpie/7")
    );
}

// A new worktree on `Start::Main` starts on that ref, so without it there is
// nothing to cut the branch from.
#[test]
fn a_new_worktree_on_main_without_main_is_an_error() {
    let rig = Rig::new();
    rig.drop_main();

    let err = prepare(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        Start::Main,
        "main",
        &Remote::Origin,
        &rig.build(),
    )
    .unwrap_err();

    assert!(
        matches!(&err, WorktreeError::Git { args, .. } if args.contains("fetch")),
        "{err}"
    );
    assert!(!rig.worktree().exists());
}

#[test]
fn origin_head_on_origin_returns_the_branch_head_as_before() {
    let rig = Rig::new();
    git(
        &rig.repo(),
        &["push", "--quiet", "origin", "contrib:kelpie/7"],
    );

    let tip = origin_head(&rig.tree, &rig.repo(), "kelpie/7", &Remote::Origin).unwrap();

    assert_eq!(tip, rig.rev("origin.git", "kelpie/7"));
}

// A worktree made before worktrees were clones is a linked one on its branch,
// holding work the worker has not committed.
#[test]
fn prepare_turns_a_linked_worktree_into_a_clone_and_keeps_its_files() {
    let rig = Rig::new();
    let at = rig.worktree();
    git(
        &rig.repo(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "kelpie/7",
            at.to_str().unwrap(),
            "origin/main",
        ],
    );
    std::fs::write(at.join("done.txt"), "done\n").unwrap();
    rig.at_worktree(&["add", "done.txt"]);
    rig.at_worktree(&["commit", "--quiet", "-m", "work"]);
    let worked = rig.at_worktree(&["rev-parse", "HEAD"]);
    std::fs::write(at.join("half.txt"), "half\n").unwrap();

    let prepared = rig.prepared("kelpie/7", Start::Main, &Remote::Origin);

    assert_eq!(prepared.git_common_dir, rig.repo().join(".git"));
    assert_eq!(prepared.git_dir, clone::git_dir_of(&at));
    assert!(!rig.repo().join(".git/worktrees").join("wt").exists());
    assert!(!git(&rig.repo(), &["worktree", "list", "--porcelain"]).contains(at.to_str().unwrap()));
    assert_eq!(rig.in_clone("refs/heads/kelpie/7"), worked);
    assert_eq!(rig.at_worktree(&["rev-parse", "HEAD"]), worked);
    assert_eq!(
        std::fs::read_to_string(at.join("half.txt")).unwrap(),
        "half\n"
    );
    assert_eq!(rig.at_worktree(&["status", "--porcelain"]), "?? half.txt");
    assert_eq!(
        rig.in_clone("refs/remotes/origin/main"),
        rig.rev("origin.git", "main")
    );
    // The checkout keeps the branch, so its gc keeps what the clone borrows.
    assert_eq!(rig.rev("repo", "refs/heads/kelpie/7"), worked);
}

#[test]
fn prepare_names_the_checkouts_git_dir_common_and_the_clones_own() {
    let rig = Rig::new();

    let prepared = rig.prepared("kelpie/7", Start::Main, &Remote::Origin);

    assert_eq!(prepared.git_common_dir, rig.repo().join(".git"));
    assert_eq!(prepared.git_dir, clone::git_dir_of(&rig.worktree()));
}

#[test]
fn prepare_mirrors_a_pushed_branchs_tracking_ref_into_its_clone() {
    let rig = Rig::new();
    git(
        &rig.repo(),
        &["push", "--quiet", "origin", "contrib:kelpie/7"],
    );

    rig.prepare("kelpie/7", Start::Pushed, &Remote::Origin);

    assert_eq!(
        rig.in_clone("refs/remotes/origin/kelpie/7"),
        rig.rev("origin.git", "kelpie/7")
    );
    assert_eq!(rig.checkout_branches(), "");
}

#[test]
fn prepare_refuses_a_clone_on_another_branch() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    rig.at_worktree(&["checkout", "--quiet", "-b", "other"]);

    let err = prepare(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        Start::Main,
        "main",
        &Remote::Origin,
        &rig.build(),
    )
    .unwrap_err();

    assert_eq!(err, WorktreeError::Foreign(rig.worktree()));
}

#[test]
fn prepare_refuses_a_clone_that_borrows_another_repos_objects() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    let alternates = clone::git_dir_of(&rig.worktree()).join("objects/info/alternates");
    std::fs::write(&alternates, format!("{}\n", rig.root.display())).unwrap();

    let err = prepare(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        Start::Main,
        "main",
        &Remote::Origin,
        &rig.build(),
    )
    .unwrap_err();

    assert_eq!(err, WorktreeError::Foreign(rig.worktree()));
}

#[test]
fn prepare_refuses_a_folder_that_is_no_worktree() {
    let rig = Rig::new();
    std::fs::create_dir(rig.worktree()).unwrap();

    let err = prepare(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        Start::Main,
        "main",
        &Remote::Origin,
        &rig.build(),
    )
    .unwrap_err();

    assert_eq!(err, WorktreeError::Foreign(rig.worktree()));
}

#[test]
fn prepare_keeps_the_work_of_a_clone_whose_worktree_folder_is_gone() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    rig.at_worktree(&["commit", "--quiet", "--allow-empty", "-m", "work"]);
    let work = rig.in_clone("refs/heads/kelpie/7");
    std::fs::remove_dir_all(rig.worktree()).unwrap();

    let err = prepare(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        Start::Main,
        "main",
        &Remote::Origin,
        &rig.build(),
    )
    .unwrap_err();

    assert_eq!(err, WorktreeError::BranchTaken("kelpie/7".into()));
    assert_eq!(rig.in_clone("refs/heads/kelpie/7"), work);
}

#[test]
fn prepare_remakes_a_clone_whose_worktree_folder_is_gone_and_holds_no_work() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    std::fs::remove_dir_all(rig.worktree()).unwrap();

    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);

    assert_eq!(rig.at_worktree(&["status", "--porcelain"]), "");
}

#[test]
fn prepare_completes_a_clone_whose_checkout_never_ran() {
    let rig = Rig::new();
    std::fs::write(rig.repo().join("a.txt"), "a\n").unwrap();
    git(&rig.repo(), &["add", "a.txt"]);
    git(&rig.repo(), &["commit", "--quiet", "-m", "file"]);
    git(&rig.repo(), &["push", "--quiet", "origin", "main"]);
    // What a kill inside `clone::make` leaves: the link and no checkout.
    clone::make(&rig.tree, &rig.repo(), &rig.worktree(), "kelpie/7", "main").unwrap();
    std::fs::remove_file(rig.worktree().join("a.txt")).unwrap();
    std::fs::remove_file(clone::git_dir_of(&rig.worktree()).join("index")).unwrap();

    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);

    assert_eq!(rig.at_worktree(&["status", "--porcelain"]), "");
    assert!(rig.worktree().join("a.txt").is_file());
}

#[test]
fn prepare_links_a_clone_whose_folder_was_made_and_never_linked() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    // What a kill inside `clone::link` leaves: an empty folder, no `.git`.
    std::fs::remove_dir_all(rig.worktree()).unwrap();
    std::fs::create_dir(rig.worktree()).unwrap();
    std::fs::remove_file(clone::git_dir_of(&rig.worktree()).join("index")).unwrap();

    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);

    assert!(rig.worktree().join(".git").is_file());
    assert_eq!(rig.at_worktree(&["status", "--porcelain"]), "");
}

#[test]
fn prepare_starts_over_a_git_dir_cut_short_between_init_and_alternates() {
    let rig = Rig::new();
    // What a kill inside `clone::skeleton` leaves: `git init --bare` and no alternates.
    git(
        &rig.root,
        &[
            "init",
            "--quiet",
            "--bare",
            clone::git_dir_of(&rig.worktree()).to_str().unwrap(),
        ],
    );

    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);

    assert_eq!(rig.at_worktree(&["status", "--porcelain"]), "");
}

#[test]
fn prepare_refuses_a_git_dir_without_alternates_that_holds_a_ref() {
    let rig = Rig::new();
    let own = clone::git_dir_of(&rig.worktree());
    git(
        &rig.root,
        &["init", "--quiet", "--bare", own.to_str().unwrap()],
    );
    std::fs::write(own.join("refs/heads/mine"), "0".repeat(40) + "\n").unwrap();

    let err = prepare(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        Start::Main,
        "main",
        &Remote::Origin,
        &rig.build(),
    )
    .unwrap_err();

    assert_eq!(err, WorktreeError::Foreign(rig.worktree()));
}

#[test]
fn prepare_refuses_a_clone_without_an_index_whose_folder_holds_files() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    std::fs::remove_file(clone::git_dir_of(&rig.worktree()).join("index")).unwrap();
    std::fs::write(rig.worktree().join("mine.txt"), "work\n").unwrap();

    let err = prepare(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        Start::Main,
        "main",
        &Remote::Origin,
        &rig.build(),
    )
    .unwrap_err();

    assert_eq!(err, WorktreeError::Foreign(rig.worktree()));
}

#[test]
fn prepare_reuses_a_rework_branch_the_checkout_kept_at_the_remotes_head() {
    let rig = Rig::new();
    git(
        &rig.repo(),
        &["push", "--quiet", "origin", "contrib:kelpie/7"],
    );
    git(&rig.repo(), &["branch", "kelpie/7", "contrib"]);

    rig.prepare("kelpie/7", Start::Pushed, &Remote::Origin);

    assert_eq!(
        rig.in_clone("refs/heads/kelpie/7"),
        rig.rev("origin.git", "kelpie/7")
    );
}

#[test]
fn prepare_clears_a_stale_packed_refs_lock_in_the_clones_git_dir() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    let lock = clone::git_dir_of(&rig.worktree()).join("packed-refs.lock");
    let file = std::fs::File::create(&lock).unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(120))
        .unwrap();

    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);

    assert!(!lock.exists());
}

#[test]
fn head_and_change_start_read_the_clone_through_its_trusted_git() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    let main = rig.rev("origin.git", "main");
    assert_eq!(head(&rig.tree, &rig.repo(), &rig.worktree()).unwrap(), main);

    rig.at_worktree(&["commit", "--quiet", "--allow-empty", "-m", "work"]);
    let worked = rig.at_worktree(&["rev-parse", "HEAD"]);

    assert_eq!(
        head(&rig.tree, &rig.repo(), &rig.worktree()).unwrap(),
        worked
    );
    assert_eq!(
        change_start(&rig.tree, &rig.repo(), &rig.worktree(), "origin/main").unwrap(),
        main
    );
}

#[test]
fn head_with_main_returns_the_branch_head_and_brings_main_to_the_checkout() {
    let rig = Rig::new();
    git(
        &rig.repo(),
        &["push", "--quiet", "origin", "contrib:kelpie/7"],
    );
    git(&rig.root, &["clone", "--quiet", "origin.git", "other"]);
    git(
        &rig.root.join("other"),
        &["commit", "--quiet", "--allow-empty", "-m", "main moves"],
    );
    git(
        &rig.root.join("other"),
        &["push", "--quiet", "origin", "main"],
    );

    let tip = head_with_main(&rig.tree, &rig.repo(), "kelpie/7", &Remote::Origin, "main").unwrap();

    assert_eq!(tip, rig.rev("origin.git", "kelpie/7"));
    assert_eq!(
        rig.rev("repo", "refs/remotes/origin/main"),
        rig.rev("origin.git", "main")
    );
}

#[test]
fn base_of_says_whether_the_pushed_head_has_main_or_lags_the_forge() {
    let rig = Rig::new();
    git(
        &rig.repo(),
        &["push", "--quiet", "origin", "contrib:kelpie/7"],
    );
    let pushed = rig.rev("origin.git", "kelpie/7");
    let base = |head: &str| {
        base_of(
            &rig.tree,
            &rig.repo(),
            "kelpie/7",
            head,
            &Remote::Origin,
            "main",
        )
        .unwrap()
    };

    assert_eq!(base(&pushed), Base::Current);
    assert_eq!(base(&rig.rev("origin.git", "main")), Base::Lagging);

    git(&rig.root, &["clone", "--quiet", "origin.git", "other"]);
    git(
        &rig.root.join("other"),
        &["commit", "--quiet", "--allow-empty", "-m", "main moves"],
    );
    git(
        &rig.root.join("other"),
        &["push", "--quiet", "origin", "main"],
    );

    assert_eq!(base(&pushed), Base::Behind);
}

// What origin holds under `name`, one line per branch
fn on_origin(root: &Path, name: &str) -> String {
    git(&root.join("origin.git"), &["branch", "--list", name])
}

#[test]
fn remove_takes_the_clones_git_dir_away() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    let git_dir = clone::git_dir_of(&rig.worktree());
    assert!(git_dir.is_dir());

    remove(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        &rig.build(),
        None,
    )
    .unwrap();

    assert!(!rig.worktree().exists());
    assert!(!git_dir.exists());
    assert!(!rig.build().exists());
}

#[test]
fn remove_takes_a_linked_worktree_and_its_branch_away_as_before() {
    let rig = Rig::new();
    let at = rig.worktree();
    git(
        &rig.repo(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "kelpie/7",
            at.to_str().unwrap(),
            "origin/main",
        ],
    );

    remove(&rig.tree, &rig.repo(), &at, "kelpie/7", &rig.build(), None).unwrap();

    assert!(!at.exists());
    assert_eq!(rig.checkout_branches(), "");
    assert!(!git(&rig.repo(), &["worktree", "list", "--porcelain"]).contains(at.to_str().unwrap()));
}

#[test]
fn remove_finishes_a_removal_that_took_the_worktree_and_left_the_clones_git_dir() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    std::fs::remove_dir_all(rig.worktree()).unwrap();

    remove(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        &rig.build(),
        None,
    )
    .unwrap();

    assert!(!clone::git_dir_of(&rig.worktree()).exists());
}

#[test]
fn remove_takes_the_branch_off_origin_too() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    rig.at_worktree(&["commit", "--quiet", "--allow-empty", "-m", "work"]);
    rig.at_worktree(&["push", "--quiet", "origin", "HEAD:refs/heads/kelpie/7"]);
    assert!(on_origin(&rig.root, "kelpie/7").contains("kelpie/7"));

    remove(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        &rig.build(),
        Some(&Remote::Origin),
    )
    .unwrap();

    assert!(!rig.worktree().exists());
    assert_eq!(on_origin(&rig.root, "kelpie/7"), "");
}

#[test]
fn remove_skips_a_branch_origin_deleted_first() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    rig.at_worktree(&["commit", "--quiet", "--allow-empty", "-m", "work"]);
    rig.at_worktree(&["push", "--quiet", "origin", "HEAD:refs/heads/kelpie/7"]);
    // What a forge does itself with a head branch it merged.
    git(
        &rig.root.join("origin.git"),
        &["update-ref", "-d", "refs/heads/kelpie/7"],
    );

    remove(
        &rig.tree,
        &rig.repo(),
        &rig.worktree(),
        "kelpie/7",
        &rig.build(),
        Some(&Remote::Origin),
    )
    .unwrap();

    assert!(!rig.worktree().exists());
    assert!(!clone::git_dir_of(&rig.worktree()).exists());
}

#[test]
fn holder_of_names_the_worktree_that_holds_the_branch() {
    let rig = Rig::new();
    let at = rig.worktree();
    git(
        &rig.repo(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "kelpie/7",
            at.to_str().unwrap(),
            "origin/main",
        ],
    );
    let own = rig.root.join("elsewhere");

    assert_eq!(
        holder_of(&rig.tree, &rig.repo(), "kelpie/7", &own).unwrap(),
        Some(at.clone())
    );
    // The worktree that holds it is not a holder of it elsewhere.
    assert_eq!(
        holder_of(&rig.tree, &rig.repo(), "kelpie/7", &at).unwrap(),
        None
    );
    assert_eq!(
        holder_of(&rig.tree, &rig.repo(), "kelpie/11", &own).unwrap(),
        None
    );
}

#[test]
fn holds_work_sees_a_commit_only_the_clone_holds() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    assert!(!holds_work(&rig.tree, &rig.repo(), &rig.worktree(), "kelpie/7", "main").unwrap());

    rig.at_worktree(&["commit", "--quiet", "--allow-empty", "-m", "work"]);

    assert!(holds_work(&rig.tree, &rig.repo(), &rig.worktree(), "kelpie/7", "main").unwrap());
}

#[test]
fn holds_work_sees_the_commits_of_a_worktree_folder_that_is_gone() {
    let rig = Rig::new();
    rig.prepare("kelpie/7", Start::Main, &Remote::Origin);
    rig.at_worktree(&["commit", "--quiet", "--allow-empty", "-m", "work"]);
    std::fs::remove_dir_all(rig.worktree()).unwrap();

    assert!(holds_work(&rig.tree, &rig.repo(), &rig.worktree(), "kelpie/7", "main").unwrap());
}

#[test]
fn holds_work_says_no_for_a_branch_and_worktree_that_are_not_there() {
    let rig = Rig::new();

    assert!(!holds_work(&rig.tree, &rig.repo(), &rig.worktree(), "kelpie/7", "main").unwrap());
}
