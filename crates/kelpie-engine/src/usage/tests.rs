//! The ledger's line, and the file it is appended to

use std::fs;
use std::path::PathBuf;

use kelpie_protocol::{CacheLifetime, Usage};

use super::*;
use crate::run::RunId;

/// A folder of its own, removed when the test ends
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("kelpie-usage-{name}-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The tokens one turn's harness reported
fn spent() -> Usage {
    Usage {
        input: 1_200,
        output: 300,
        cache_read: 900,
        cache_write: 100,
        cache_lifetime: Some(CacheLifetime::FiveMinutes),
    }
}

/// A turn on ticket `74`, started at the second `1_700_000_000`
fn draft() -> Draft {
    draft_on("74")
}

/// A turn on `ticket`, started at the second `1_700_000_000`
fn draft_on(ticket: &str) -> Draft {
    Draft {
        ticket: ticket.to_owned(),
        role: Role::Worker,
        harness: Some(Harness::ClaudeCode),
        model: Some("sonnet-high".to_owned()),
        effort: Some("high".to_owned()),
        session: Some(SessionId::from("fake-1")),
        started: Timestamp::from_secs(1_700_000_000),
    }
}

#[test]
fn a_draft_opens_with_who_and_when_and_a_line_ends_it_with_the_tokens() {
    let line = draft().line(
        Timestamp::from_secs(1_700_000_090),
        Ended::Done,
        Some(spent()),
    );

    assert_eq!(line.ticket, "74");
    assert_eq!(line.role, Role::Worker);
    assert_eq!(line.harness, Some(Harness::ClaudeCode));
    assert_eq!(line.model.as_deref(), Some("sonnet-high"));
    assert_eq!(line.effort.as_deref(), Some("high"));
    assert_eq!(
        line.session.as_ref().map(SessionId::as_str),
        Some("fake-1"),
        "the session its harness gave it"
    );
    assert_eq!(
        (line.start, line.end, line.seconds),
        (
            Timestamp::from_secs(1_700_000_000),
            Timestamp::from_secs(1_700_000_090),
            90
        ),
        "the turn's own seconds, from its start to its end"
    );
    assert_eq!(line.ended, Ended::Done);
    assert_eq!(line.usage, spent());

    let unreported = draft().line(Timestamp::from_secs(1_700_000_090), Ended::Failed, None);
    assert_eq!(
        unreported.usage,
        Usage {
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            cache_lifetime: None,
        },
        "a harness that says nothing spends nothing kelpie can name"
    );
}

#[test]
fn each_line_is_one_json_line_appended_to_the_file() {
    let dir = Dir::new("append");
    let file = dir.path().join("projects/p1/usage.jsonl");
    let lines = [
        draft().line(
            Timestamp::from_secs(1_700_000_090),
            Ended::Done,
            Some(spent()),
        ),
        draft().line(Timestamp::from_secs(1_700_000_180), Ended::Cancelled, None),
    ];

    for line in &lines {
        append_to(&file, line).unwrap();
    }

    let kept = fs::read_to_string(&file).unwrap();
    assert_eq!(kept.lines().count(), 2, "{kept}");
    let read: Vec<Line> = kept
        .lines()
        .map(|line| serde_json::from_str(line).expect("one line is not one JSON object"))
        .collect();
    assert_eq!(read, lines);
}

#[test]
fn whoever_takes_a_draft_first_writes_its_line_and_the_second_writes_none() {
    let dir = Dir::new("take once");
    let file = dir.path().join(FILE);
    let flight = InFlight::new(&file);
    let turn = flight.open(draft());

    let line = flight
        .end(
            turn,
            Timestamp::from_secs(1_700_000_090),
            Ended::Done,
            Some(spent()),
        )
        .unwrap()
        .expect("the first take writes the line");
    assert_eq!(line.ended, Ended::Done);
    assert_eq!(line.usage, spent());

    assert!(
        flight
            .end(
                turn,
                Timestamp::from_secs(1_700_000_120),
                Ended::Failed,
                None
            )
            .unwrap()
            .is_none(),
        "the second take finds no draft open"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap().lines().count(),
        1,
        "one line for one turn"
    );
}

#[test]
fn stop_all_writes_a_stopped_line_for_each_draft_left_and_none_twice() {
    let dir = Dir::new("stop all");
    let file = dir.path().join(FILE);
    let flight = InFlight::new(&file);
    let ended = flight.open(draft_on("74"));
    let running = flight.open(draft_on("75"));
    flight
        .end(
            ended,
            Timestamp::from_secs(1_700_000_060),
            Ended::Done,
            Some(spent()),
        )
        .unwrap();

    let stopped = flight.stop_all(Timestamp::from_secs(1_700_000_090));
    assert_eq!(
        stopped
            .iter()
            .map(|line| (line.ticket.as_str(), line.ended, line.seconds))
            .collect::<Vec<_>>(),
        [("75", Ended::Stopped, 90)],
        "the turn still open gets a stopped line, the one already ended no second one"
    );
    assert!(
        flight
            .end(
                running,
                Timestamp::from_secs(1_700_000_150),
                Ended::Failed,
                None
            )
            .unwrap()
            .is_none(),
        "the turn the stop wrote is not written again by its own end"
    );
    assert!(
        flight
            .stop_all(Timestamp::from_secs(1_700_000_120))
            .is_empty(),
        "none twice"
    );
    assert_eq!(
        fs::read_to_string(&file).unwrap().lines().count(),
        2,
        "the ended turn's line and the stopped one"
    );
}

#[test]
fn a_ledger_that_cannot_be_written_writes_no_line_and_keeps_no_draft() {
    let dir = Dir::new("unwritable ledger");
    let blocked = dir.path().join("blocked");
    fs::write(&blocked, "a file where a folder should be").unwrap();
    let flight = InFlight::new(blocked.join(FILE));
    let turn = flight.open(draft());

    assert!(
        flight
            .end(
                turn,
                Timestamp::from_secs(1_700_000_090),
                Ended::Done,
                Some(spent()),
            )
            .is_err(),
        "the line cannot reach a file that stands where the project's folder should be"
    );
    assert!(
        flight
            .end(
                turn,
                Timestamp::from_secs(1_700_000_120),
                Ended::Stopped,
                None,
            )
            .unwrap()
            .is_none(),
        "the turn whose line failed is taken all the same, so it writes nothing later"
    );
    assert!(
        flight
            .stop_all(Timestamp::from_secs(1_700_000_150))
            .is_empty()
    );
}
