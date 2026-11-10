//! Telling a running worker something: a Note, Stop and tell, and Stop
//!
//! Each test runs a worker on a [`FakeAgent`] whose first turn waits to be
//! cancelled, so the test decides when the turn ends.

use super::*;
use crate::worker::{Noted, note, stop, stop_and_tell};

/// Run `then` with a project in `rig` whose opener builds no second session
fn told<R>(rig: &Rig, then: impl FnOnce(&Project<'_>) -> R) -> R {
    let binding = binding();
    let open = |_: &Opening<'_>| -> Result<Session, String> {
        Err("a note starts no session of its own".to_owned())
    };
    then(&rig.project(&binding, &open, 2))
}

/// Wait until the turn has said something, which it does once the harness has its prompt
fn until_working(turn: &Mid) {
    loop {
        let event = turn.heard.recv_timeout(WAIT).expect("the turn said nothing");
        if matches!(event, SessionEvent::Text { .. }) {
            return;
        }
    }
}

/// The text of each prompt the harness was sent, oldest first
fn prompts(agent: &FakeAgent) -> Vec<String> {
    agent
        .params("session/prompt")
        .iter()
        .map(|call| call["prompt"][0]["text"].as_str().unwrap().to_owned())
        .collect()
}

/// What the records on disk say of ticket `74`
fn standing(rig: &Rig) -> (Status, Seat) {
    let record = kept(rig).remove(0);
    (record.status, record.seat)
}

#[test]
fn a_note_during_a_turn_is_queued_in_the_record_and_sent_when_the_turn_ends() {
    let rig = Rig::new("note queued");
    let agent = FakeAgent::new()
        .turn(Turn::new().text("working").until_cancelled())
        .turn(Turn::new().text("noted"));
    let turn = mid_turn(&rig, &agent);
    until_working(&turn);

    let noted = told(&rig, |project| {
        note(project, &turn.session, "74", "use the new name")
    })
    .unwrap();

    assert_eq!(noted, Noted::Queued);
    assert_eq!(kept(&rig)[0].notes, ["use the new name"]);
    assert_eq!(
        prompts(&agent).len(),
        1,
        "the note waits for the turn's end"
    );

    turn.session.cancel();
    until_turn_ends(&turn.heard);
    assert_eq!(
        standing(&rig),
        (Status::Running, Seat::Held),
        "the worker goes on, holding its slot"
    );
    until_turn_ends(&turn.heard);

    assert_eq!(
        prompts(&agent)[1..],
        ["use the new name"],
        "the note is the next turn's prompt"
    );
    assert!(kept(&rig)[0].notes.is_empty(), "a sent note is not kept");
    assert_eq!(
        standing(&rig),
        (Status::Ended, Seat::Without),
        "the turn the note started settles the record as any other"
    );
}

#[test]
fn two_notes_go_out_as_one_prompt_in_the_order_they_came() {
    let rig = Rig::new("two notes");
    let agent = FakeAgent::new()
        .turn(Turn::new().text("working").until_cancelled())
        .turn(Turn::new().text("noted"));
    let turn = mid_turn(&rig, &agent);
    until_working(&turn);

    for text in ["first, rename it", "second, and add a test"] {
        let noted = told(&rig, |project| note(project, &turn.session, "74", text)).unwrap();
        assert_eq!(noted, Noted::Queued);
    }
    turn.session.cancel();
    until_turn_ends(&turn.heard);
    until_turn_ends(&turn.heard);

    assert_eq!(
        prompts(&agent)[1..],
        ["first, rename it\n\nsecond, and add a test"],
        "one prompt, one note to a paragraph"
    );
    assert!(kept(&rig)[0].notes.is_empty());
}

#[test]
fn a_note_to_an_idle_worker_starts_its_next_turn() {
    let rig = Rig::new("note idle");
    let agent = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    let turn = mid_turn(&rig, &agent);
    until_turn_ends(&turn.heard);
    assert_eq!(standing(&rig), (Status::Ended, Seat::Without));

    let noted = told(&rig, |project| {
        note(project, &turn.session, "74", "one more thing")
    })
    .unwrap();

    assert_eq!(noted, Noted::Sent);
    assert_eq!(
        standing(&rig),
        (Status::Running, Seat::Held),
        "the worker takes a slot for the turn"
    );
    until_turn_ends(&turn.heard);
    assert_eq!(prompts(&agent)[1..], ["one more thing"]);
    assert_eq!(standing(&rig), (Status::Ended, Seat::Without));
    assert!(kept(&rig)[0].notes.is_empty(), "nothing is queued for it");
}

#[test]
fn a_turn_that_ends_held_keeps_its_notes_for_the_next_turn() {
    let rig = Rig::new("note held");
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    let turn = mid_turn(&rig, &agent);
    until_working(&turn);
    told(&rig, |project| {
        note(project, &turn.session, "74", "mind the docs")
    })
    .unwrap();

    // A harness's own file on the branch holds the ticket for the maintainer
    commit(&turn.worktree, ".claude/settings.json", "{\"hooks\":{}}\n");
    turn.session.cancel();
    until_turn_ends(&turn.heard);

    assert_eq!(standing(&rig), (Status::WaitingOnMaintainer, Seat::Without));
    assert_eq!(kept(&rig)[0].notes, ["mind the docs"], "the note is kept");
    assert_eq!(prompts(&agent).len(), 1, "no turn went out with it");

    // The maintainer rules, and the next start carries the note
    let resumed = kept(&rig)[0].session.clone().expect("no session id kept");
    turn.session.stop();
    let second = FakeAgent::new()
        .can_resume()
        .knows(&resumed.to_string())
        .turn(Turn::new().text("second turn"));
    let binding = binding();
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&second, opening)) };
    let (sink, heard) = events();
    start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    until_turn_ends(&heard);

    assert_eq!(prompts(&second), ["Build the thing\n\nmind the docs"]);
    assert!(kept(&rig)[0].notes.is_empty(), "the turn took the note");
}

#[test]
fn an_old_workers_json_with_no_notes_still_loads() {
    let rig = Rig::new("old json");
    std::fs::create_dir_all(rig.paths.project()).unwrap();
    std::fs::write(
        rig.paths.records(),
        r#"[{"ticket": "74", "branch": "kelpie/74", "session": null,
             "status": "paused", "seat": "without"}]"#,
    )
    .unwrap();

    let records = Records::load(rig.paths.records()).unwrap();

    assert_eq!(records.get("74").unwrap().notes, Vec::<String>::new());
}
