//! The gate the app runs, on a bare origin, worktrees and stand-ins for the forge
//!
//! Each test makes a project on a local origin, ends a worker on a ticket with its work
//! pushed, and has the gate read a listing, which runs the passes the order calls for.
//! No test needs `gh`, a model or the network.

use std::thread;
use std::time::{Duration, Instant};

mod rig;

use kelpie_engine::ci::{HeadStatus, Queued};
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::worker::{Seat, Status};
use rig::{Rig, git};

/// The ticket the tests work on
const TICKET: &str = "7";

/// How long the gate waits for CI's checks to register
const SETTLE: u64 = 120;

/// A rig whose workers run on an agent that works until it is cancelled
fn rig() -> Rig {
    Rig::new(FakeAgent::new().turn(Turn::new().text("Fixing.").until_cancelled()))
}

/// The text of `at`'s prompt to the agent, `None` before it arrives
fn prompt(agent: &FakeAgent, at: usize) -> Option<String> {
    agent
        .params("session/prompt")
        .into_iter()
        .nth(at)
        .and_then(|params| params["prompt"][0]["text"].as_str().map(str::to_owned))
}

/// Wait for `ask` to answer, and return what it answered
fn until_some<T>(mut ask: impl FnMut() -> Option<T>) -> T {
    let since = Instant::now();
    loop {
        if let Some(found) = ask() {
            return found;
        }
        assert!(
            since.elapsed() < Duration::from_secs(20),
            "it never happened"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// The parents of the commit `at` names in the origin
fn parents(rig: &Rig, at: &str) -> Vec<String> {
    let origin = rig.root.join("origin.git");
    let line = git(&origin, &["rev-list", "--parents", "-n", "1", at]);
    line.split(' ').skip(1).map(str::to_owned).collect()
}

/// Whether the worktree of `ticket` has a merge in progress
fn merging(rig: &Rig, ticket: &str) -> bool {
    rig.worktree(ticket).join(".git").exists()
        && std::process::Command::new("git")
            .current_dir(rig.worktree(ticket))
            .args(["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .output()
            .unwrap()
            .status
            .success()
}

#[test]
fn a_review_ticket_behind_its_base_is_caught_up_and_pushed() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    rig.main_moves("main.txt", "main\n");

    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    let pushed = rig.origin("kelpie/7");
    assert_ne!(pushed, head, "the branch was pushed on the latest base");
    assert_eq!(
        parents(&rig, "kelpie/7"),
        [rig.origin("main")],
        "it was rebased onto the base"
    );
    let kept = rig.gate.kept(TICKET).expect("the gate keeps the ticket");
    assert!(
        kept.pushed.contains(&pushed),
        "the pushed head is kelpie's own, so a read of it is no foreign change"
    );
}

#[test]
fn a_branch_behind_its_base_with_no_conflict_passes_unrebased_under_only_on_conflict() {
    let rig = rig();
    rig.catch_up_on_conflicts();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    rig.main_moves("main.txt", "main\n");
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);

    rig.read(std::slice::from_ref(&pull));
    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(&pull));

    assert_eq!(rig.origin("kelpie/7"), head, "the branch was not rebased");
    assert!(
        rig.gate.ready().contains(TICKET),
        "the pass went on to CI, and found the head green"
    );
}

#[test]
fn a_branch_behind_its_base_that_conflicts_is_caught_up_under_only_on_conflict() {
    let rig = rig();
    rig.catch_up_on_conflicts();
    let head = rig.worker_on(TICKET, "shared.txt", "mine\n");
    rig.main_moves("shared.txt", "theirs\n");

    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    assert!(
        merging(&rig, TICKET),
        "the catch-up was tried, and its conflict is left in the worktree"
    );
    assert!(
        rig.gate
            .kept(TICKET)
            .and_then(|kept| kept.conflict)
            .is_some(),
        "the worker was sent back for it"
    );
}

#[test]
fn a_conflict_sends_a_fix_turn_naming_the_files_with_the_merge_in_progress() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "shared.txt", "mine\n");
    rig.main_moves("shared.txt", "theirs\n");

    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    let prompt = until_some(|| prompt(&rig.agent, 0));
    assert!(prompt.contains("shared.txt"), "{prompt}");
    assert!(
        merging(&rig, TICKET),
        "the worker's turn starts with the merge in progress"
    );
    assert_eq!(rig.origin("kelpie/7"), head, "nothing was pushed");
}

#[test]
fn a_fourth_conflict_rules_and_aborts_the_merge() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "shared.txt", "mine\n");
    rig.main_moves("shared.txt", "theirs\n");
    // The three turns the worker has had, each at a head and a base of its own
    rig.gate.keep_for_test(TICKET, |kept| {
        for turn in 0..3 {
            kept.conflict_sent(&format!("head-{turn}"), &format!("base-{turn}"));
        }
    });

    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    assert!(!merging(&rig, TICKET), "the merge was given up");
    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    assert!(
        record
            .reason
            .as_deref()
            .is_some_and(|why| why.contains("shared.txt")),
        "{:?}",
        record.reason
    );
    assert!(
        rig.agent.params("session/prompt").is_empty(),
        "no fourth turn was sent"
    );
}
