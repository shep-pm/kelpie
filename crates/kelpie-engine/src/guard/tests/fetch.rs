//! A worker's `git fetch` in the shared clone, answered before it runs
//!
//! Every ticket's worktree shares one `.git`, and a worker may write only
//! its own branch's refs. A bare `git fetch` updates every
//! `refs/remotes/origin/*`, so each other branch's lock fails and git exits 1,
//! which a worker spent turns reading as a broken sandbox. `git pull`,
//! `git remote update` and `git remote prune` write the same refs. Kelpie
//! fetches `origin/main` itself, so the guard answers the attempt instead. A
//! fetch git is pointed at another repo, by `--git-dir` or `GIT_DIR`, is left
//! to git: kelpie cannot tell which repo such a command names.

use super::*;

const FETCHED: &str = "kelpie fetches `origin/main`";
const MERGE: &str = "`git merge origin/main`";

fn refused_with(verdict: Verdict, part: &str, command: &str) {
    let why = refusal(verdict);
    assert!(why.contains(part), "{command}: {why}");
}

#[test]
fn a_fetch_in_the_tickets_repo_is_answered_with_what_kelpie_does() {
    let tree = WorkerTree::new();
    tree.write("src/main.rs", "fn main() {}\n");
    for command in [
        "git fetch",
        "git fetch origin",
        "git fetch --quiet origin main",
        "git fetch --all",
        "git fetch --tags",
        "git pull",
        "git pull origin main",
        "git pull --ff-only",
        "git -C . fetch",
        "cd src && git fetch",
        "bash -c 'git fetch origin'",
        "env git fetch origin",
    ] {
        let why = refusal(tree.bash(command));
        assert!(why.contains(FETCHED), "{command}: {why}");
        assert!(why.contains(MERGE), "{command}: {why}");
        // Kelpie fetches main. Whether it lands in the branch is the worker's
        // merge or kelpie's catch-up, so the answer claims no merge of its own.
        assert!(!why.contains("merges"), "{command}: {why}");
    }
}

// A fetch that names only one ref still writes a ref of `origin`'s, which is
// kelpie's to move, and a tag of what it fetched.
#[test]
fn a_fetch_of_one_ref_of_origins_is_answered_the_same_way() {
    let tree = WorkerTree::new();
    for command in [
        "git fetch origin main",
        "git fetch origin kelpie/7",
        "git fetch origin refs/heads/main:refs/remotes/origin/main",
    ] {
        refused_with(tree.bash(command), FETCHED, command);
    }
}

// The worker's own branch is the one ref of `origin`'s it may write, and
// `git merge` of what kelpie fetched needs no fetch of its own.
#[test]
fn what_a_worker_does_with_the_refs_kelpie_fetched_goes_through() {
    let tree = WorkerTree::new();
    tree.git(&["push", "--quiet", "origin", "HEAD:refs/heads/kelpie/7"]);
    for command in [
        "git merge origin/main",
        "git merge --no-edit origin/main",
        "git log origin/main..HEAD",
        "git ls-remote origin",
        "git push origin HEAD",
    ] {
        assert_eq!(tree.bash(command), Verdict::Allow, "{command}");
    }
}

// A repo the worker made inside its worktree is its own: its refs are writable
// there, and a test fixture that fetches from a local origin is ordinary work.
#[test]
fn a_fetch_in_a_repo_the_worker_made_goes_through() {
    let tree = WorkerTree::new();
    tree.git(&["init", "--quiet", "--bare", "fixture/origin"]);
    tree.git(&["clone", "--quiet", "fixture/origin", "fixture/clone"]);
    tree.git(&[
        "-C",
        "fixture/clone",
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "one",
    ]);
    tree.git(&[
        "-C",
        "fixture/clone",
        "push",
        "--quiet",
        "origin",
        "HEAD:refs/heads/main",
    ]);

    assert_eq!(tree.bash("cd fixture/clone && git fetch"), Verdict::Allow);
    assert_eq!(
        tree.bash("git -C fixture/clone fetch origin"),
        Verdict::Allow
    );
    assert_eq!(tree.bash("cd fixture/clone && git pull"), Verdict::Allow);
}

// A word in a program kelpie does not know may only name git.
#[test]
fn a_fetch_that_only_a_program_names_is_not_refused() {
    let tree = WorkerTree::new();
    assert_eq!(tree.bash("rg 'git fetch'"), Verdict::Allow);
}

// A fetch git is pointed at another repo is left to git. `--git-dir` and
// `GIT_DIR` name a repo kelpie cannot tell the worker's own fixture from.
// `--git-dir=.git` is the shared clone named plainly, and git's own failure
// is what answers it there.
#[test]
fn a_fetch_git_is_pointed_at_another_repo_goes_through() {
    let tree = WorkerTree::new();
    tree.git(&["init", "--quiet", "--bare", "fixture/repo.git"]);
    for command in [
        "git --git-dir=fixture/repo.git fetch origin",
        "git --git-dir fixture/repo.git fetch",
        "git --git-dir=fixture/repo.git pull",
        "GIT_DIR=fixture/repo.git git fetch origin",
        "env GIT_DIR=fixture/repo.git git fetch origin",
        "export GIT_DIR=fixture/repo.git; git fetch origin",
        "git --git-dir=.git fetch",
    ] {
        assert_eq!(tree.bash(command), Verdict::Allow, "{command}");
    }
}

// git searches upward for a repo, so a folder kelpie cannot follow still
// leaves it in the item's shared clone.
#[test]
fn a_fetch_in_a_folder_kelpie_cannot_follow_is_answered() {
    let tree = WorkerTree::new();
    for command in [
        "cd \"$(git rev-parse --show-toplevel)\" && git fetch origin main",
        "cd \"$WT\" && git pull",
        "git -C \"$WT\" fetch",
        "env -C \"$WT\" git fetch origin",
    ] {
        refused_with(tree.bash(command), FETCHED, command);
    }
}

// A folder the call makes inside the worktree holds no repo of its own, so
// git's search walks up into the shared clone.
#[test]
fn a_fetch_in_a_folder_the_call_makes_is_answered() {
    let tree = WorkerTree::new();
    for command in [
        "mkdir -p scratch && cd scratch && git fetch",
        "mkdir -p scratch/nested && cd scratch/nested && git pull",
        "cd scratch && git fetch origin main",
    ] {
        refused_with(tree.bash(command), FETCHED, command);
    }
}

// A repo the call made is the worker's own, and a folder past the worktree is
// no repo of kelpie's, so neither is answered.
#[test]
fn a_fetch_in_no_repo_of_kelpie_s_goes_through() {
    let tree = WorkerTree::new();
    for command in [
        "git init --quiet x && cd x && git fetch",
        "mkdir -p x && cd x && git init --quiet && git fetch",
        "cd /tmp && git fetch",
        "cd ../elsewhere && git fetch",
    ] {
        assert_eq!(tree.bash(command), Verdict::Allow, "{command}");
    }
}

// `git remote update` fetches every remote the way `git fetch` does, and
// `git remote prune` deletes the same refs, so both are answered. What only
// reads the remotes is left alone.
#[test]
fn a_remote_update_or_prune_of_origins_refs_is_answered() {
    let tree = WorkerTree::new();
    for command in [
        "git remote update",
        "git remote update origin",
        "git remote update --fetch",
        "git remote prune origin",
        "cd src && git remote update",
    ] {
        refused_with(tree.bash(command), FETCHED, command);
    }
    for command in [
        "git remote",
        "git remote -v",
        "git remote show origin",
        "git remote set-url origin https://example.invalid/x",
    ] {
        assert_eq!(tree.bash(command), Verdict::Allow, "{command}");
    }
}
