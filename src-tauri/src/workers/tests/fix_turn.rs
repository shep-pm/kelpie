//! A fix turn: the prompt the gate sends a worker back with
//!
//! Each test starts a real worker through [`Workers::start`] on a [`FakeAgent`], lets its
//! turn end, and then calls [`Workers::fix_turn`] as the gate does. No test needs `gh`, a
//! model or the network.

use kelpie_engine::session::fake::{FakeAgent, FakeNetwork, Turn};
use kelpie_engine::settings::Field;
use kelpie_engine::worker::{Record, Seat, Status};
use kelpie_protocol::SessionEvent;
use serde_json::json;

use super::Rig;
use super::tell::{
    TICKET, opener, prompts, record, session, standing, started, until, until_turns,
};
use crate::workers::{NoteOutcome, cancel_on, lock};

/// What the gate sends a worker whose pull request is red
const PROMPT: &str = "The check `test` failed:\n\nassertion failed: left == right";

/// What a resumed worker's first prompt starts with, before the notes it carries
const CARRY_ON: &str = "Carry on from where you stopped.";

/// Call `fix_turn` on ticket 12 with `agent`'s session as the opener
///
/// No source is given: ticket 12's worker has had a turn, so its text is never read.
fn fix(rig: &Rig, agent: &FakeAgent, prompt: &str) -> Result<NoteOutcome, String> {
    let (heard, kept) = session();
    let open = opener(rig, agent, &FakeNetwork::default(), &heard, &kept);
    let folder = rig.state.project_path_of(&rig.project).unwrap();
    rig.state.workers().fix_turn(
        &rig.project,
        &folder,
        rig.state.tools(),
        TICKET,
        prompt,
        Err("no ticket is read for a worker that has had a turn".to_owned()),
        &open,
    )
}

#[test]
fn a_fix_turn_to_a_live_session_is_its_next_turn() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("fixed"));
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_turns(&heard, 1);
    assert!(
        rig.state.workers().session(&rig.project, TICKET).is_some(),
        "a worker whose turn ended keeps its session",
    );

    let sent = fix(&rig, &agent, PROMPT).unwrap();

    assert_eq!(sent, NoteOutcome::Sent);
    until_turns(&heard, 2);
    assert_eq!(
        prompts(&agent)[1..],
        [PROMPT],
        "the prompt is the worker's next turn, on the session it had",
    );
    assert_eq!(agent.launches(), 1, "no second session was started");
    assert!(record(&rig).notes.is_empty(), "a sent prompt is not kept");
}

#[test]
fn a_fix_turn_after_a_relaunch_resumes_the_session_with_the_prompt() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("fixed"));
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_turns(&heard, 1);
    let session_id = record(&rig).session.expect("its turn gave it a session");
    // What a relaunch leaves: the record says what the worker did, and no session
    assert!(
        rig.state.workers().forget_session(&rig.project, TICKET),
        "the worker kept a session to forget"
    );

    let sent = fix(&rig, &agent, PROMPT).unwrap();

    assert_eq!(sent, NoteOutcome::Sent);
    until(|| prompts(&agent).len() == 2);
    assert_eq!(agent.launches(), 2, "the session was started again");
    assert!(
        agent.methods().contains(&"session/resume".to_owned()),
        "and it resumed the conversation the record names"
    );
    // A session's first prompt has the worker's instructions before it
    let carried = format!("{CARRY_ON}\n\n{PROMPT}");
    assert!(
        prompts(&agent)[1].ends_with(&carried),
        "the resumed session's first prompt does not end with {carried:?}",
    );
    assert_eq!(
        record(&rig).session,
        Some(session_id),
        "it is the same conversation",
    );
    assert!(record(&rig).notes.is_empty(), "the prompt went out");
}

#[test]
fn a_fix_turn_with_no_free_slot_waits_in_line_and_starts_when_one_frees() {
    let rig = Rig::new();
    rig.with_origin();
    rig.set(Field::WorkerSlots, json!(1));
    let agent = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("fixed"));
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_turns(&heard, 1);
    // Another ticket's worker takes the one slot
    let records = rig.state.workers().records(&rig.project).unwrap();
    {
        let mut records = lock(&records);
        let mut busy = Record::new("7", "kelpie/7");
        busy.status = Status::Running;
        records.keep(busy).unwrap();
        records.seat(1).unwrap();
        assert_eq!(records.get("7").unwrap().seat, Seat::Held);
    }

    let waiting = fix(&rig, &agent, PROMPT).unwrap();

    assert_eq!(waiting, NoteOutcome::Queued, "no slot was free");
    assert_eq!(prompts(&agent).len(), 1, "nothing was sent while in line");
    assert_eq!(standing(&rig), (Status::Running, Seat::Waiting));
    assert_eq!(
        record(&rig).notes,
        [PROMPT],
        "the prompt waits in the record"
    );

    lock(&records).set_status("7", Status::Ended).unwrap();
    let retried = fix(&rig, &agent, PROMPT).unwrap();

    assert_eq!(retried, NoteOutcome::Sent, "the freed slot is taken");
    until(|| prompts(&agent).len() == 2);
    let carried = format!("{CARRY_ON}\n\n{PROMPT}");
    assert!(
        prompts(&agent)[1].ends_with(&carried),
        "the prompt went out once, however often it was asked for: it does not end with {carried:?}",
    );
    assert_eq!(
        prompts(&agent)[1].matches(PROMPT).count(),
        1,
        "asking twice kept the prompt once"
    );
}

#[test]
fn a_fix_turn_shows_the_worker_running_until_its_turn_ends() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("fixing").until_cancelled());
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_turns(&heard, 1);
    assert_eq!(standing(&rig), (Status::Ended, Seat::Without));

    fix(&rig, &agent, PROMPT).unwrap();
    until(|| {
        lock(&heard)
            .iter()
            .any(|event| matches!(event, SessionEvent::Text { delta } if delta == "fixing"))
    });

    assert_eq!(
        standing(&rig),
        (Status::Running, Seat::Held),
        "the worker runs, holding a slot, so its ticket is in Building",
    );

    cancel_on(&rig.state, TICKET).unwrap();
    until_turns(&heard, 2);
    assert_eq!(
        standing(&rig),
        (Status::Paused, Seat::Without),
        "the turn's end settles the worker and frees the slot",
    );
}
