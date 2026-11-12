//! The gate the app runs, on a bare origin, worktrees and stand-ins for the forge
//!
//! Each test makes a project on a local origin, ends a worker on a ticket with its work
//! pushed, and has the gate read a listing, which runs the passes the order calls for.
//! No test needs `gh`, a model or the network.

use std::thread;
use std::time::{Duration, Instant};

mod after;
mod merge;
mod red;
pub(crate) mod rig;

use kelpie_engine::ci::{HeadStatus, Queued};
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::ticket::PullState;
use kelpie_engine::ticket::fake::PullCall;
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

/// A worker that committed a harness's own file, which the gate rules on
fn fenced(rig: &Rig) -> String {
    rig.worker_on(TICKET, "work.txt", "mine\n");
    let worktree = rig.worktree(TICKET);
    std::fs::create_dir_all(worktree.join(".claude")).unwrap();
    std::fs::write(worktree.join(".claude/settings.json"), "{}\n").unwrap();
    git(&worktree, &["add", ".claude/settings.json"]);
    rig.commit_staged(TICKET)
}

#[test]
fn a_ruling_records_the_worker_waiting_with_its_reason_and_holds_no_slot() {
    let rig = rig();
    let head = fenced(&rig);

    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    assert!(
        record
            .reason
            .as_deref()
            .is_some_and(|why| why.contains(".claude/settings.json")),
        "{:?}",
        record.reason
    );
    assert_eq!(record.seat, Seat::Without, "a ruled worker holds no slot");
}

#[test]
fn a_ruled_ticket_gets_no_pass_until_its_head_moves() {
    let rig = rig();
    let head = fenced(&rig);
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.read(std::slice::from_ref(&pull));
    assert_eq!(rig.record(TICKET).status, Status::WaitingOnMaintainer);
    // A pass sets this to the listing's number, so a pass that ran again would show
    rig.gate.keep_for_test(TICKET, |kept| kept.pull = Some(999));

    rig.read(std::slice::from_ref(&pull));
    assert_eq!(
        rig.gate.kept(TICKET).and_then(|kept| kept.pull),
        Some(999),
        "no pass ran at the head it ruled on"
    );

    // The worker takes the file out, which moves the head
    let worktree = rig.worktree(TICKET);
    git(&worktree, &["rm", "--quiet", ".claude/settings.json"]);
    let moved = rig.commit_staged(TICKET);
    let pull = rig.pull(TICKET, &moved, HeadStatus::Passed);
    rig.read(std::slice::from_ref(&pull));
    assert_eq!(
        rig.gate.kept(TICKET).and_then(|kept| kept.pull),
        Some(pull.number.get()),
        "a pass ran at the new head"
    );
    assert_eq!(
        rig.record(TICKET).status,
        Status::Ended,
        "the ruling is given up, and the ticket is back in the gate"
    );
}

#[test]
fn a_worker_waiting_for_another_reason_than_the_gates_ruling_gets_no_pass() {
    let rig = rig();
    let head = fenced(&rig);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);
    assert_eq!(rig.record(TICKET).status, Status::WaitingOnMaintainer);

    // The maintainer resumed the worker, which took the file out and then asked a question
    let worktree = rig.worktree(TICKET);
    git(&worktree, &["rm", "--quiet", ".claude/settings.json"]);
    let moved = rig.commit_staged(TICKET);
    rig.park(TICKET, "Which of the two should it be?");

    rig.read(&[rig.pull(TICKET, &moved, HeadStatus::Passed)]);

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    assert_eq!(
        record.reason.as_deref(),
        Some("Which of the two should it be?"),
        "the question stays for the maintainer"
    );
}

#[test]
fn a_ready_mark_goes_when_the_worker_runs_again() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.read(std::slice::from_ref(&pull));
    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(&pull));
    assert!(rig.gate.ready().contains(TICKET));

    // A Note from the maintainer has the worker running again
    rig.set_status(TICKET, Status::Running);
    rig.read(std::slice::from_ref(&pull));

    assert!(
        rig.gate.ready().is_empty(),
        "the head the gate checked is not the one the worker is on"
    );
}

#[test]
fn a_conflict_ruling_on_a_ticket_whose_head_moved_while_ruled_is_recorded() {
    let rig = rig();
    let head = fenced(&rig);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);
    assert_eq!(rig.record(TICKET).status, Status::WaitingOnMaintainer);

    // The head moves by a hand push that conflicts with the base, and the turns are used up
    let worktree = rig.worktree(TICKET);
    git(&worktree, &["rm", "--quiet", ".claude/settings.json"]);
    std::fs::write(worktree.join("shared.txt"), "mine\n").unwrap();
    git(&worktree, &["add", "shared.txt"]);
    let moved = rig.commit_staged(TICKET);
    rig.main_moves("shared.txt", "theirs\n");
    rig.gate.keep_for_test(TICKET, |kept| {
        for turn in 0..3 {
            kept.conflict_sent(&format!("head-{turn}"), &format!("base-{turn}"));
        }
    });

    rig.read(&[rig.pull(TICKET, &moved, HeadStatus::Passed)]);

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
    assert!(!merging(&rig, TICKET), "the merge was given up");
}

#[test]
fn a_pull_merged_by_hand_is_read_as_merged_and_not_ruled_on() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.read(std::slice::from_ref(&pull));
    assert!(rig.gate.kept(TICKET).is_some(), "the gate kept the ticket");
    rig.pulls.script_state(pull.number, Ok(PullState::Merged));

    rig.read(&[]);

    assert!(rig.seen_merged(TICKET), "the gate read it merged");
    assert_eq!(rig.pulls.calls(), [PullCall::State(pull.number)]);
    assert_eq!(
        rig.record(TICKET).status,
        Status::Ended,
        "nothing was ruled"
    );
}

#[test]
fn a_pull_closed_on_github_is_the_closed_ruling() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.read(std::slice::from_ref(&pull));
    rig.pulls.script_state(pull.number, Ok(PullState::Closed));

    rig.read(&[]);

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    assert_eq!(
        record.reason.as_deref(),
        Some("The pull request was closed without merging.")
    );
}

#[test]
fn a_run_stuck_queued_past_the_ci_wait_is_rerun_once() {
    let rig = rig();
    rig.ci_waits(1);
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let stuck = HeadStatus::Pending(vec![Queued {
        name: "test".to_owned(),
        run: Some(42),
        running: false,
        held: false,
    }]);
    let pull = rig.pull(TICKET, &head, stuck);

    rig.read(std::slice::from_ref(&pull));
    assert!(rig.ci.reruns().is_empty(), "the wait has not passed");
    rig.clock.advance(SETTLE + 60);
    rig.read(std::slice::from_ref(&pull));
    rig.read(std::slice::from_ref(&pull));

    assert_eq!(rig.ci.reruns(), [42], "the run was rerun once, not again");
}

#[test]
fn a_green_head_on_the_latest_base_is_ready_to_merge_and_needs_you() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);

    rig.read(std::slice::from_ref(&pull));
    assert!(rig.gate.ready().is_empty(), "the checks have not settled");
    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(&pull));

    assert_eq!(rig.gate.ready().into_iter().collect::<Vec<_>>(), [TICKET]);
    assert_eq!(
        rig.record(TICKET).status,
        Status::Ended,
        "the worker stays ended, and the ticket in Review"
    );
}

#[test]
fn a_running_or_paused_worker_gets_no_pass() {
    for status in [Status::Running, Status::Paused] {
        let rig = rig();
        let head = rig.worker_on(TICKET, "work.txt", "mine\n");
        rig.set_status(TICKET, status);

        rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

        assert!(
            rig.gate.kept(TICKET).is_none(),
            "a {status:?} worker got a pass"
        );
    }
}

#[test]
fn a_pass_runs_off_the_poll_thread_and_one_at_a_time_per_ticket() {
    let rig = rig();
    rig.ci_waits(1);
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let stuck = HeadStatus::Pending(vec![Queued {
        name: "test".to_owned(),
        run: Some(42),
        running: false,
        held: false,
    }]);
    let pull = rig.pull(TICKET, &head, stuck);
    rig.read(std::slice::from_ref(&pull));
    rig.clock.advance(SETTLE + 60);
    rig.ci.hold();

    // The read returns while the pass is held in its call to CI
    rig.gate
        .after_read(&rig.records(), std::slice::from_ref(&pull));
    until_some(|| (rig.ci.asked() == 1).then_some(()));
    assert!(rig.gate.at_work_on(TICKET), "the pass is at work");

    // Another read lands while it is, and starts no second pass
    rig.gate
        .after_read(&rig.records(), std::slice::from_ref(&pull));
    thread::sleep(Duration::from_millis(200));
    assert_eq!(rig.ci.asked(), 1, "one pass at a time on a ticket");

    rig.ci.release();
    rig.gate.idle();
    assert_eq!(rig.ci.reruns(), [42]);
}
