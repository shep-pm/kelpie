//! Tests for a worker's feed: its past from the log, its live events, and Resume
//!
//! A stand-in agent plays the worker's turns and its session writes a log in a
//! folder of the test's own, so no test needs a model.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::session::log::Log;
use kelpie_engine::session::{Choice, Session};
use kelpie_engine::worker::{Paths, Record, Status};
use kelpie_protocol::{LogEntry, SessionEvent};

use super::Rig;
use crate::workers::feed::Feed;
use crate::workers::resume_on;

const WAIT: Duration = Duration::from_secs(20);

fn text(delta: &str) -> SessionEvent {
    SessionEvent::Text {
        delta: delta.to_owned(),
    }
}

/// A watcher that keeps what it hears, and the receiving end
fn listener() -> (
    impl Fn(SessionEvent) -> bool + Send + 'static,
    Receiver<SessionEvent>,
) {
    let (sender, receiver) = channel();
    (move |event| sender.send(event).is_ok(), receiver)
}

/// Every event up to and including the turn's end
fn until_turn_ends(heard: &Receiver<SessionEvent>) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    loop {
        let event = heard
            .recv_timeout(WAIT)
            .expect("the turn sent no more events");
        let ends = matches!(event, SessionEvent::TurnEnded { .. });
        seen.push(event);
        if ends {
            return seen;
        }
    }
}

impl Rig {
    /// A worker on `ticket` with a record, Paused when `status` says so
    fn worker(&self, ticket: &str, status: Status) {
        let records = self.state.workers().records(&self.project).unwrap();
        let mut record = Record::new(ticket, format!("kelpie/{ticket}"));
        record.status = status;
        records.lock().unwrap().keep(record).unwrap();
    }

    fn log_file(&self, ticket: &str) -> std::path::PathBuf {
        Paths::new(self.state.data_dir(), &self.project).log_file(ticket)
    }

    /// A session of `agent` that logs to the worker's log and forwards into its feed
    fn run_turn(&self, agent: &FakeAgent, ticket: &str, message: &str) -> Vec<SessionEvent> {
        let session = agent.session(self.dir.join("checkout"), None, Choice::default());
        let session: Session = session.with_log(Log::open(self.log_file(ticket)).unwrap());
        let feed = self.state.workers().feed(&self.project, ticket);
        let (turn, heard) = channel();
        session.send(message, move |event| {
            feed.forward(event.clone());
            let _ = turn.send(event);
        });
        until_turn_ends(&heard)
    }
}

#[test]
fn a_watcher_gets_every_event_forwarded_after_it_attaches() {
    let feed = Feed::new();
    let (send, heard) = listener();
    feed.watch(|| Ok::<_, ()>(()), send).unwrap();

    feed.forward(text("one"));
    feed.forward(text("two"));

    assert_eq!(heard.try_recv().unwrap(), text("one"));
    assert_eq!(heard.try_recv().unwrap(), text("two"));
    assert!(heard.try_recv().is_err());
}

#[test]
fn a_channel_whose_send_fails_is_dropped_and_the_others_keep_receiving() {
    let feed = Feed::new();
    let tried = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&tried);
    feed.watch(
        || Ok::<_, ()>(()),
        move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            false
        },
    )
    .unwrap();
    let (send, heard) = listener();
    feed.watch(|| Ok::<_, ()>(()), send).unwrap();

    feed.forward(text("one"));
    feed.forward(text("two"));

    assert_eq!(
        tried.load(Ordering::SeqCst),
        1,
        "a failed channel is not tried again"
    );
    assert_eq!(heard.try_recv().unwrap(), text("one"));
    assert_eq!(heard.try_recv().unwrap(), text("two"));
}

#[test]
fn an_event_forwarded_while_the_entries_are_read_reaches_the_channel_after_it_attaches() {
    let feed = Arc::new(Feed::new());
    let forwarding = Arc::clone(&feed);
    let (send, heard) = listener();

    let (forwarder, entries) = feed
        .watch(
            || {
                let forwarder = std::thread::spawn(move || forwarding.forward(text("late")));
                // Time for the forwarder to reach the lock the read holds
                std::thread::sleep(Duration::from_millis(100));
                Ok::<_, ()>((forwarder, vec!["entry"]))
            },
            send,
        )
        .unwrap();
    forwarder.join().unwrap();

    assert_eq!(entries, ["entry"]);
    assert_eq!(heard.try_recv().unwrap(), text("late"));
}

#[test]
fn the_entries_returned_are_the_logs_and_an_event_is_never_in_both_places() {
    let rig = Rig::new();
    rig.worker("12", Status::Running);
    let agent = FakeAgent::new()
        .turn(Turn::new().text("first"))
        .turn(Turn::new().text("second"));
    let past = rig.run_turn(&agent, "12", "Begin.");
    let (send, heard) = listener();

    let entries = rig.state.workers().watch(&rig.project, "12", send).unwrap();
    let live = rig.run_turn(&agent, "12", "Go on.");

    let mut expected = vec![LogEntry::User {
        text: "Begin.".to_owned(),
    }];
    expected.extend(past.into_iter().map(LogEntry::Event));
    assert_eq!(
        entries, expected,
        "the log holds the first turn and nothing of the second"
    );
    let received: Vec<_> = std::iter::from_fn(|| heard.try_recv().ok()).collect();
    assert_eq!(
        received, live,
        "the channel holds the second turn and nothing of the first"
    );
}

#[test]
fn a_worker_with_no_log_yet_returns_no_entries() {
    let rig = Rig::new();
    rig.worker("12", Status::Running);
    let (send, _heard) = listener();

    let entries = rig.state.workers().watch(&rig.project, "12", send).unwrap();

    assert!(entries.is_empty());
}

#[test]
fn watching_a_ticket_with_no_worker_is_refused_by_name() {
    let rig = Rig::new();
    let (send, _heard) = listener();

    let said = rig
        .state
        .workers()
        .watch(&rig.project, "12", send)
        .unwrap_err();

    assert!(said.contains("12") && said.contains("no worker"), "{said}");
}

/// Run `git` in `folder`, as the checkout of a project would be made
fn git(folder: &Path, args: &[&str]) {
    let done = Command::new("git")
        .current_dir(folder)
        .args([
            "-c",
            "user.name=kelpie",
            "-c",
            "user.email=kelpie@example.com",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        done.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&done.stderr)
    );
}

impl Rig {
    /// Make the project's checkout a repo with an origin holding `main`
    fn with_origin(&self) {
        let checkout = self.dir.join("checkout");
        git(
            &self.dir,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&checkout, &["init", "--quiet", "-b", "main"]);
        git(&checkout, &["remote", "add", "origin", "../origin.git"]);
        git(
            &checkout,
            &["commit", "--quiet", "--allow-empty", "-m", "init"],
        );
        git(&checkout, &["push", "--quiet", "origin", "main"]);
    }

    /// What `resume_on` does for `ticket`, with `agent` as the worker's harness
    fn resume(&self, agent: &FakeAgent, ticket: &str) -> Result<super::WorkerStarted, String> {
        resume_on(&self.state, &self.project, ticket, &|_, opening| {
            Ok(agent.session(
                opening.worktree.to_path_buf(),
                opening.resume.clone(),
                Choice::default(),
            ))
        })
    }
}

#[test]
fn resume_refuses_a_worker_that_is_not_paused_and_names_its_state() {
    let rig = Rig::new();
    let agent = FakeAgent::new();
    for (ticket, status, doing) in [
        ("1", Status::Running, "running"),
        (
            "2",
            Status::WaitingOnMaintainer,
            "waiting on the maintainer",
        ),
        ("3", Status::Ended, "ended"),
    ] {
        rig.worker(ticket, status);

        let said = rig.resume(&agent, ticket).unwrap_err();

        assert!(said.contains(ticket) && said.contains(doing), "{said}");
    }
    let said = rig.resume(&agent, "4").unwrap_err();
    assert!(said.contains('4') && said.contains("no worker"), "{said}");
    assert!(agent.calls().is_empty(), "no session was started");
}

#[test]
fn resume_sends_the_carry_on_text_to_a_paused_worker() {
    let rig = Rig::new();
    rig.with_origin();
    rig.worker("12", Status::Paused);
    let agent = FakeAgent::new().turn(Turn::new().text("Carrying on."));
    let (send, heard) = listener();
    rig.state.workers().watch(&rig.project, "12", send).unwrap();

    let started = rig.resume(&agent, "12").unwrap();

    assert_eq!(
        started,
        super::WorkerStarted {
            ticket: "12".to_owned(),
            waiting: false
        }
    );
    let live = until_turn_ends(&heard);
    assert!(live.contains(&text("Carrying on.")), "{live:?}");
    let (send, _heard) = listener();
    let entries = rig.state.workers().watch(&rig.project, "12", send).unwrap();
    assert_eq!(
        entries.first(),
        Some(&LogEntry::User {
            text: "Carry on from where you stopped.".to_owned()
        })
    );
}
