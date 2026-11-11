//! The worker on goose: the harness its settings name, in the worktree the start gives it
//!
//! `goose::harness` searches `PATH` for the person's own goose, and the goose
//! arm's session step starts crabpot around the harness it builds. A unit test
//! has neither, as in the engine's own worker tests, so each test stands in for
//! one of the two and reads what the arm built. The `start_on` tests stand in
//! for the session step and its sandbox; the last keeps the arm's own session
//! step and stands in for the sandbox alone.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kelpie_engine::harness::{HarnessError, goose};
use kelpie_engine::lease::SOCKET_VAR;
use kelpie_engine::login::Store;
use kelpie_engine::sandbox::{Sandbox, SandboxError};
use kelpie_engine::session::Choice;
use kelpie_engine::session::Harness as SessionHarness;
use kelpie_engine::session::Session;
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::settings::Field;
use kelpie_engine::ticket::Tickets;
use kelpie_engine::ticket::fake::FakeTickets;
use kelpie_engine::worker::{Opening, Paths, Refused};
use kelpie_protocol::SessionEvent;
use serde_json::json;

use super::{Rig, git_out, id, issue};
use crate::workers::open::{goose_session_with, open_with};
use crate::workers::{OpenOn, WorkerStarted, lock, plan, start_on};

/// How long a test waits for a worker's turn to end
const WAIT: Duration = Duration::from_secs(20);

/// The feed watch id these tests attach under
const WATCH: u32 = 1;

/// What the goose arm handed each step a unit test stands in for
#[derive(Debug, Default)]
struct Seen {
    /// What it handed `goose::harness`
    asks: Mutex<Vec<Ask>>,

    /// What it handed the step that starts the worker's sandbox and opens its session
    opens: Mutex<Vec<Open>>,
}

/// What the goose arm handed `goose::harness`
#[derive(Debug, Clone)]
struct Ask {
    state_dir: PathBuf,
    provider: goose::Provider,
    harness_global_folder: bool,
}

/// What the goose arm handed the session step, and what the start handed the opener
#[derive(Debug, Clone)]
struct Open {
    harness: SessionHarness,
    worktree: PathBuf,
    env: Vec<(OsString, OsString)>,
    leased: bool,
}

/// Every event a worker's feed carried
type Heard = Arc<Mutex<Vec<SessionEvent>>>;

/// The goose arm's `build` step, stood in for: record the provider kelpie asked
/// for, and hand back `agent`
fn found(
    agent: &FakeAgent,
    asks: &Mutex<Vec<Ask>>,
) -> impl Fn(&Path, goose::Provider, bool) -> Result<SessionHarness, HarnessError> + Send + Sync {
    let agent = agent.clone();
    move |state_dir, provider, harness_global_folder| {
        lock(asks).push(Ask {
            state_dir: state_dir.to_path_buf(),
            provider,
            harness_global_folder,
        });
        Ok(agent.harness())
    }
}

impl Rig {
    /// What `start_on` does for `ticket` on goose, with the goose arm's two real steps stood in for
    ///
    /// `agent` stands in for `goose acp`: the arm hands it the provider the
    /// settings name, and the session step opens a session on `agent` in the
    /// worktree the start prepared. `seen` holds what each step was handed, and
    /// `heard` every event of the turn, which is why the feed is attached from
    /// inside the opener: the turn is in flight as soon as the session exists.
    fn on_goose(
        &self,
        agent: &FakeAgent,
        seen: &Seen,
        heard: &Heard,
        tickets: &FakeTickets,
        ticket: &str,
    ) -> Result<WorkerStarted, String> {
        let (state, project) = (&self.state, &self.project);
        let (agent, seen) = (agent.clone(), seen);
        let heard = Arc::clone(heard);
        let source: &dyn Tickets = tickets;
        let opener = &|on: &OpenOn<'_>, opening: &Opening<'_>| {
            open_with(
                on,
                opening,
                found(&agent, &seen.asks),
                |opening, harness| {
                    let send = {
                        let heard = Arc::clone(&heard);
                        move |event: SessionEvent| {
                            lock(&heard).push(event);
                            true
                        }
                    };
                    state
                        .workers()
                        .watch(project, ticket, WATCH, send)
                        .expect("the worker has a record to watch");
                    lock(&seen.opens).push(Open {
                        harness,
                        worktree: opening.worktree.to_path_buf(),
                        env: opening
                            .env
                            .iter()
                            .map(|(key, value)| (key.into(), value.into()))
                            .collect(),
                        leased: opening.lease.is_some(),
                    });
                    Ok(agent.session(
                        opening.worktree.to_path_buf(),
                        opening.resume.clone(),
                        opening.choice.clone(),
                    ))
                },
            )
        };
        start_on(state, project, Ok(source), ticket, opener)
    }
}

/// What the goose arm answers for a worker whose `Opening` the test builds, with
/// `sandbox` standing in for its sandbox start
///
/// The engine's own `Opening::sandbox` is [`Sandbox::start_worker`], which no
/// unit test can start, so the test hands in its own and reads the harness the
/// arm asks it to confine.
fn opened(
    rig: &Rig,
    agent: &FakeAgent,
    seen: &Seen,
    sandbox: &dyn Fn(&SessionHarness) -> Result<Sandbox, SandboxError>,
) -> Result<Session, String> {
    let plan = plan(rig.config(), &rig.project).unwrap();
    let logins = Store::memory();
    let checkout = rig.dir.join("checkout");
    let on = OpenOn {
        data_dir: rig.state.data_dir(),
        logins: &logins,
        config: rig.config(),
        project_id: &rig.project,
        folder: &checkout,
        plan: &plan,
    };
    let tree = rig.state.tree();
    let worktree = rig.dir.join("worktree");
    let state_dir = rig.dir.join("state");
    let opening = Opening {
        tree: &tree,
        binding: &plan.binding,
        choice: Choice::default(),
        worktree: &worktree,
        state_dir: &state_dir,
        resume: None,
        tools: None,
        env: vec![("CARGO_TARGET_DIR".to_owned(), "/build".to_owned())],
        lease: None,
        sandbox,
        refused: Refused::default(),
    };
    // No unit test runs bwrap, so the probe of a sandbox that started passes
    open_with(
        &on,
        &opening,
        found(agent, &seen.asks),
        |opening, harness| goose_session_with(opening, harness, |_| Ok(())),
    )
}

/// The tickets holding one open issue `12`
fn tickets() -> FakeTickets {
    let tickets = FakeTickets::new();
    tickets.script_issue(id(12), issue("Work on goose", "One turn.", true));
    tickets
}

/// Wait for the worker's turn to end, and everything its feed carried
fn until_turn_ends(heard: &Heard) -> Vec<SessionEvent> {
    let since = Instant::now();
    loop {
        let events = lock(heard).clone();
        if events
            .iter()
            .any(|event| matches!(event, SessionEvent::TurnEnded { .. }))
        {
            return events;
        }
        assert!(
            since.elapsed() < WAIT,
            "the worker's turn never ended: {events:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_worker_on_goose_starts_in_its_worktree_inside_the_worker_sandbox() {
    let rig = Rig::new();
    rig.with_origin();
    rig.set(Field::WorkerHarness, json!("goose"));
    rig.set(Field::Provider, json!("ollama"));
    rig.set(Field::Host, json!("http://127.0.0.1:11434"));

    let agent = FakeAgent::new().turn(Turn::new().text("Working in the worktree."));
    let seen = Seen::default();
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));

    let started = rig
        .on_goose(&agent, &seen, &heard, &tickets(), "12")
        .unwrap();
    let events = until_turn_ends(&heard);

    assert_eq!(
        started,
        WorkerStarted {
            ticket: "12".to_owned(),
            waiting: false
        }
    );
    assert!(
        events.contains(&SessionEvent::Text {
            delta: "Working in the worktree.".to_owned()
        }),
        "{events:?}"
    );

    // goose is asked to run on the provider the project's settings name, in the
    // worker's own state folder
    let asked = seen.asks.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "the settings named goose once");
    assert_eq!(
        asked[0].provider,
        goose::Provider {
            name: "ollama".to_owned(),
            host: Some("http://127.0.0.1:11434".to_owned()),
        },
        "the same provider the PM's goose runs on"
    );
    let paths = Paths::new(rig.dir.clone(), rig.project.clone());
    assert_eq!(
        asked[0].state_dir,
        paths.state_dir("12"),
        "goose keeps its config in this worker's state folder"
    );
    assert!(
        !asked[0].harness_global_folder,
        "the person's own goose config stays out until they turn it on"
    );

    // the harness the worker's sandbox is started around carries the variables
    // and the lease socket the start gave the opener
    let opened = seen.opens.lock().unwrap().clone();
    let open = opened
        .first()
        .expect("the goose arm opened one session")
        .clone();
    for (key, value) in &open.env {
        assert!(
            open.harness.env.iter().any(|(k, v)| k == key && v == value),
            "{key:?} did not reach goose"
        );
    }
    assert!(
        open.env.iter().any(|(key, _)| key == "CARGO_TARGET_DIR"),
        "the start gave the worker its own build folder"
    );
    assert_eq!(
        open.harness.env.iter().any(|(key, _)| key == SOCKET_VAR),
        open.leased,
        "the lease socket reaches goose wherever the worker has one"
    );

    // and the session runs in the ticket's worktree, not in the project's checkout
    assert_eq!(open.worktree, paths.worktree("12"));
    assert_ne!(open.worktree, rig.dir.join("checkout"));
    assert_eq!(
        git_out(&open.worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "kelpie/12",
        "the worktree is on the ticket's branch"
    );
    assert_eq!(
        agent
            .params("session/new")
            .first()
            .map(|params| params["cwd"].clone()),
        Some(json!(open.worktree)),
        "goose was told its working folder is the worktree"
    );
}

#[test]
fn a_worker_on_goose_with_no_provider_gives_its_slot_back() {
    let rig = Rig::new();
    rig.with_origin();
    rig.set(Field::WorkerHarness, json!("goose"));
    // `provider` is left unset

    let agent = FakeAgent::new().turn(Turn::new().text("never runs"));
    let seen = Seen::default();
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));

    let said = rig
        .on_goose(&agent, &seen, &heard, &tickets(), "12")
        .expect_err("a goose worker with no provider cannot start");

    assert!(said.contains("`provider`"), "{said}");
    assert!(
        seen.asks.lock().unwrap().is_empty(),
        "goose was never asked for"
    );
    assert!(agent.calls().is_empty(), "no harness was started");
    assert!(rig.recorded().is_empty(), "the failed start holds no slot");
}

#[test]
fn a_worker_on_goose_hands_the_goose_harness_to_the_worker_sandbox() {
    let rig = Rig::new();
    rig.set(Field::WorkerHarness, json!("goose"));
    rig.set(Field::Provider, json!("ollama"));
    rig.set(Field::Host, json!("http://127.0.0.1:11434"));

    let agent = FakeAgent::new();
    let seen = Seen::default();
    let asked: Mutex<Vec<SessionHarness>> = Mutex::new(Vec::new());
    let refused = |harness: &SessionHarness| -> Result<Sandbox, SandboxError> {
        lock(&asked).push(harness.clone());
        Err(SandboxError::Probe("the sandbox cannot start".to_owned()))
    };

    let said = opened(&rig, &agent, &seen, &refused).expect_err("no sandbox, no worker");

    assert!(said.contains("the sandbox cannot start"), "{said}");
    let asked = lock(&asked).clone();
    assert_eq!(
        asked.len(),
        1,
        "the harness the arm built is the one its sandbox is started around"
    );
    assert!(
        asked[0]
            .env
            .iter()
            .any(|(key, _)| key == "CARGO_TARGET_DIR"),
        "the worker's own variables are in the harness the sandbox is built from"
    );
    assert!(agent.calls().is_empty(), "no harness was started");
}
