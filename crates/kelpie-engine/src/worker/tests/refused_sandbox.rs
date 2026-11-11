//! A worker whose sandbox cannot start waits Paused and holds no slot
//!
//! The opener names the refusal through [`Opening::refuse`], as the app's does
//! once its probe of the worker's sandbox is refused (`docs/design/sandbox-setup.md`,
//! Running without a sandbox).

use super::*;
use crate::sandbox::Refusal;

/// What an opener does when bwrap refuses its sandbox
fn refused(opening: &Opening<'_>) -> Result<Session, String> {
    Err(opening.refuse(Refusal::NoCapabilities))
}

/// The words a refused worker's record and its start's error carry
fn why() -> String {
    format!(
        "its sandbox cannot start because {}",
        Refusal::NoCapabilities
    )
}

#[test]
fn a_refused_sandbox_parks_the_worker_paused_with_the_refusals_words() {
    let rig = Rig::new("refused");
    let binding = binding();
    let project = rig.project(&binding, &refused, 1);

    let (sink, heard) = events();
    let error =
        start(&project, &ticket("74", "First ticket"), sink).expect_err("the sandbox cannot start");

    assert!(
        matches!(error, StartError::Sandbox(Refusal::NoCapabilities)),
        "{error:?}"
    );
    assert!(
        error
            .to_string()
            .contains(&why().replace("its", "the worker's")),
        "{error}"
    );
    assert!(heard.try_recv().is_err(), "nothing ran for it");
    let records = kept(&rig);
    assert_eq!(records.len(), 1, "the refused ticket keeps its record");
    assert_eq!(records[0].status, Status::Paused);
    assert_eq!(records[0].seat, Seat::Without);
    assert_eq!(records[0].reason.as_deref(), Some(why().as_str()));
}

#[test]
fn a_refused_worker_holds_no_slot_so_the_next_ticket_runs() {
    let rig = Rig::new("refused slot");
    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("second ticket"));
    let project = rig.project(&binding, &refused, 1);
    let (sink, _) = events();
    start(&project, &ticket("74", "First ticket"), sink).expect_err("the sandbox cannot start");

    // The one slot is free, so the next ticket takes it
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let project = rig.project(&binding, &open, 1);
    let (sink, heard) = events();
    let second = start(&project, &ticket("75", "Second ticket"), sink).unwrap();

    let Started::Running { record, .. } = second else {
        panic!("the second ticket did not run: {second:?}");
    };
    assert_eq!(record.seat, Seat::Held);
    until_turn_ends(&heard);
    let seats = kept(&rig)
        .iter()
        .map(|record| (record.ticket.clone(), record.status, record.seat))
        .collect::<Vec<_>>();
    assert_eq!(
        seats,
        [
            ("74".to_owned(), Status::Paused, Seat::Without),
            ("75".to_owned(), Status::Ended, Seat::Without),
        ]
    );
}

#[test]
fn a_refused_worker_starts_once_its_sandbox_can_and_loses_the_reason() {
    let rig = Rig::new("refused then ready");
    let binding = binding();
    let project = rig.project(&binding, &refused, 1);
    let (sink, _) = events();
    start(&project, &ticket("74", "First ticket"), sink).expect_err("the sandbox cannot start");
    assert!(kept(&rig)[0].reason.is_some(), "the refusal is kept");

    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let project = rig.project(&binding, &open, 1);
    let (sink, _heard) = events();
    let again = start(&project, &ticket("74", "First ticket"), sink).unwrap();

    let Started::Running {
        record, session, ..
    } = again
    else {
        panic!("the ticket did not start: {again:?}");
    };
    assert_eq!(record.reason, None);
    assert_eq!(kept(&rig)[0].reason, None);
    assert_eq!(kept(&rig)[0].status, Status::Running);
    session.stop();
}

#[test]
fn a_refused_restart_keeps_the_session_id_and_notes_of_its_record() {
    let rig = Rig::new("refused restart");
    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("first turn"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();
    start(
        &rig.project(&binding, &open, 1),
        &ticket("74", "First ticket"),
        sink,
    )
    .unwrap();
    until_turn_ends(&heard);
    let session = kept(&rig)[0].session.clone().expect("no session id kept");
    rig.records
        .lock()
        .unwrap()
        .add_note("74", "Mind the tests")
        .unwrap();

    let (sink, _) = events();
    start(
        &rig.project(&binding, &refused, 1),
        &ticket("74", "First ticket"),
        sink,
    )
    .expect_err("the sandbox cannot start");

    let record = kept(&rig).remove(0);
    assert_eq!(record.status, Status::Paused);
    assert_eq!(record.session, Some(session));
    assert_eq!(record.notes, ["Mind the tests"]);
}
