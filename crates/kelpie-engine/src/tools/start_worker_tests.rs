//! Who is given `start_worker`, and what one call of it does
//!
//! The PM's session is the only one that has the tool, so a worker's session is
//! the least-privileged role (`docs/design/kelpie-tools.md`, The role split).
//! Each test dials the door with a session's token, as a harness's relay does.

use std::sync::{Arc, Mutex};

use serde_json::json;

use super::SessionTools;
use super::fake::FakeTools;
use super::network::REQUEST_NETWORK;
use super::start_worker::{START_WORKER, WorkerStarter};
use super::tests::{Scratch, started};

/// What a starter answers, and every ticket it was asked for
#[derive(Debug)]
struct Starter {
    asked: Mutex<Vec<String>>,
    answer: Result<String, String>,
}

impl Starter {
    /// A starter that says `line` for every ticket
    fn answering(line: &str) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            answer: Ok(line.to_owned()),
        })
    }

    /// A starter that refuses with `reason`
    fn refusing(reason: &str) -> Arc<Self> {
        Arc::new(Self {
            asked: Mutex::new(Vec::new()),
            answer: Err(reason.to_owned()),
        })
    }

    /// The tickets it was asked to start a worker on, in order
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

/// A starter that says `Started a worker on #155` for every ticket
fn starter() -> Arc<dyn WorkerStarter> {
    Starter::answering("Started a worker on #155") as Arc<dyn WorkerStarter>
}

impl WorkerStarter for Starter {
    fn start(&self, ticket: &str) -> Result<String, String> {
        self.asked.lock().unwrap().push(ticket.to_owned());
        self.answer.clone()
    }
}

/// The tools kelpie lists for a session, dialled with its token
fn listed(session: &SessionTools) -> Vec<String> {
    FakeTools::connect(session)
        .expect("the door takes the session's token")
        .listed()
        .expect("the server lists its tools")
}

#[test]
fn a_pm_session_lists_start_worker_and_request_network() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let pm = tools.pm_session(starter());

    assert_eq!(
        listed(&pm),
        [REQUEST_NETWORK, START_WORKER],
        "the PM's tools"
    );
}

#[test]
fn a_worker_session_lists_request_network_alone() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let worker = tools.session();

    assert_eq!(listed(&worker), [REQUEST_NETWORK], "a worker's tools");
}

#[test]
fn a_worker_session_calling_start_worker_gets_no_such_tool_and_starts_nothing() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let starter = Starter::answering("Started a worker on #155");
    let pm = tools.pm_session(Arc::clone(&starter) as Arc<dyn WorkerStarter>);
    let worker = tools.session();
    let mut agent = FakeTools::connect(&worker).expect("the door takes the token");

    agent.start_worker("155").expect("the call is sent");

    let (line, failed) = agent.answered().expect("the call is answered");
    assert!(failed, "a worker was given the PM's tool: {line}");
    assert_eq!(
        line,
        format!("kelpie has no tool named {START_WORKER}"),
        "{line}"
    );
    assert!(starter.asked().is_empty(), "a worker started a worker");
    // The control: the same server does give the tool to the PM's session
    assert!(listed(&pm).contains(&START_WORKER.to_owned()));
}

#[test]
fn the_pm_calling_start_worker_reaches_the_starter_with_the_ticket() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let starter = Starter::answering("Started a worker on #155");
    let pm = tools.pm_session(Arc::clone(&starter) as Arc<dyn WorkerStarter>);
    let mut agent = FakeTools::connect(&pm).expect("the door takes the token");

    for given in [json!("155"), json!(155), json!("#155")] {
        agent
            .call(START_WORKER, json!({"ticket": given}))
            .expect("the call is sent");
        let (line, failed) = agent.answered().expect("the call is answered");
        assert!(!failed, "{given}: {line}");
        assert_eq!(line, "Started a worker on #155", "{given}");
    }

    assert_eq!(starter.asked(), ["155", "155", "155"]);
}

#[test]
fn a_starters_error_is_the_tools_error_line() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let starter = Starter::refusing("issue 155 is closed, so no worker starts on it");
    let pm = tools.pm_session(Arc::clone(&starter) as Arc<dyn WorkerStarter>);
    let mut agent = FakeTools::connect(&pm).expect("the door takes the token");

    agent.start_worker("155").expect("the call is sent");

    let (line, failed) = agent.answered().expect("the call is answered");
    assert!(failed, "the start's refusal was told as a success: {line}");
    assert_eq!(line, "issue 155 is closed, so no worker starts on it");
    assert_eq!(starter.asked(), ["155"]);
}

#[test]
fn a_call_with_no_ticket_or_a_bad_one_is_refused_before_the_starter() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let starter = Starter::answering("Started a worker on #155");
    let pm = tools.pm_session(Arc::clone(&starter) as Arc<dyn WorkerStarter>);
    let mut agent = FakeTools::connect(&pm).expect("the door takes the token");

    for given in [
        json!({}),
        json!({"ticket": ""}),
        json!({"ticket": "155 issues"}),
        json!({"ticket": 0}),
    ] {
        agent
            .call(START_WORKER, given.clone())
            .expect("the call is sent");
        let (line, failed) = agent.answered().expect("the call is answered");
        assert!(failed, "{given} started a worker: {line}");
        assert!(line.contains("ticket"), "{given}: {line}");
    }
    assert!(
        starter.asked().is_empty(),
        "a bad ticket reached the starter"
    );

    // The control: the same call with a ticket number is taken
    agent.start_worker("155").expect("the call is sent");
    let (line, failed) = agent.answered().expect("the call is answered");
    assert!(!failed, "{line}");
    assert_eq!(starter.asked(), ["155"]);
}
