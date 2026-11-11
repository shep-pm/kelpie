//! Tests for the board's poller and its moves
//!
//! Each test reads a [`Feed`] scripted by hand, so none needs `gh`, a browser or
//! the network, and the poller is stepped by calling [`Poller::read`] and
//! [`Poller::place`] where a run would wait on a clock.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use kelpie_engine::ci::HeadStatus;
use kelpie_engine::github::{Clock, ForgeSlug, Timestamp};
use kelpie_engine::run::RunId;
use kelpie_engine::settings::PickUp;
use kelpie_engine::ticket::fake::FakeTickets;
use kelpie_engine::ticket::{
    ForgeError, GithubTickets, Issue, Listed, OpenPull, TicketId, Tickets,
};
use kelpie_engine::worker::{Record, Records, Status};

use super::poll::{Cadence, Choices, Feed, Poller};
use super::{BoardEvent, BoardHost, BoardView, Column, MoveView, TicketView, move_ticket};
use crate::workers::{Feed as WorkerFeed, WorkerStarted};

mod link;
mod needs_you;

/// A project's source, scripted
///
/// `strict` fails the test on a second call to any list, so a test can say that
/// placing makes no call.
struct Source {
    tickets: FakeTickets,
    repo: ForgeSlug,
    viewer: Mutex<Result<String, ForgeError>>,
    lifts: Mutex<Option<Timestamp>>,
    calls: AtomicUsize,
    strict: bool,
}

impl Source {
    fn new() -> Self {
        Self {
            tickets: FakeTickets::new(),
            repo: ForgeSlug::try_from("shep-pm/kelpie".to_owned()).unwrap(),
            viewer: Mutex::new(Ok("octocat".to_owned())),
            lifts: Mutex::new(None),
            calls: AtomicUsize::new(0),
            strict: false,
        }
    }

    fn once() -> Self {
        Self {
            strict: true,
            ..Self::new()
        }
    }

    fn called(&self) {
        let before = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            !(self.strict && before >= 4),
            "the source was read a second time"
        );
    }
}

impl Tickets for Source {
    fn issue(&self, id: &TicketId) -> Result<Issue, ForgeError> {
        self.tickets.issue(id)
    }

    fn ready_issues(&self, label: &str) -> Result<Vec<Listed>, ForgeError> {
        self.called();
        self.tickets.ready_issues(label)
    }
}

impl GithubTickets for Source {
    fn open_issues(&self) -> Result<Vec<Listed>, ForgeError> {
        self.called();
        self.tickets.open_issues()
    }

    fn closed_issues(&self, since: Timestamp) -> Result<Vec<Listed>, ForgeError> {
        self.called();
        self.tickets.closed_issues(since)
    }

    fn open_pulls(&self) -> Result<Vec<OpenPull>, ForgeError> {
        self.called();
        self.tickets.open_pulls()
    }
}

impl Feed for Source {
    fn repo(&self) -> &ForgeSlug {
        &self.repo
    }

    fn viewer(&self) -> Result<String, ForgeError> {
        self.viewer.lock().unwrap().clone()
    }

    fn lifts_at(&self) -> Option<Timestamp> {
        *self.lifts.lock().unwrap()
    }
}

struct Fixed(u64);

impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::from_secs(self.0)
    }
}

/// The poller of a [`Source`], with records in a folder of its own
struct Rig {
    dir: PathBuf,
    source: Arc<Source>,
    records: Arc<Mutex<Records>>,
    /// The feed of each worker, by ticket, which the poller reads the waiting requests from
    feeds: Feeds,
    poller: Poller<Source>,
}

type Feeds = Arc<Mutex<BTreeMap<String, Arc<WorkerFeed>>>>;

impl Rig {
    fn new(source: Source) -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-board-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        let records = Arc::new(Mutex::new(Records::load(dir.join("workers.json")).unwrap()));
        let source = Arc::new(source);
        let feeds = Feeds::default();
        let poller = Self::poller(&source, &records, &feeds);
        Self {
            dir,
            source,
            records,
            feeds,
            poller,
        }
    }

    /// A poller of `source`, on `records`, with the project's default choices
    fn poller(
        source: &Arc<Source>,
        records: &Arc<Mutex<Records>>,
        feeds: &Feeds,
    ) -> Poller<Source> {
        let choose = Box::new(|| {
            Ok(Choices {
                pick_up: PickUp::Me,
                ready_label: "ready-for-agent".to_owned(),
                done_days: 14,
            })
        });
        let feeds = Arc::clone(feeds);
        let requests = Box::new(move || {
            feeds
                .lock()
                .unwrap()
                .iter()
                .map(|(ticket, feed)| (ticket.clone(), feed.waiting()))
                .collect()
        });
        // These tests read the board; the hand-overs a read owes the forge are
        // `workers::tests::hand_over`'s to test.
        let retry = Box::new(|_: &[Record]| {});
        Poller::new(
            Arc::clone(source),
            choose,
            Arc::clone(records),
            requests,
            retry,
            Arc::new(Fixed(1_700_000_000)),
        )
    }

    /// A worker on `ticket` that holds a slot
    fn work_on(&self, ticket: &str) {
        let mut record = Record::new(ticket, format!("kelpie/{ticket}"));
        record.status = Status::Running;
        self.records.lock().unwrap().keep(record).unwrap();
    }

    /// The feed of the worker on `ticket`, made on the first ask
    fn feed(&self, ticket: &str) -> Arc<WorkerFeed> {
        let mut feeds = self.feeds.lock().unwrap();
        Arc::clone(
            feeds
                .entry(ticket.to_owned())
                .or_insert_with(|| Arc::new(WorkerFeed::new(0))),
        )
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn id(number: u64) -> TicketId {
    TicketId::from_issue(number).unwrap()
}

/// An issue of `octocat`'s, opened at second `created`
fn listed(number: u64, title: &str, labels: &[&str], created: u64) -> Listed {
    Listed {
        id: id(number),
        title: title.to_owned(),
        labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        author: "octocat".to_owned(),
        created: Timestamp::from_secs(created),
    }
}

/// The tickets of a board event, as (id, column)
fn placed(event: &BoardEvent) -> Vec<(String, Column)> {
    let BoardEvent::Board(view) = event else {
        panic!("expected a board, got {event:?}");
    };
    view.tickets
        .iter()
        .map(|ticket| (ticket.id.clone(), ticket.column))
        .collect()
}

#[test]
fn a_read_becomes_a_board_view_with_each_tickets_url() {
    let mut rig = Rig::new(Source::new());
    let ready = listed(
        12,
        "Add the thing",
        &["ready-for-agent", "priority: P1"],
        100,
    );
    rig.source.tickets.script_ready(Ok(vec![ready.clone()]));
    rig.source.tickets.script_open(Ok(vec![
        ready,
        listed(13, "Think about it", &[], 200),
        listed(14, "Reviewing", &[], 300),
    ]));
    rig.source.tickets.script_pulls(Ok(vec![OpenPull {
        number: std::num::NonZeroU64::new(99).unwrap(),
        branch: "kelpie/14".to_owned(),
        draft: true,
        closes: vec![id(14)],
        head: "0".repeat(40),
        base: "main".to_owned(),
        labels: Vec::new(),
        checks: HeadStatus::None,
    }]));

    let event = rig.poller.read().unwrap();

    assert_eq!(
        event,
        BoardEvent::Board(BoardView {
            tickets: vec![
                TicketView {
                    id: "13".to_owned(),
                    title: "Think about it".to_owned(),
                    priority: None,
                    column: Column::Draft,
                    url: "https://github.com/shep-pm/kelpie/issues/13".to_owned(),
                    pull_url: None,
                    needs_you: Vec::new(),
                },
                TicketView {
                    id: "12".to_owned(),
                    title: "Add the thing".to_owned(),
                    priority: Some(1),
                    column: Column::Ready,
                    url: "https://github.com/shep-pm/kelpie/issues/12".to_owned(),
                    pull_url: None,
                    needs_you: Vec::new(),
                },
                TicketView {
                    id: "14".to_owned(),
                    title: "Reviewing".to_owned(),
                    priority: None,
                    column: Column::Review,
                    url: "https://github.com/shep-pm/kelpie/issues/14".to_owned(),
                    pull_url: Some("https://github.com/shep-pm/kelpie/pull/99".to_owned()),
                    needs_you: Vec::new(),
                },
            ],
            held_until: None,
            needs_you: 0,
        })
    );
}

/// A source that lists ticket 12 as Ready, and fails the test if it is read twice
fn one_ready_ticket() -> Rig {
    let rig = Rig::new(Source::once());
    let ready = listed(12, "Add the thing", &["ready-for-agent"], 100);
    rig.source.tickets.script_ready(Ok(vec![ready.clone()]));
    rig.source.tickets.script_open(Ok(vec![ready]));
    rig
}

#[test]
fn a_changed_record_moves_a_ticket_without_a_new_read() {
    let mut rig = one_ready_ticket();
    let first = rig.poller.read().unwrap();
    assert_eq!(placed(&first), [("12".to_owned(), Column::Ready)]);

    rig.work_on("12");
    let moved = rig.poller.place().unwrap();

    assert_eq!(placed(&moved), [("12".to_owned(), Column::Building)]);
    assert_eq!(
        rig.source.calls.load(Ordering::SeqCst),
        4,
        "one read of each list, and placing made no call"
    );
}

#[test]
fn a_board_that_did_not_change_sends_nothing() {
    let mut rig = one_ready_ticket();
    rig.poller.read().unwrap();

    assert_eq!(rig.poller.place(), None);
    assert_eq!(rig.poller.place(), None);
    rig.work_on("12");
    assert!(rig.poller.place().is_some());
    assert_eq!(rig.poller.place(), None);
}

#[test]
fn a_held_limit_sends_the_last_board_with_the_time_it_lifts() {
    let mut rig = Rig::new(Source::new());
    let ready = listed(12, "Add the thing", &["ready-for-agent"], 100);
    rig.source.tickets.script_ready(Ok(vec![ready.clone()]));
    rig.source.tickets.script_open(Ok(vec![ready]));
    rig.poller.read().unwrap();

    // The hold stands: the next read is refused without a call
    rig.source
        .tickets
        .script_open(Err(ForgeError::Held(Timestamp::from_secs(1_700_000_500))));
    let held = rig.poller.read().unwrap();

    let BoardEvent::Board(view) = held else {
        panic!("expected a board, got {held:?}");
    };
    assert_eq!(view.held_until, Some(1_700_000_500));
    assert_eq!(view.tickets.len(), 1, "the board it has is still shown");

    // The call that used the limit up carries no time, so the source is asked
    let mut rig = Rig::new(Source::new());
    rig.source.tickets.script_open(Err(ForgeError::Limited {
        said: "gh failed: API rate limit exceeded".to_owned(),
        cooldown: kelpie_engine::github::Cooldown::default(),
    }));
    *rig.source.lifts.lock().unwrap() = Some(Timestamp::from_secs(1_700_000_900));
    let held = rig.poller.read().unwrap();
    let BoardEvent::Board(view) = held else {
        panic!("expected a board, got {held:?}");
    };
    assert_eq!(view.held_until, Some(1_700_000_900));
    assert!(
        view.tickets.is_empty(),
        "no board was read before the limit"
    );
}

#[test]
fn a_gh_with_no_login_sends_unavailable_saying_to_sign_in() {
    let mut rig = Rig::new(Source::new());
    *rig.source.viewer.lock().unwrap() = Err(ForgeError::Failed(
        "gh: To get started with GitHub CLI, please run:  gh auth login".to_owned(),
    ));

    let event = rig.poller.read().unwrap();

    let BoardEvent::Unavailable { message } = event else {
        panic!("expected unavailable, got {event:?}");
    };
    assert!(message.contains("gh auth login"), "{message}");

    // The same answer again changes nothing on the window
    assert_eq!(rig.poller.read(), None);

    // The login is read again after an Unavailable, so signing in brings the board
    *rig.source.viewer.lock().unwrap() = Ok("octocat".to_owned());
    rig.source
        .tickets
        .script_open(Ok(vec![listed(13, "Think", &[], 200)]));
    let event = rig.poller.read().unwrap();
    assert_eq!(placed(&event), [("13".to_owned(), Column::Draft)]);
}

#[test]
fn a_login_switched_while_watching_picks_up_the_new_logins_tickets() {
    let mut rig = Rig::new(Source::new());
    rig.source
        .tickets
        .script_open(Ok(vec![listed(13, "Think", &[], 200)]));
    let event = rig.poller.read().unwrap();
    assert_eq!(placed(&event), [("13".to_owned(), Column::Draft)]);

    // `gh auth switch` to an account that opened none of them
    *rig.source.viewer.lock().unwrap() = Ok("someone-else".to_owned());
    let event = rig.poller.read().unwrap();

    assert_eq!(placed(&event), []);
}

#[test]
fn a_missing_gh_sends_unavailable_saying_to_install_it() {
    let mut rig = Rig::new(Source::new());
    *rig.source.viewer.lock().unwrap() = Err(ForgeError::Spawn("no such file".to_owned()));

    let BoardEvent::Unavailable { message } = rig.poller.read().unwrap() else {
        panic!("expected unavailable");
    };

    assert!(message.contains("install gh"), "{message}");
}

#[test]
fn a_closed_channel_stops_the_poller() {
    let rig = one_ready_ticket();
    let (_keep, commands) = std::sync::mpsc::channel();
    let (done, finished) = std::sync::mpsc::channel();
    let sent = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&sent);
    std::thread::spawn(move || {
        let mut rig = rig;
        super::poll::run(
            &mut rig.poller,
            &commands,
            super::poll::Cadence {
                place: std::time::Duration::from_millis(5),
                read: std::time::Duration::from_secs(3600),
            },
            &mut |_| {
                count.fetch_add(1, Ordering::SeqCst);
                // the channel is closed: the send fails
                false
            },
        );
        done.send(()).unwrap();
    });

    finished
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the poller kept running after its channel closed");
    assert_eq!(sent.load(Ordering::SeqCst), 1, "it sent once, and stopped");
}

/// A host watching a board of 12 Ready and 13 Draft, once its first board is out
fn watching(rig: &Rig) -> BoardHost {
    rig.source.tickets.script_ready(Ok(vec![listed(
        12,
        "Add the thing",
        &["ready-for-agent"],
        100,
    )]));
    rig.source.tickets.script_open(Ok(vec![
        listed(12, "Add the thing", &["ready-for-agent"], 100),
        listed(13, "Think about it", &[], 200),
    ]));
    let host = BoardHost::default();
    let (sent, first) = std::sync::mpsc::channel();
    host.watch(
        Rig::poller(&rig.source, &rig.records, &rig.feeds),
        Cadence {
            place: std::time::Duration::from_millis(5),
            read: std::time::Duration::from_secs(3600),
        },
        move |event| sent.send(event).is_ok(),
    );
    first
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("the host sent no board");
    host
}

#[test]
fn a_move_from_ready_to_building_starts_the_worker_and_any_other_move_changes_nothing() {
    let rig = Rig::new(Source::new());
    let host = watching(&rig);
    let started = Mutex::new(Vec::new());
    let start = |ticket: &str| {
        started.lock().unwrap().push(ticket.to_owned());
        Ok(WorkerStarted {
            ticket: ticket.to_owned(),
            waiting: true,
        })
    };

    let moved = move_ticket(&host, "12", Column::Building, start);
    assert_eq!(moved, Ok(MoveView::Started { waiting: true }));
    assert_eq!(*started.lock().unwrap(), ["12"]);

    let stay = move_ticket(&host, "12", Column::Ready, start);
    assert_eq!(stay, Ok(MoveView::Stay));

    let ready = move_ticket(&host, "13", Column::Ready, start);
    let Ok(MoveView::SnapBack { line }) = ready else {
        panic!("expected a snap back, got {ready:?}");
    };
    assert!(line.contains("ready label"), "{line}");

    let merge = move_ticket(&host, "12", Column::Done, start);
    assert!(matches!(merge, Ok(MoveView::SnapBack { .. })), "{merge:?}");

    assert_eq!(
        *started.lock().unwrap(),
        ["12"],
        "only the first move started a worker"
    );
}

#[test]
fn a_move_of_a_ticket_not_on_the_board_is_refused() {
    let rig = Rig::new(Source::new());
    let host = watching(&rig);

    let refused = move_ticket(&host, "99", Column::Building, |_| {
        panic!("no worker should start")
    });

    let said = refused.unwrap_err();
    assert!(
        said.contains("99") && said.contains("not on the board"),
        "{said}"
    );
}

#[test]
fn a_stopped_host_no_longer_answers_for_the_board_it_watched() {
    let rig = Rig::new(Source::new());
    let host = watching(&rig);
    assert_eq!(host.column_of("12"), Ok(Column::Ready));

    host.stop();

    // A drop must not be checked against the columns of a project left behind
    let said = host.column_of("12").unwrap_err();
    assert!(said.contains("not been read"), "{said}");
}
