//! Metering a worker's turns: the line each one leaves in the project's ledger
//!
//! Each test runs a worker on a [`FakeAgent`] with no model behind it, metered
//! into an [`InFlight`] of its own over the rig's ledger file.

use std::fs;
use std::time::{Duration, Instant};

use kelpie_protocol::Usage;

use super::*;
use crate::session::SessionId;
use crate::usage::{Ended, InFlight, Line, Role, Timestamp};

/// The tokens a turn reports, as `Usage` holds them
fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Usage {
    Usage {
        input,
        output,
        cache_read,
        cache_write,
        cache_lifetime: None,
    }
}

/// A worker on ticket `74` in `project`, its turn in flight
fn running(project: &Project<'_>) -> Mid {
    let (sink, heard) = events();
    let started = start(project, &ticket("74", "Build the thing"), sink).unwrap();
    let Started::Running {
        session, worktree, ..
    } = started
    else {
        panic!("the ticket was not started: {started:?}");
    };
    assert!(
        matches!(until_started(&heard), SessionEvent::Started { .. }),
        "the turn is in flight"
    );
    Mid {
        worktree,
        session,
        heard,
    }
}

/// Wait up to `WAIT` for the ledger to hold `want` lines
///
/// A line is written when the turn that spent it is over, which for a turn the
/// harness ended by failing is after its `Failed` reaches the test: the sink that
/// writes it is dropped once the session's worker ends.
fn ledger_holding(rig: &Rig, want: usize) -> Vec<Line> {
    let until = Instant::now() + WAIT;
    loop {
        let lines = ledger(rig);
        if lines.len() >= want || Instant::now() >= until {
            return lines;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The ledger lines on disk, oldest first
fn ledger(rig: &Rig) -> Vec<Line> {
    let kept = fs::read_to_string(rig.paths.usage()).unwrap_or_default();
    kept.lines()
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_str(line).expect("a ledger line is not one JSON object"))
        .collect()
}

/// What each line says, in the three facts a test reads
fn told(lines: &[Line]) -> Vec<(String, Ended, Usage)> {
    lines
        .iter()
        .map(|line| (line.ticket.clone(), line.ended, line.usage))
        .collect()
}

#[test]
fn a_workers_first_turn_writes_one_line_with_the_usage_acp_reported() {
    let rig = Rig::new("usage first turn");
    let flight = InFlight::new(rig.paths.usage());
    let agent = FakeAgent::new().turn(Turn::new().text("working").usage(1_200, 300, 900, 100));
    let binding = Binding {
        harness: Sourced {
            value: Some(crate::settings::Harness::Goose),
            from: Origin::BuiltIn,
        },
        ..binding()
    };
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let mut project = rig.project(&binding, &open, 2);
    project.usage = Some(&flight);

    let turn = running(&project);
    until_turn_ends(&turn.heard);

    let lines = ledger(&rig);
    let [line] = lines.as_slice() else {
        panic!("one line for the worker's first turn: {lines:#?}");
    };
    assert_eq!(line.ticket, "74");
    assert_eq!(line.role, Role::Worker);
    assert_eq!(line.harness, Some(crate::settings::Harness::Goose));
    assert_eq!(line.ended, Ended::Done);
    assert_eq!(line.usage, usage(1_200, 300, 900, 100));
    assert_eq!(
        line.session.as_ref().map(SessionId::as_str),
        Some("fake-1"),
        "the id the harness gave the session"
    );
    assert!(
        line.end >= line.start && line.seconds < 60,
        "the turn's own seconds: {:?}",
        (line.start, line.end, line.seconds)
    );
}

#[test]
fn a_turn_cancelled_writes_a_cancelled_line() {
    let rig = Rig::new("usage cancelled");
    let flight = InFlight::new(rig.paths.usage());
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    let binding = binding();
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let mut project = rig.project(&binding, &open, 2);
    project.usage = Some(&flight);

    let turn = running(&project);
    turn.session.cancel();
    until_turn_ends(&turn.heard);

    let lines = ledger(&rig);
    assert_eq!(
        told(&lines),
        [("74".to_owned(), Ended::Cancelled, usage(0, 0, 0, 0))],
        "a cancelled turn reports no tokens: {lines:#?}"
    );
}

#[test]
fn a_turn_that_fails_writes_a_failed_line_once() {
    let rig = Rig::new("usage failed");
    let flight = InFlight::new(rig.paths.usage());
    // The harness goes away mid-turn, which ends the send with a `Failed` and no `TurnEnded`
    let agent = FakeAgent::new().turn(Turn::new().text("working").exit());
    let binding = binding();
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let mut project = rig.project(&binding, &open, 2);
    project.usage = Some(&flight);

    let turn = running(&project);
    loop {
        let event = turn
            .heard
            .recv_timeout(WAIT)
            .expect("the turn never failed");
        if matches!(event, SessionEvent::Failed { .. }) {
            break;
        }
    }

    let lines = ledger_holding(&rig, 1);
    assert_eq!(
        told(&lines),
        [("74".to_owned(), Ended::Failed, usage(0, 0, 0, 0))],
        "the turn the harness went on: {lines:#?}"
    );
    assert!(
        flight.stop_all(Timestamp::now()).is_empty(),
        "the turn's draft is gone, so nothing is left to write it again"
    );
}

#[test]
fn a_note_sent_when_a_turn_ends_opens_and_writes_a_draft_of_its_own() {
    let rig = Rig::new("usage note");
    let flight = InFlight::new(rig.paths.usage());
    let agent = FakeAgent::new()
        .turn(Turn::new().text("working").until_cancelled())
        .turn(Turn::new().text("noted").usage(70, 30, 0, 0));
    let binding = binding();
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let mut project = rig.project(&binding, &open, 2);
    project.usage = Some(&flight);

    let turn = running(&project);
    let noted = note(&project, &turn.session, "74", "use the new name").unwrap();
    assert_eq!(noted, Noted::Queued);

    turn.session.cancel();
    until_turn_ends(&turn.heard);
    until_turn_ends(&turn.heard);

    assert_eq!(
        told(&ledger(&rig)),
        [
            ("74".to_owned(), Ended::Cancelled, usage(0, 0, 0, 0)),
            ("74".to_owned(), Ended::Done, usage(70, 30, 0, 0)),
        ],
        "the note's turn is metered as any other"
    );
}
