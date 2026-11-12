//! What the gate does with the maintainer's yes: the merge, its withdrawals, and the merge queue

use std::num::NonZeroU64;

use kelpie_engine::ci::{FailedCheck, FailedLog, HeadStatus};
use kelpie_engine::gate::Line;
use kelpie_engine::github::Clock;
use kelpie_engine::ticket::fake::PullCall;
use kelpie_engine::ticket::{ForgeError, OpenPull, PullState, QueueStanding};
use kelpie_engine::worker::Status;

use super::rig::Rig;
use super::{SETTLE, TICKET, prompt, rig, until_some};

const NUMBER: NonZeroU64 = NonZeroU64::new(107).unwrap();
const REMOVAL: &str = "Required status check \"test\" failed.";

/// The worker on the ticket has ended with its work pushed, and CI is green on the latest base
///
/// Two reads, a settling apart, find the head ready to merge. The head is returned.
fn ready(rig: &Rig) -> String {
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    ready_at(rig, &rig.pull(TICKET, &head, HeadStatus::Passed));
    head
}

/// The same, for a pull request as given
fn ready_at(rig: &Rig, pull: &OpenPull) {
    rig.read(std::slice::from_ref(pull));
    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(pull));
    assert!(
        rig.gate.ready().contains(TICKET),
        "the head is ready to merge"
    );
}

/// The merges asked of the forge, with the head each was pinned to
fn merges(rig: &Rig) -> Vec<String> {
    rig.pulls
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            PullCall::Merge { head, .. } => Some(head),
            _ => None,
        })
        .collect()
}

fn marks(rig: &Rig) -> usize {
    (rig.pulls.calls().into_iter())
        .filter(|call| matches!(call, PullCall::MarkReady(_)))
        .count()
}

/// The merge queue shows the pull request as removed `removals` times, the last for `REMOVAL`
fn removed(rig: &Rig, removals: u32) {
    rig.queue.script(
        NUMBER,
        Ok(QueueStanding {
            removals,
            reason: Some(REMOVAL.to_owned()),
            ..QueueStanding::default()
        }),
    );
}

/// The merge queue shows the pull request in the queue
fn in_queue(rig: &Rig) {
    rig.queue.script(
        NUMBER,
        Ok(QueueStanding {
            queued: true,
            ..QueueStanding::default()
        }),
    );
}

fn red() -> HeadStatus {
    HeadStatus::Failed(vec![FailedCheck {
        name: "test".to_owned(),
        url: None,
    }])
}

#[test]
fn a_yes_merges_the_head_it_was_given_on_pinned_to_it() {
    let rig = rig();
    let head = ready(&rig);
    rig.pulls.script_state(NUMBER, Ok(PullState::Merged));

    rig.gate.yes(TICKET).expect("the ticket is ready to merge");
    assert!(merges(&rig).is_empty(), "the yes alone merges nothing");
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);
    assert_eq!(merges(&rig), [head], "one merge, pinned to the head");

    // The next read no longer lists it
    rig.read(&[]);
    assert_eq!(merges(&rig).len(), 1, "and no second");
    assert!(
        rig.seen_merged(TICKET),
        "a merged pull request is left to its cleanup"
    );
}

#[test]
fn a_yes_on_a_ticket_not_ready_is_refused_with_its_line() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    let refused = rig
        .gate
        .yes(TICKET)
        .expect_err("the checks have not settled");

    assert_eq!(refused.line, Some(Line::WaitingForCi));
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);
    assert!(merges(&rig).is_empty(), "a refused yes is not kept");
    assert_eq!(rig.gate.kept(TICKET).and_then(|kept| kept.yes), None);
}

#[test]
fn a_yes_on_a_head_that_moved_is_withdrawn() {
    let rig = rig();
    ready(&rig);
    rig.gate.yes(TICKET).unwrap();
    let moved = rig.commit(TICKET, "late.txt", "late\n");

    rig.read(&[rig.pull(TICKET, &moved, HeadStatus::Passed)]);

    assert!(merges(&rig).is_empty(), "the head moved after the yes");
    assert_eq!(
        rig.gate.kept(TICKET).and_then(|kept| kept.yes),
        None,
        "the yes is withdrawn"
    );
    assert!(
        !rig.gate.ready().contains(TICKET),
        "the gate runs again at the new head"
    );
}

#[test]
fn a_yes_after_ci_stopped_being_green_is_withdrawn() {
    let rig = rig();
    let head = ready(&rig);
    rig.gate.yes(TICKET).unwrap();

    rig.read(&[rig.pull(TICKET, &head, red())]);

    assert!(merges(&rig).is_empty());
    assert_eq!(rig.gate.kept(TICKET).and_then(|kept| kept.yes), None);
    assert!(!rig.gate.ready().contains(TICKET));
}

#[test]
fn a_run_still_going_past_the_settling_time_keeps_the_yes_and_merges_when_it_passes() {
    let rig = rig();
    let head = ready(&rig);
    rig.gate.yes(TICKET).unwrap();

    rig.clock.advance(SETTLE * 10);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Pending(Vec::new()))]);

    assert!(merges(&rig).is_empty());
    assert_eq!(
        rig.gate.kept(TICKET).and_then(|kept| kept.yes),
        Some(head.clone()),
        "the yes waits on the run"
    );

    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);
    assert_eq!(merges(&rig), [head]);
}

#[test]
fn a_yes_after_the_base_moved_is_withdrawn_and_the_branch_caught_up() {
    let rig = rig();
    let head = ready(&rig);
    rig.gate.yes(TICKET).unwrap();
    rig.main_moves("main.txt", "main\n");

    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    assert!(merges(&rig).is_empty());
    assert_eq!(rig.gate.kept(TICKET).and_then(|kept| kept.yes), None);
    assert_ne!(rig.origin("kelpie/7"), head, "the branch was pushed");
    assert_eq!(
        super::parents(&rig, "kelpie/7"),
        [rig.origin("main")],
        "on the latest base"
    );
}

#[test]
fn a_draft_marked_ready_merges_only_once_checks_settle() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let mut draft = rig.pull(TICKET, &head, HeadStatus::Passed);
    draft.draft = true;
    ready_at(&rig, &draft);
    let ready = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.gate.yes(TICKET).unwrap();

    rig.read(std::slice::from_ref(&draft));
    assert_eq!(marks(&rig), 1, "the draft was marked ready");
    assert!(
        merges(&rig).is_empty(),
        "the checks it starts are waited for"
    );

    rig.clock.advance(SETTLE - 1);
    rig.read(std::slice::from_ref(&ready));
    assert!(merges(&rig).is_empty(), "the checks have not settled");

    rig.clock.advance(1);
    rig.read(std::slice::from_ref(&ready));
    assert_eq!(merges(&rig), [head], "merged once they settled");
    assert_eq!(marks(&rig), 1, "and marked ready once");
}

#[test]
fn marking_ready_is_no_foreign_change() {
    let rig = rig();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let mut draft = rig.pull(TICKET, &head, HeadStatus::Passed);
    draft.draft = true;
    ready_at(&rig, &draft);
    rig.gate.yes(TICKET).unwrap();
    rig.read(std::slice::from_ref(&draft));
    assert_eq!(marks(&rig), 1);
    rig.ci.script_logs(
        &head,
        vec![FailedLog {
            check: "test".to_owned(),
            run: 42,
            url: "https://github.com/acme/app/actions/runs/42".to_owned(),
            tail: "panicked at work.txt:3".to_owned(),
        }],
    );

    // The marked pull request goes red, which withdraws the yes and runs the order again
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &head, red())]);

    assert_eq!(rig.gate.kept(TICKET).and_then(|kept| kept.yes), None);
    let prompt = until_some(|| prompt(&rig.agent, 0));
    assert!(prompt.contains("panicked at work.txt:3"), "{prompt}");
    assert_ne!(
        rig.record(TICKET).status,
        Status::WaitingOnMaintainer,
        "kelpie's own mark is no ruling for a foreign change"
    );
}

#[test]
fn a_merge_github_refuses_catches_up_once_then_is_merge_refused() {
    let rig = rig();
    let head = ready(&rig);
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.pulls.script_merge(Err(ForgeError::Failed(
        "Pull request is not mergeable".to_owned(),
    )));

    rig.gate.yes(TICKET).unwrap();
    rig.read(std::slice::from_ref(&pull));

    assert_eq!(merges(&rig).len(), 1);
    let kept = rig.gate.kept(TICKET).unwrap();
    assert_eq!(kept.merge_refusals, 1);
    assert_eq!(
        kept.yes, None,
        "the yes is withdrawn, and the order ran again"
    );
    assert_eq!(kept.sent, None, "nothing is waiting on a merge");
    assert!(
        rig.gate.ready().contains(TICKET),
        "the head is ready to ask for again"
    );
    assert_ne!(rig.record(TICKET).status, Status::WaitingOnMaintainer);

    rig.gate.yes(TICKET).unwrap();
    rig.read(std::slice::from_ref(&pull));

    assert_eq!(merges(&rig).len(), 2);
    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    let reason = record.reason.expect("the ruling says why");
    assert!(reason.contains("could not merge"), "{reason}");
    assert!(reason.contains("Pull request is not mergeable"), "{reason}");
    assert_eq!(rig.gate.kept(TICKET).and_then(|kept| kept.yes), None);
}

#[test]
fn a_relaunch_between_the_record_and_the_merge_does_not_merge_twice() {
    let rig = rig();
    let head = ready(&rig);
    rig.gate.yes(TICKET).unwrap();
    // The merge was recorded as sent, and kelpie quit before, or after, the call
    let now = Clock::now(&rig.clock);
    rig.gate
        .keep_for_test(TICKET, |kept| kept.merge_sent(&head, 0, now));
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);

    rig.read(std::slice::from_ref(&pull));
    assert!(merges(&rig).is_empty(), "the pass watches the pull request");

    // It had merged: the listing no longer holds it
    rig.pulls.script_state(NUMBER, Ok(PullState::Merged));
    rig.read(&[]);
    assert!(merges(&rig).is_empty());
    assert!(rig.seen_merged(TICKET), "the merge is seen as done");
}

#[test]
fn a_merge_that_left_no_trace_is_sent_again_once_the_forge_had_time_to_show_it() {
    let rig = rig();
    let head = ready(&rig);
    rig.gate.yes(TICKET).unwrap();
    let now = Clock::now(&rig.clock);
    rig.gate
        .keep_for_test(TICKET, |kept| kept.merge_sent(&head, 0, now));
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);

    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(&pull));

    assert_eq!(
        merges(&rig),
        [head],
        "the call never happened, so it is made"
    );
}

#[test]
fn a_queued_pull_is_watched_until_it_merges() {
    let rig = rig();
    let head = ready(&rig);
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);

    // The merge call is made, and the base's merge queue takes the pull request in
    rig.gate.yes(TICKET).unwrap();
    rig.read(std::slice::from_ref(&pull));
    assert_eq!(merges(&rig).len(), 1);
    assert!(
        !rig.gate.ready().contains(TICKET),
        "no Merge is offered for a pull request already sent"
    );

    in_queue(&rig);
    rig.clock.advance(SETTLE * 4);
    rig.read(std::slice::from_ref(&pull));
    assert_eq!(merges(&rig).len(), 1, "queued once, not again");
    assert!(
        rig.gate
            .kept(TICKET)
            .and_then(|kept| kept.sent)
            .unwrap()
            .queued
    );
    assert_eq!(rig.gate.line(TICKET), Some(Line::InMergeQueue));

    rig.pulls.script_state(NUMBER, Ok(PullState::Merged));
    rig.read(&[]);
    assert_eq!(merges(&rig).len(), 1);
    assert!(rig.seen_merged(TICKET), "the queue landed it");
}

#[test]
fn a_pull_already_in_the_queue_when_the_yes_is_read_is_not_merged_again() {
    let rig = rig();
    let head = ready(&rig);
    rig.gate.yes(TICKET).unwrap();
    in_queue(&rig);

    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    assert!(
        merges(&rig).is_empty(),
        "a second merge call would refuse it"
    );
    assert_eq!(rig.gate.line(TICKET), Some(Line::InMergeQueue));
}

#[test]
fn a_queue_removal_goes_back_to_the_worker_uncounted() {
    let rig = rig();
    let head = ready(&rig);
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.gate.yes(TICKET).unwrap();
    rig.read(std::slice::from_ref(&pull));

    removed(&rig, 1);
    rig.read(std::slice::from_ref(&pull));

    let prompt = until_some(|| prompt(&rig.agent, 0));
    assert!(prompt.contains("merge queue"), "{prompt}");
    assert!(prompt.contains(REMOVAL), "{prompt}");
    let kept = rig.gate.kept(TICKET).unwrap();
    assert_eq!(kept.fix_turns, 0, "a removal is not a fix attempt");
    assert_eq!(kept.red, [head], "the head it was sent for is kept");
    assert_eq!((kept.sent, kept.yes), (None, None), "the yes is spent");
    assert_eq!(rig.record(TICKET).status, Status::Running);
}

#[test]
fn a_second_removal_at_one_head_is_merge_refused() {
    let rig = rig();
    let head = ready(&rig);
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.gate.yes(TICKET).unwrap();
    rig.read(std::slice::from_ref(&pull));
    removed(&rig, 1);
    rig.read(std::slice::from_ref(&pull));
    until_some(|| prompt(&rig.agent, 0));
    // The worker ends without pushing, and the head is ready again
    rig.set_status(TICKET, Status::Ended);
    rig.read(std::slice::from_ref(&pull));
    assert!(rig.gate.ready().contains(TICKET));

    rig.gate.yes(TICKET).unwrap();
    rig.read(std::slice::from_ref(&pull));
    removed(&rig, 2);
    rig.read(std::slice::from_ref(&pull));

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    let reason = record.reason.expect("the ruling says why");
    assert!(reason.contains("could not merge"), "{reason}");
    assert!(reason.contains(REMOVAL), "{reason}");
    assert_eq!(
        rig.agent.params("session/prompt").len(),
        1,
        "the worker was not sent back a second time"
    );
}

#[test]
fn an_armed_auto_merge_on_a_head_gone_red_is_disarmed() {
    let rig = rig();
    let head = ready(&rig);
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.gate.yes(TICKET).unwrap();
    rig.read(std::slice::from_ref(&pull));
    rig.queue.script(
        NUMBER,
        Ok(QueueStanding {
            armed: true,
            ..QueueStanding::default()
        }),
    );

    rig.read(&[rig.pull(TICKET, &head, red())]);

    assert!(
        rig.pulls
            .calls()
            .contains(&PullCall::DisableAutoMerge(NUMBER))
    );
    let kept = rig.gate.kept(TICKET).unwrap();
    assert_eq!((kept.sent, kept.yes), (None, None));
}

#[test]
fn a_merge_that_never_reached_the_forge_keeps_the_yes_and_is_no_refusal() {
    let rig = rig();
    let head = ready(&rig);
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.gate.yes(TICKET).unwrap();
    for _ in 0..2 {
        rig.pulls
            .script_merge(Err(ForgeError::Spawn("gh is not installed".to_owned())));
        rig.read(std::slice::from_ref(&pull));
    }

    let kept = rig.gate.kept(TICKET).unwrap();
    assert_eq!(kept.merge_refusals, 0, "the forge refused nothing");
    assert_eq!(kept.yes, Some(head.clone()), "the yes stands");
    assert_eq!(kept.sent, None, "nothing was sent");
    assert_ne!(rig.record(TICKET).status, Status::WaitingOnMaintainer);

    rig.read(std::slice::from_ref(&pull));
    assert_eq!(merges(&rig).len(), 3, "the next pass tried again");
}

#[test]
fn an_armed_auto_merge_disarmed_by_hand_is_no_queue_removal() {
    let rig = rig();
    let head = ready(&rig);
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.gate.yes(TICKET).unwrap();
    rig.queue.script(
        NUMBER,
        Ok(QueueStanding {
            armed: true,
            ..QueueStanding::default()
        }),
    );
    rig.read(std::slice::from_ref(&pull));
    assert!(
        merges(&rig).is_empty(),
        "armed already, so not merged again"
    );

    rig.queue.script(NUMBER, Ok(QueueStanding::default()));
    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(&pull));

    assert_eq!(prompt(&rig.agent, 0), None, "the worker is not sent back");
    let kept = rig.gate.kept(TICKET).unwrap();
    assert_eq!(kept.red, Vec::<String>::new());
    assert_eq!(merges(&rig), [head], "the merge was sent again");
}
