//! A ticket's reads, watched: the logs replayed, the read in flight streamed, and the line
//! between two reads
//!
//! The feed host is driven on its own. The gate rig's scripted reviewer runs no session and
//! keeps no log (`crate::gate::tests::rig`), so each test writes the log a read's session
//! would have kept, in the order it writes it: the entry first, then the event to the sink.

use std::sync::{Arc, Mutex};

use kelpie_engine::run::RunId;
use kelpie_engine::session::log::Log;
use kelpie_engine::worker::Paths;
use kelpie_protocol::{LogEntry, SessionEvent, StopReason};

use crate::review::watch::Reads;

/// The ticket the tests work on
const TICKET: &str = "7";

/// The reviewer every read here is by
const REVIEWER: &str = "kelpie:reviewer";

/// The feeds of a project of the test's own, with no read yet
fn reads() -> (Paths, Reads) {
    let dir = std::env::temp_dir().join(format!("kelpie-watch-{}", RunId::new().unwrap()));
    let paths = Paths::new(&dir, "watch");
    (paths.clone(), Reads::new(&paths))
}

/// A channel that keeps what it is sent, and answers that it was sent
fn taken() -> (
    Arc<Mutex<Vec<SessionEvent>>>,
    impl Fn(SessionEvent) -> bool + Send,
) {
    let kept: Arc<Mutex<Vec<SessionEvent>>> = Arc::default();
    let send = Arc::clone(&kept);
    (kept, move |event| {
        send.lock().unwrap().push(event);
        true
    })
}

/// The events a channel kept
fn kept(ch: &Arc<Mutex<Vec<SessionEvent>>>) -> Vec<SessionEvent> {
    ch.lock().unwrap().clone()
}

/// What the events the tests stream say
fn text(delta: &str) -> SessionEvent {
    SessionEvent::Text {
        delta: delta.to_owned(),
    }
}

/// What the log of a read starts with: the prompt its session was sent
fn prompt(text: &str) -> LogEntry {
    LogEntry::User {
        text: text.to_owned(),
    }
}

/// An event as the log holds it
fn logged(event: SessionEvent) -> LogEntry {
    LogEntry::Event(event)
}

/// The line between two reads
fn divider(text: &str) -> LogEntry {
    LogEntry::Divider {
        text: text.to_owned(),
    }
}

/// Read `round` by `name` as its session begins it: the read is seated on its round, its
/// log opened, and its prompt written
fn read_by(reads: &Reads, paths: &Paths, round: u32, name: &str, second: bool, said: &str) -> Log {
    reads.seated(TICKET, name, second);
    let log = Log::open(paths.review_log_file(TICKET, round)).unwrap();
    log.append(&prompt(said)).unwrap();
    log
}

/// Read `round` by the project's one reviewer, as its session begins it
fn began(reads: &Reads, paths: &Paths, round: u32, second: bool, said: &str) -> Log {
    read_by(reads, paths, round, REVIEWER, second, said)
}

/// A read that was seated and never ran: no session, so no log
fn never_ran(reads: &Reads, name: &str, second: bool) {
    reads.seated(TICKET, name, second);
}

/// One event of a running read, in the order the session sends it: written to its log, then
/// forwarded to the feed
fn streamed(reads: &Reads, log: &Log, event: SessionEvent) {
    log.append(&logged(event.clone())).unwrap();
    reads.feed(TICKET).forward(event);
}

/// A read by `name` that ended its turn, as the pass ran it whole before the tab opened
fn read_ended(
    reads: &Reads,
    paths: &Paths,
    round: u32,
    name: &str,
    second: bool,
    said: &str,
    delta: &str,
) {
    let log = read_by(reads, paths, round, name, second, said);
    streamed(reads, &log, text(delta));
    streamed(
        reads,
        &log,
        SessionEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            usage: None,
        },
    );
}

/// The lines a replay put between its reads, in order
fn dividers(entries: &[LogEntry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            LogEntry::Divider { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_watch_replays_the_reads_log_then_streams_its_events() {
    let (paths, reads) = reads();
    let log = began(&reads, &paths, 1, false, "Read the diff");
    streamed(&reads, &log, text("a finding"));

    let (ch, send) = taken();
    let past = reads.watch(TICKET, 1, send).unwrap();

    assert_eq!(
        past,
        [prompt("Read the diff"), logged(text("a finding"))],
        "the tab reads the read it opened on from its log"
    );

    streamed(&reads, &log, text(" and here is another"));

    assert_eq!(
        kept(&ch),
        [text(" and here is another")],
        "and follows what the read says next"
    );
}

#[test]
fn an_unwatch_stops_the_stream_and_the_read_goes_on() {
    let (paths, reads) = reads();
    let log = began(&reads, &paths, 1, false, "Read the diff");

    let (ch, send) = taken();
    assert_eq!(
        reads.watch(TICKET, 1, send).unwrap(),
        [prompt("Read the diff")]
    );

    streamed(&reads, &log, text("a finding"));
    assert_eq!(kept(&ch), [text("a finding")]);

    reads.unwatch(TICKET, 1);
    streamed(&reads, &log, text("one more"));

    assert_eq!(
        kept(&ch),
        [text("a finding")],
        "the closed tab hears nothing more, and the read was never stopped by it"
    );

    let (again, send) = taken();
    let past = reads.watch(TICKET, 2, send).unwrap();

    assert_eq!(
        past,
        [
            prompt("Read the diff"),
            logged(text("a finding")),
            logged(text("one more")),
        ],
        "a tab that opens again reads the whole read, which went on without it"
    );

    streamed(&reads, &log, text(" and it is still going"));
    assert_eq!(kept(&again), [text(" and it is still going")]);
}

#[test]
fn a_second_look_streams_into_the_same_feed_after_a_divider() {
    let (paths, reads) = reads();
    read_ended(
        &reads,
        &paths,
        1,
        REVIEWER,
        false,
        "Read the diff",
        "a finding",
    );
    let first = reads.feed(TICKET);
    let log = began(&reads, &paths, 2, true, "Read it again");

    assert!(
        Arc::ptr_eq(&first, &reads.feed(TICKET)),
        "the second look streams into the ticket's one feed"
    );

    let (ch, send) = taken();
    let past = reads.watch(TICKET, 1, send).unwrap();

    assert_eq!(
        past,
        [
            prompt("Read the diff"),
            logged(text("a finding")),
            logged(SessionEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                usage: None,
            }),
            divider("kelpie:reviewer reads again"),
            prompt("Read it again"),
        ],
        "the two reads come back as one history, with the line naming the second"
    );

    streamed(&reads, &log, text("and one more"));

    assert_eq!(
        kept(&ch),
        [text("and one more")],
        "and the read in flight streams onto the channel the tab is watching"
    );
}

#[test]
fn a_read_that_never_ran_leaves_its_round_to_the_read_that_wrote_the_log() {
    let (paths, reads) = reads();
    // The read's harness or sandbox refused to start, so it left no log behind it
    never_ran(&reads, "first:reviewer", false);
    // The pass stepped the ticket again, and this read ran on the same round
    read_ended(
        &reads,
        &paths,
        1,
        "second:reviewer",
        false,
        "Read the diff",
        "a finding",
    );
    read_ended(
        &reads,
        &paths,
        2,
        "third:reviewer",
        true,
        "Read it again",
        "one more",
    );

    let (_ch, send) = taken();
    let past = reads.watch(TICKET, 1, send).unwrap();

    assert_eq!(
        dividers(&past),
        ["third:reviewer reads again"],
        "the line above a log names the read that wrote it, not the one that never ran"
    );
}

#[test]
fn a_feed_made_after_a_relaunch_names_each_read_by_the_log_it_left() {
    let (paths, before) = reads();
    read_ended(
        &before,
        &paths,
        1,
        REVIEWER,
        false,
        "Read the diff",
        "a finding",
    );

    // kelpie quit and started again: the feeds are new, the logs are not
    let reads = Reads::new(&paths);
    read_ended(
        &reads,
        &paths,
        2,
        REVIEWER,
        true,
        "Read it again",
        "one more",
    );
    read_ended(
        &reads,
        &paths,
        3,
        "other:reviewer",
        false,
        "One more read",
        "nothing more",
    );

    let (_ch, send) = taken();
    let past = reads.watch(TICKET, 1, send).unwrap();

    assert_eq!(
        dividers(&past),
        ["kelpie:reviewer reads again", "other:reviewer reads",],
        "each read is named by the look seated on its own round, not by its place in the feed"
    );
}
