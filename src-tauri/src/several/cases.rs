//! Several tickets at once, and what the outside world sees of them
//!
//! Each case starts real workers through [`crate::workers::Workers`] on a
//! [`FakeAgent`] each, with the project's GitHub stood in by [`super::forge::Github`],
//! and asserts only what a person could see: the board's columns, the pull requests
//! opened, and the prompts each worker got. No case needs `gh`, the network or a model.

use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::worker::Status;

use super::Rig;
use crate::board::Column;
use crate::workers::{NoteOutcome, WorkerStarted};

#[test]
fn two_tickets_started_at_once_each_reach_review_with_a_draft_of_their_own() {
    let rig = Rig::new(2);
    rig.ticket(12, "Add the thing", "It does one.", &FakeAgent::new());
    rig.ticket(13, "Fix the other", "It does two.", &FakeAgent::new());
    assert_eq!(
        rig.columns(),
        [("12".into(), Column::Ready), ("13".into(), Column::Ready)],
        "both tickets wait on the board before a worker starts",
    );

    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);

    let pulls = rig.github.pulls();
    let branches: Vec<_> = pulls.iter().map(|pull| pull.branch.as_str()).collect();
    assert!(
        branches.contains(&"kelpie/12") && branches.contains(&"kelpie/13"),
        "a draft from each worker's own branch: {branches:?}",
    );
    for (ticket, title) in [(12, "feat: Add the thing"), (13, "feat: Fix the other")] {
        let pull = pulls
            .iter()
            .find(|pull| pull.branch == format!("kelpie/{ticket}"))
            .unwrap();
        assert_eq!(
            pull.title, title,
            "each draft is from its own worker's file"
        );
        assert!(pull.draft, "kelpie opens a draft");
        assert_eq!(
            pull.body.lines().last(),
            Some(format!("Resolves #{ticket}").as_str()),
            "and it closes its own ticket",
        );
    }
    assert_eq!(
        rig.columns(),
        [("12".into(), Column::Review), ("13".into(), Column::Review)],
        "both tickets sit in Review",
    );
}

#[test]
fn with_one_slot_the_second_ticket_waits_and_starts_when_the_first_turn_ends() {
    let rig = Rig::new(1);
    let first = FakeAgent::new()
        .turn(Turn::new().text("working").until_cancelled())
        .turn(Turn::new().text("done"));
    let second = FakeAgent::new();
    rig.ticket(12, "Add the thing", "It does one.", &first);
    rig.ticket(13, "Fix the other", "It does two.", &second);
    rig.start(12);
    rig.until_working(12);

    let waiting = rig.start(13);

    assert_eq!(
        waiting,
        WorkerStarted {
            ticket: "13".to_owned(),
            waiting: true
        },
        "no slot is free",
    );
    assert_eq!(second.launches(), 0, "nothing runs for a ticket in line");
    assert_eq!(
        rig.columns(),
        [
            ("12".into(), Column::Building),
            ("13".into(), Column::Building)
        ],
        "the ticket in line is on the board with the one at work",
    );

    assert_eq!(
        rig.stop_and_tell(12, "wrap it up"),
        NoteOutcome::Sent,
        "the first worker's turn is cut short and its next one is the maintainer's text",
    );
    rig.until_turns(12, 2);
    let started = rig.start(13);

    assert_eq!(
        started,
        WorkerStarted {
            ticket: "13".to_owned(),
            waiting: false
        },
        "the turn that ended freed the slot",
    );
    rig.until_turns(13, 1);
    assert_eq!(
        rig.prompts(&second).len(),
        1,
        "the second worker got its prompt"
    );
    assert!(
        rig.prompts(&second)[0].contains("It does two."),
        "and it is the second ticket's text: {:?}",
        rig.prompts(&second),
    );
}

#[test]
fn a_note_to_one_worker_reaches_only_that_worker() {
    let rig = Rig::new(2);
    let first = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    let second = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    rig.ticket(12, "Add the thing", "It does one.", &first);
    rig.ticket(13, "Fix the other", "It does two.", &second);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);

    let noted = rig.note(12, "use the new name");

    assert_eq!(
        noted,
        NoteOutcome::Sent,
        "a worker between turns takes it as its next turn"
    );
    rig.until_turns(12, 2);
    let first_prompts = rig.prompts(&first);
    assert_eq!(first_prompts.len(), 2);
    assert_eq!(
        first_prompts[1], "use the new name",
        "the first worker's next prompt is the note",
    );
    let second_prompts = rig.prompts(&second);
    assert_eq!(
        second_prompts.len(),
        1,
        "the second worker was not prompted again"
    );
    assert!(
        !second_prompts[0].contains("use the new name"),
        "and what it was told is its ticket alone: {second_prompts:?}",
    );
    assert_eq!(second.launches(), 1);
}

#[test]
fn a_ruling_answered_for_one_ticket_leaves_the_other_alone() {
    let rig = Rig::new(2);
    let first = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    let second = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    rig.ticket(12, "Add the thing", "It does one.", &first);
    rig.ticket(13, "Fix the other", "It does two.", &second);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    rig.rule(12, "the pull request is still red");
    rig.rule(13, "the pull request is still red");

    let answered = rig.note(12, "the failure is in the fixture, not the code");

    assert_eq!(
        answered,
        NoteOutcome::Sent,
        "the Note answers the ruling by sending the worker back",
    );
    rig.until_turns(12, 2);
    let first_prompts = rig.prompts(&first);
    assert_eq!(first_prompts.len(), 2);
    assert!(
        first_prompts[1].ends_with("the failure is in the fixture, not the code"),
        "the first worker's next prompt carries the answer",
    );
    assert_eq!(
        rig.prompts(&second).len(),
        1,
        "the second worker was not prompted again",
    );
    assert_eq!(second.launches(), 1, "and no session started for it");
    let waiting = rig.record(13);
    assert_eq!(
        (waiting.status, waiting.reason.as_deref()),
        (
            Status::WaitingOnMaintainer,
            Some("the pull request is still red")
        ),
        "the other ticket still waits on its own ruling",
    );
    assert_eq!(
        rig.record(12).reason,
        None,
        "the answered ticket's ruling is over",
    );
}
