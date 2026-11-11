//! The needs-you lines a ticket carries while its worker waits on the maintainer

use std::collections::BTreeSet;

use kelpie_engine::worker::Status;
use kelpie_protocol::{SessionEvent, StopReason};

use super::{BoardEvent, BoardView, Rig, Source, TicketView, listed};

/// A rig whose board lists tickets 12 and 13, both with workers on them
fn two_tickets() -> Rig {
    let mut rig = Rig::new(Source::new());
    let open = vec![
        listed(12, "Add the thing", &["ready-for-agent"], 100),
        listed(13, "Mend the other", &["ready-for-agent"], 200),
    ];
    rig.source.tickets.script_ready(Ok(open.clone()));
    rig.source.tickets.script_open(Ok(open));
    rig.poller.read().unwrap();
    rig.work_on("12");
    rig.work_on("13");
    rig
}

fn view(event: Option<BoardEvent>) -> BoardView {
    let Some(BoardEvent::Board(view)) = event else {
        panic!("expected a board, got {event:?}");
    };
    view
}

fn ticket<'a>(view: &'a BoardView, id: &str) -> &'a TicketView {
    view.tickets.iter().find(|t| t.id == id).unwrap()
}

/// `status` on the record of `ticket`, with `reason` beside it
fn wait(rig: &Rig, ticket: &str, status: Status, reason: Option<&str>) {
    let mut records = rig.records.lock().unwrap();
    records.set_status(ticket, status).unwrap();
    records
        .set_reason(ticket, reason.map(str::to_owned))
        .unwrap();
}

fn request(id: &str, host: &str) -> SessionEvent {
    SessionEvent::SandboxRequest {
        id: id.to_owned(),
        host: host.to_owned(),
        why: "to fetch a crate".to_owned(),
    }
}

fn turn_ended() -> SessionEvent {
    SessionEvent::TurnEnded {
        stop_reason: StopReason::EndTurn,
        usage: None,
    }
}

#[test]
fn a_held_worker_marks_its_ticket_with_the_fences_line() {
    let mut rig = two_tickets();
    let line = "the branch changes a harness's own files: .claude/settings.json (Claude Code)";
    wait(&rig, "12", Status::WaitingOnMaintainer, Some(line));

    let board = view(rig.poller.place());

    assert_eq!(ticket(&board, "12").needs_you, [line]);
    assert!(ticket(&board, "13").needs_you.is_empty());
}

#[test]
fn a_paused_worker_marks_its_ticket_paused() {
    let mut rig = two_tickets();
    wait(&rig, "12", Status::Paused, None);

    let board = view(rig.poller.place());

    assert_eq!(
        ticket(&board, "12").needs_you,
        ["Paused, resumes when you say so"]
    );
}

#[test]
fn a_paused_worker_with_a_reason_says_it_after_the_pause() {
    let mut rig = two_tickets();
    let reason = "its sandbox cannot start because the kernel does not allow user namespaces";
    wait(&rig, "12", Status::Paused, Some(reason));

    let board = view(rig.poller.place());

    assert_eq!(
        ticket(&board, "12").needs_you,
        [format!("Paused, resumes when you say so: {reason}")]
    );
}

#[test]
fn each_waiting_sandbox_request_is_its_own_line_and_its_own_count() {
    let mut rig = two_tickets();
    let feed = rig.feed("12");
    feed.forward(request("r1", "crates.io"));
    feed.forward(request("r2", "static.crates.io"));

    let board = view(rig.poller.place());

    assert_eq!(
        ticket(&board, "12").needs_you,
        [
            "Wants to reach crates.io",
            "Wants to reach static.crates.io"
        ]
    );
    assert_eq!(board.needs_you, 2);
}

#[test]
fn an_answered_request_and_a_turn_that_ended_clear_the_marker() {
    let mut rig = two_tickets();
    let feed = rig.feed("12");

    // Answered: only that request goes
    feed.forward(request("r1", "crates.io"));
    feed.forward(request("r2", "npmjs.org"));
    assert_eq!(view(rig.poller.place()).needs_you, 2);
    feed.answered("r1");
    let board = view(rig.poller.place());
    assert_eq!(ticket(&board, "12").needs_you, ["Wants to reach npmjs.org"]);

    // A turn that ends answers what waits denied
    feed.forward(turn_ended());
    let board = view(rig.poller.place());
    assert!(ticket(&board, "12").needs_you.is_empty());
    assert_eq!(board.needs_you, 0);

    // So does a session that fails
    feed.forward(request("r3", "pypi.org"));
    assert_eq!(view(rig.poller.place()).needs_you, 1);
    feed.forward(SessionEvent::Failed {
        message: "the harness exited".to_owned(),
    });
    assert_eq!(view(rig.poller.place()).needs_you, 0);
}

#[test]
fn the_boards_count_is_the_sum_of_every_tickets_lines() {
    let mut rig = two_tickets();
    wait(
        &rig,
        "12",
        Status::WaitingOnMaintainer,
        Some("the branch could not be read"),
    );
    wait(&rig, "13", Status::Paused, None);
    let feed = rig.feed("13");
    feed.forward(request("r1", "crates.io"));

    let board = view(rig.poller.place());

    assert_eq!(ticket(&board, "12").needs_you.len(), 1);
    assert_eq!(ticket(&board, "13").needs_you.len(), 2);
    assert_eq!(board.needs_you, 3);
}

#[test]
fn a_ticket_the_gate_found_ready_to_merge_says_so_beside_its_other_lines() {
    let mut rig = two_tickets();
    rig.poller = std::mem::replace(
        &mut rig.poller,
        Rig::poller(&rig.source, &rig.records, &rig.feeds),
    )
    .with_ready(Box::new(|| BTreeSet::from(["12".to_owned()])));
    rig.feed("12").forward(request("r1", "crates.io"));

    let board = view(rig.poller.place());

    assert_eq!(
        ticket(&board, "12").needs_you,
        ["Ready to merge", "Wants to reach crates.io"]
    );
    assert!(ticket(&board, "13").needs_you.is_empty());
    assert_eq!(board.needs_you, 2);
}
