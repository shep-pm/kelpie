//! A reviewer's sandbox: the worktree read, its tests folder written, and GitHub
//! out of reach
//!
//! A child of `tests/sandbox.rs`, which is its own `main` (crabpot's Linux helper
//! is this binary run again).

use std::fs;
use std::path::{Path, PathBuf};

use kelpie_engine::sandbox::{Project, Reads, Reviewer, Sandbox};

use super::host::working;
use super::support::{Scratch, harness_in, sh};

/// The folders of one reviewer in a scratch: a worktree holding a file, its tests
/// folder, and the sandbox they are started in
struct Review {
    worktree: PathBuf,
    tests: PathBuf,
    sandbox: Sandbox,
}

/// A reviewer's sandbox in `scratch`, whose project allowlists `allowlist`, once it
/// is known to run commands, or `None` when this host cannot run one
fn started(scratch: &Scratch, test: &str, allowlist: &[String]) -> Option<Review> {
    let worktree = scratch.folder("data/worktrees/1");
    fs::write(worktree.join("lib.rs"), "fn main() {}").unwrap();
    let common = scratch.folder("work/project/.git");
    let tests = scratch.folder("data/reviews/1/tests");
    let state = scratch.folder("data/reviews/1/state");
    let project = Project {
        folder: &scratch.project(),
        network_allowlist: allowlist,
        reads: Reads::Project,
        sessions_root: &scratch.sessions_root(),
    };
    let reviewer = Reviewer {
        worktree: &worktree,
        git_common_dir: &common,
        state_dir: &state,
        tests: Some(&tests),
        kelpie_home: &scratch.root.join("data"),
    };
    let harness = harness_in(Path::new("/bin/sh"), &[]);
    let sandbox = Sandbox::start_reviewer(&harness, &project, &reviewer)
        .unwrap_or_else(|error| panic!("the sandbox did not start: {error}"));
    let sandbox = working(sandbox, test)?;
    Some(Review {
        worktree,
        tests,
        sandbox,
    })
}

pub(super) fn a_reviewer_cannot_write_the_worktree() {
    let scratch = Scratch::new("reviewer-worktree");
    let Some(review) = started(&scratch, "a_reviewer_cannot_write_the_worktree", &[]) else {
        return;
    };
    // The control: the reviewer reads the file, so the refusals below are the
    // policy's and not a sandbox that cannot see the worktree
    let read = sh(
        &review.sandbox,
        &format!("cat {}", review.worktree.join("lib.rs").display()),
    );
    assert_eq!(String::from_utf8_lossy(&read.stdout), "fn main() {}");
    let added = review.worktree.join("added.rs");
    let changed = review.worktree.join("lib.rs");

    let add = sh(&review.sandbox, &format!("echo no > {}", added.display()));
    let change = sh(&review.sandbox, &format!("echo no > {}", changed.display()));

    assert!(!add.status.success(), "a new file was not refused: {add:?}");
    assert!(
        !change.status.success(),
        "a change to a file was not refused: {change:?}"
    );
    assert!(!added.exists(), "a new file landed in the worktree");
    assert_eq!(fs::read_to_string(&changed).unwrap(), "fn main() {}");
}

pub(super) fn a_reviewer_writes_its_tests_folder() {
    let scratch = Scratch::new("reviewer-tests");
    let Some(review) = started(&scratch, "a_reviewer_writes_its_tests_folder", &[]) else {
        return;
    };
    let test = review.tests.join("finding_1.rs");

    let run = sh(
        &review.sandbox,
        &format!("echo '#[test] fn fails() {{}}' > {}", test.display()),
    );

    assert!(run.status.success(), "{run:?}");
    assert_eq!(
        fs::read_to_string(&test).unwrap(),
        "#[test] fn fails() {}\n"
    );
}

pub(super) fn a_reviewer_cannot_reach_github() {
    let scratch = Scratch::new("reviewer-github");
    let allowlist = ["example.com".to_owned()];
    let Some(review) = started(&scratch, "a_reviewer_cannot_reach_github", &allowlist) else {
        return;
    };
    let code = |host: &str| {
        let out = sh(
            &review.sandbox,
            &format!("curl -sS -m 20 -o /dev/null -w '%{{http_connect}}' https://{host}/"),
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };

    // The control: a host the project allows is not refused by the same proxy
    assert_ne!(code("example.com"), "403", "the allowed host was refused");
    for host in ["github.com", "api.github.com"] {
        assert_eq!(code(host), "403", "{host} was not refused by the proxy");
    }
}
