//! A worker whose sandbox bwrap refuses waits Paused, and the start says why
//!
//! No unit test runs bwrap, so each test stands in for the probe of the sandbox
//! and runs the app's own [`confine`], which names a refusal to the engine's
//! start (`docs/design/sandbox-setup.md`, Running without a sandbox).

use kelpie_engine::sandbox::{Refusal, SandboxError};
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::settings::Field;
use kelpie_engine::ticket::Tickets;
use kelpie_engine::ticket::fake::FakeTickets;
use kelpie_engine::worker::{Record, Seat, Status};
use serde_json::json;

use super::{Rig, id, issue};
use crate::workers::open::{confine, open_with};
use crate::workers::{WorkerStarted, resume_on, start_on};

/// What the probe of the worker's sandbox finds
type Probe = fn() -> Result<(), SandboxError>;

/// What bwrap does on Ubuntu 24.04 without kelpie's AppArmor profile
fn refuses() -> Result<(), SandboxError> {
    Err(SandboxError::Refused(Refusal::NoCapabilities))
}

/// What bwrap does once the profile is loaded
fn allows() -> Result<(), SandboxError> {
    Ok(())
}

/// The words a refused worker's record keeps
fn why() -> String {
    format!(
        "its sandbox cannot start because {}",
        Refusal::NoCapabilities
    )
}

impl Rig {
    /// A project that runs its workers on goose, with an origin holding `main`
    fn goose_project(&self) {
        self.with_origin();
        self.set(Field::WorkerHarness, json!("goose"));
        self.set(Field::Provider, json!("ollama"));
        self.set(Field::Host, json!("http://127.0.0.1:11434"));
    }

    /// What `start_on` does for ticket 12, with `probe` as the sandbox's probe
    fn start_probed(&self, agent: &FakeAgent, probe: Probe) -> Result<WorkerStarted, String> {
        let tickets = FakeTickets::new();
        tickets.script_issue(id(12), issue("Work on goose", "One turn.", true));
        let source: &dyn Tickets = &tickets;
        start_on(
            &self.state,
            &self.project,
            Ok(source),
            "12",
            &|on, opening| goose(agent, probe, on, opening),
        )
    }

    /// The record of ticket 12
    fn record_of_12(&self) -> Option<Record> {
        let records = self.state.workers().records(&self.project).unwrap();
        let records = records.lock().unwrap();
        records.get("12").cloned()
    }
}

/// The goose arm, with the sandbox's probe standing in for bwrap
fn goose(
    agent: &FakeAgent,
    probe: Probe,
    on: &crate::workers::OpenOn<'_>,
    opening: &kelpie_engine::worker::Opening<'_>,
) -> Result<kelpie_engine::session::Session, String> {
    open_with(
        on,
        opening,
        |_, _, _| Ok(agent.harness()),
        |opening, _harness| {
            confine(opening, Ok(()), |()| probe())?;
            Ok(agent.session(
                opening.worktree.to_path_buf(),
                opening.resume.clone(),
                opening.choice.clone(),
            ))
        },
    )
}

#[test]
fn a_goose_worker_whose_sandbox_is_refused_waits_paused_and_the_start_says_why() {
    let rig = Rig::new();
    rig.goose_project();
    let agent = FakeAgent::new().turn(Turn::new().text("never runs"));

    let said = rig
        .start_probed(&agent, refuses)
        .expect_err("a refused sandbox starts no worker");

    assert!(
        said.contains(&Refusal::NoCapabilities.to_string()),
        "{said}"
    );
    assert!(said.contains("Resume"), "{said}");
    let record = rig
        .record_of_12()
        .expect("the refused ticket keeps its record");
    assert_eq!(record.status, Status::Paused);
    assert_eq!(record.seat, Seat::Without);
    assert_eq!(record.reason.as_deref(), Some(why().as_str()));
    assert!(agent.calls().is_empty(), "no harness was started");
}

#[test]
fn a_goose_worker_whose_probe_fails_for_another_reason_gives_its_slot_back() {
    let rig = Rig::new();
    rig.goose_project();
    let agent = FakeAgent::new().turn(Turn::new().text("never runs"));

    let said = rig
        .start_probed(&agent, || {
            Err(SandboxError::Probe("no bwrap on PATH".to_owned()))
        })
        .expect_err("a probe that fails starts no worker");

    assert!(
        said.contains("could not start the worker's sandbox"),
        "{said}"
    );
    assert!(rig.recorded().is_empty(), "the failed start holds no slot");
}

#[test]
fn a_refused_worker_resumes_through_resume_on_once_its_sandbox_can_start() {
    let rig = Rig::new();
    rig.goose_project();
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    rig.start_probed(&agent, refuses)
        .expect_err("the sandbox is refused");

    let resumed = resume_on(&rig.state, &rig.project, "12", &|on, opening| {
        goose(&agent, allows, on, opening)
    })
    .expect("the sandbox can start now");

    assert!(!resumed.waiting);
    let record = rig.record_of_12().unwrap();
    assert_eq!(record.status, Status::Running);
    assert_eq!(record.seat, Seat::Held);
    assert_eq!(record.reason, None);
}
