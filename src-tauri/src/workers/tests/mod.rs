//! Tests for the worker commands
//!
//! Each test opens the app's state in a folder of its own and reads tickets from
//! a [`FakeTickets`], so no test needs `gh`, a model or the network.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use kelpie_engine::login::Store;
use kelpie_engine::run::RunId;
use kelpie_engine::session::{Choice, Harness as SessionHarness, Session};
use kelpie_engine::settings::{ConfigDir, Field, LayerFile, Settings};
use kelpie_engine::ticket::fake::FakeTickets;
use kelpie_engine::ticket::{ForgeError, Issue, TicketId, Tickets};
use kelpie_engine::worker::{Record, Seat, Status};
use serde_json::{Value, json};

use super::{WorkerStarted, start_on, ticket_of};
use crate::state::AppState;

mod feed;
mod goose;
mod plan;

/// An app state in a folder of its own, with one project picked
struct Rig {
    dir: PathBuf,
    state: AppState,
    project: String,
}

impl Rig {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-workers-{}", RunId::new().unwrap()));
        let folder = dir.join("checkout");
        fs::create_dir_all(&folder).unwrap();
        let state = AppState::open(
            dir.clone(),
            ConfigDir::at(dir.join("config")),
            Store::memory(),
            Box::new(|_| Err("these tests run no PM".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        state.set_project(folder).unwrap();
        let project = state.project_id(None).unwrap();
        Self {
            dir,
            state,
            project,
        }
    }

    fn config(&self) -> &ConfigDir {
        self.state.config()
    }

    /// Set `field` in the project's own settings file
    fn set(&self, field: Field, value: Value) {
        Settings::set(
            self.config(),
            LayerFile::Project(&self.project),
            field,
            &value,
        )
        .unwrap();
    }

    /// What `start_on` does for `ticket` when the project's tickets are `tickets`
    ///
    /// No worker's session is ever built: the opener of these tests fails the
    /// test that reaches it.
    fn start(&self, tickets: &FakeTickets, ticket: &str) -> Result<WorkerStarted, String> {
        let source: &dyn Tickets = tickets;
        start_on(&self.state, &self.project, Ok(source), ticket, &|_, _| {
            panic!("no session should be built")
        })
    }

    /// The tickets the project's workers have a record of
    fn recorded(&self) -> Vec<String> {
        let records = self.state.workers().records(&self.project).unwrap();
        let records = records.lock().unwrap();
        records.list().iter().map(|r| r.ticket.clone()).collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The issue `number` as the source holds it
fn issue(title: &str, body: &str, open: bool) -> Result<Issue, ForgeError> {
    Ok(Issue {
        title: title.to_owned(),
        body: body.to_owned(),
        labels: Vec::new(),
        open,
    })
}

fn id(number: u64) -> TicketId {
    TicketId::from_issue(number).unwrap()
}

#[test]
fn a_ticket_is_sent_as_its_title_a_blank_line_then_its_body() {
    let tickets = FakeTickets::new();
    tickets.script_issue(
        id(12),
        issue("Add the thing", "It does one.\n\nAnd two.", true),
    );

    let ticket = ticket_of(&tickets, "12").unwrap();

    assert_eq!(ticket.id, "12");
    assert_eq!(ticket.text, "Add the thing\n\nIt does one.\n\nAnd two.");
}

#[test]
fn a_number_written_another_way_names_the_same_ticket() {
    let tickets = FakeTickets::new();
    tickets.script_issue(id(12), issue("Add the thing", "It does one.", true));

    for written in ["012", "+12", "#12", " 12 "] {
        let ticket = ticket_of(&tickets, written).unwrap();

        assert_eq!(
            ticket.id, "12",
            "{written:?} must not start a second worker"
        );
    }
}

#[test]
fn a_session_is_given_up_once_its_worker_holds_no_slot() {
    let rig = Rig::new();
    let session = Arc::new(Session::new(
        rig.state.tree(),
        SessionHarness::new("true"),
        rig.dir.join("checkout"),
        None,
        Choice::default(),
    ));
    let weak = Arc::downgrade(&session);
    {
        let records = rig.state.workers().records(&rig.project).unwrap();
        let mut records = records.lock().unwrap();
        let mut busy = Record::new("12", "kelpie/12");
        busy.status = Status::Running;
        records.keep(busy).unwrap();
        records.seat(1).unwrap();
    }
    rig.state.workers().keep(&rig.project, "12", session);
    assert!(rig.state.workers().session(&rig.project, "12").is_some());

    {
        let records = rig.state.workers().records(&rig.project).unwrap();
        let mut records = records.lock().unwrap();
        records.set_status("12", Status::Ended).unwrap();
        records.seat(1).unwrap();
    }

    assert!(rig.state.workers().session(&rig.project, "12").is_none());
    assert!(
        weak.upgrade().is_none(),
        "nothing keeps the ended worker's session alive"
    );
}

#[test]
fn a_closed_issue_starts_nothing_and_says_so() {
    let rig = Rig::new();
    let tickets = FakeTickets::new();
    tickets.script_issue(id(12), issue("Done already", "Shipped.", false));

    let said = rig.start(&tickets, "12").unwrap_err();

    assert!(said.contains("12") && said.contains("closed"), "{said}");
    assert!(rig.recorded().is_empty(), "no slot was held");
}

#[test]
fn a_number_naming_no_issue_starts_nothing_and_says_so() {
    let rig = Rig::new();
    let tickets = FakeTickets::new();

    let said = rig.start(&tickets, "99").unwrap_err();

    assert!(said.contains("99") && said.contains("no issue"), "{said}");
    assert!(rig.recorded().is_empty(), "no slot was held");
}

#[test]
fn a_project_whose_origin_is_not_github_starts_nothing_and_says_so() {
    // The checkout holds no git repo, so it names no origin
    let rig = Rig::new();
    let source = rig.state.source(&rig.project);
    let source = source.as_ref().map(|s| &**s as &dyn Tickets);

    let said = start_on(
        &rig.state,
        &rig.project,
        source.map_err(Clone::clone),
        "12",
        &|_, _| panic!("no session should be built"),
    )
    .unwrap_err();

    assert!(said.contains("GitHub"), "{said}");
    assert!(rig.recorded().is_empty(), "no slot was held");
}

#[test]
fn a_start_with_no_free_slot_answers_waiting() {
    let rig = Rig::new();
    rig.set(Field::WorkerSlots, json!(1));
    {
        let records = rig.state.workers().records(&rig.project).unwrap();
        let mut records = records.lock().unwrap();
        let mut busy = Record::new("7", "kelpie/7");
        busy.status = Status::Running;
        records.keep(busy).unwrap();
        records.seat(1).unwrap();
        assert_eq!(records.get("7").unwrap().seat, Seat::Held);
    }
    let tickets = FakeTickets::new();
    tickets.script_issue(id(8), issue("Next", "In line.", true));

    let started = rig.start(&tickets, "8").unwrap();

    assert_eq!(
        started,
        WorkerStarted {
            ticket: "8".to_owned(),
            waiting: true
        }
    );
    assert_eq!(
        rig.recorded(),
        ["7", "8"],
        "the ticket is in line for a slot"
    );
}

/// Run `git` in `folder`, as the checkout of a project would be made
pub(super) fn git(folder: &Path, args: &[&str]) {
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

/// What `git` answers for `folder`, trimmed
pub(super) fn git_out(folder: &Path, args: &[&str]) -> String {
    let done = Command::new("git")
        .current_dir(folder)
        .args(args)
        .output()
        .unwrap();
    assert!(
        done.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

impl Rig {
    /// Make the project's checkout a repo with an origin holding `main`
    pub(super) fn with_origin(&self) {
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
}
