//! What the gate does with a red run, on the same rig as its other passes

use kelpie_engine::ci::{FailedCheck, FailedLog, HeadStatus};
use kelpie_engine::settings::Field;
use kelpie_engine::worker::{Seat, Status};
use serde_json::json;

use super::rig::Rig;
use super::{SETTLE, TICKET, prompt, rig, until_some};

const RUN: &str = "https://github.com/acme/app/actions/runs/42";

/// The failed `test` check, as a listing shows it
fn red() -> HeadStatus {
    HeadStatus::Failed(vec![FailedCheck {
        name: "test".to_owned(),
        url: Some(RUN.to_owned()),
    }])
}

/// What the CI's log of run 42 says
fn log(tail: &str) -> Vec<FailedLog> {
    vec![FailedLog {
        check: "test".to_owned(),
        run: 42,
        url: RUN.to_owned(),
        tail: tail.to_owned(),
    }]
}

/// Read the red `head` once it has had time to register its checks
fn read_settled(rig: &Rig, head: &str) {
    let pull = rig.pull(TICKET, head, red());
    rig.read(std::slice::from_ref(&pull));
    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(&pull));
}

#[test]
fn a_red_run_sends_a_fix_turn_naming_the_check_with_its_log() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    rig.ci
        .script_logs(&head, log("thread 'main' panicked at work.txt:3: left != right"));

    read_settled(&rig, &head);

    let prompt = until_some(|| prompt(&rig.agent, 0));
    assert!(prompt.contains("test"), "{prompt}");
    assert!(prompt.contains("panicked at work.txt:3"), "{prompt}");
    assert!(prompt.contains("starting from `origin`'s head"), "{prompt}");
    let kept = rig.gate.kept(TICKET).expect("the gate keeps the ticket");
    assert_eq!(kept.fix_turns, 1, "the turn counts against the fix attempts");
    assert_eq!(kept.red, [head], "the head it was sent for is kept");
    assert!(rig.ci.reruns().is_empty(), "the log names a changed file");
}

#[test]
fn logs_naming_no_changed_file_rerun_the_run_once_for_that_head() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    rig.ci
        .script_logs(&head, log("FAILED tests/flaky_network.rs: timed out"));

    read_settled(&rig, &head);

    assert_eq!(rig.ci.reruns(), [42], "the run was started again");
    assert!(
        rig.agent.params("session/prompt").is_empty(),
        "no turn was spent before the rerun"
    );
    assert_eq!(
        rig.gate.kept(TICKET).map(|kept| kept.reran),
        Some(vec![head.clone()]),
        "the head is kept as rerun"
    );

    // It fails again, once the rerun's checks have had their time
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &head, red())]);

    assert_eq!(rig.ci.reruns(), [42], "the same head is rerun once");
    let prompt = until_some(|| prompt(&rig.agent, 0));
    assert!(prompt.contains("failed again"), "{prompt}");
}

#[test]
fn the_same_head_red_again_with_nothing_pushed_is_still_red() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    rig.ci.script_logs(&head, log("panicked at work.txt:3"));
    // A fix turn went out for this head, and the worker ended without pushing
    rig.gate
        .keep_for_test(TICKET, |kept| kept.fix_turn_sent(&head, true));

    read_settled(&rig, &head);

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    let reason = record.reason.expect("the ruling says why");
    assert!(reason.contains("failed again"), "{reason}");
    assert!(reason.contains("test"), "{reason}");
    assert!(
        rig.agent.params("session/prompt").is_empty(),
        "the worker was not sent back again"
    );
}

#[test]
fn past_the_fix_attempts_is_still_red() {
    let rig = rig();
    rig.set(Field::FixAttempts, &json!(1));
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    rig.ci.script_logs(&head, log("panicked at work.txt:3"));
    // The one attempt went out for an earlier head
    rig.gate
        .keep_for_test(TICKET, |kept| kept.fix_turn_sent("an-earlier-head", true));

    read_settled(&rig, &head);

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    let reason = record.reason.expect("the ruling says why");
    assert!(reason.contains("fix attempts are used up"), "{reason}");
    assert!(
        rig.agent.params("session/prompt").is_empty(),
        "no turn past the cap"
    );
}

#[test]
fn a_fix_turn_takes_a_slot_and_moves_the_ticket_to_building() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    rig.ci.script_logs(&head, log("panicked at work.txt:3"));
    assert_eq!(rig.record(TICKET).seat, Seat::Without, "an ended worker");

    read_settled(&rig, &head);

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::Running, "the ticket is in Building");
    assert_eq!(record.seat, Seat::Held, "the turn holds a slot");
}
