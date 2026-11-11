//! What the app gives the PM's session, and what its answers do
//!
//! Each test dials the session's door as a harness's relay does, or reads the
//! server the session was handed (`docs/design/kelpie-tools.md`). The PM's
//! session is the only one kelpie gives `start_worker` to, and the app is what
//! hands it the starter that starts a worker.

use kelpie_engine::session::SandboxAnswer;
use kelpie_engine::session::fake::{FakeAgent, FakeNetwork, Turn};
use kelpie_engine::tools::{DOOR_VAR, SERVER_NAME, TOKEN_VAR};

use super::*;

#[test]
fn the_pm_session_is_given_kelpies_tools() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
    let chat = Chat::with_tools(&agent, &FakeNetwork::allowing(&["api.anthropic.com"]));

    chat.send("Fetch docs.rs").until_turn_ends();

    let opened = agent.params("session/new");
    let [new] = opened.as_slice() else {
        panic!("the agent should have been opened once");
    };
    let tools = chat.session_tools();
    let servers = new["mcpServers"].as_array().unwrap();
    assert_eq!(servers.len(), 1, "{servers:?}");
    let server = &servers[0];
    assert_eq!(server["name"], serde_json::json!(SERVER_NAME));
    assert_eq!(server["args"], serde_json::json!(["tools"]));
    assert_eq!(
        server["env"],
        serde_json::json!([
            {"name": DOOR_VAR, "value": tools.door().to_string()},
            {"name": TOKEN_VAR, "value": tools.token()},
        ])
    );
}

#[test]
fn pm_answer_widens_the_session() {
    let network = FakeNetwork::allowing(&["api.anthropic.com"]);
    let agent = FakeAgent::new().turn(Turn::new().until_cancelled());
    let chat = Chat::with_tools(&agent, &network);
    let events = chat.send("Fetch docs.rs");
    let mut asked = chat.asking();

    asked.ask("docs.rs", "to read the crate's docs").unwrap();
    let (id, host, why) = events.until_request();
    assert_eq!(
        (host.as_str(), why.as_str()),
        ("docs.rs", "to read the crate's docs")
    );
    assert_eq!(
        network.allowed(),
        ["api.anthropic.com"],
        "refused until the answer"
    );

    pm_answer(chat.state(), id.clone(), SandboxAnswer::Once).unwrap();

    assert_eq!(
        asked.answered().unwrap(),
        ("docs.rs is allowed for this session".to_owned(), false)
    );
    assert_eq!(network.allowed(), ["api.anthropic.com", "docs.rs"]);
    assert_eq!(
        pm_answer(chat.state(), id, SandboxAnswer::Once),
        Err("That request is over.".to_owned()),
        "the answered request is not waiting any more"
    );
}

#[test]
fn allow_for_this_project_saves_the_host() {
    let network = FakeNetwork::default();
    let agent = FakeAgent::new().turn(Turn::new().until_cancelled());
    let chat = Chat::with_tools(&agent, &network);
    let events = chat.send("Fetch docs.rs");
    let mut asked = chat.asking();

    asked.ask("docs.rs", "to read the crate's docs").unwrap();
    let (id, ..) = events.until_request();

    pm_answer(chat.state(), id, SandboxAnswer::Project).unwrap();

    assert_eq!(
        asked.answered().unwrap().0,
        "docs.rs is allowed and saved for this project"
    );
    assert_eq!(network.allowed(), ["docs.rs"]);

    let project = chat.state().project_id(None).unwrap();
    let plan = plan(chat.state().config(), &project).unwrap();
    assert_eq!(plan.network_allowlist, ["docs.rs"]);
}

#[test]
fn a_project_answer_the_sandbox_could_not_take_says_so_and_saves_nothing() {
    let network = FakeNetwork::gone();
    let agent = FakeAgent::new().turn(Turn::new().until_cancelled());
    let chat = Chat::with_tools(&agent, &network);
    let events = chat.send("Fetch docs.rs");
    let mut asked = chat.asking();

    asked.ask("docs.rs", "to read the crate's docs").unwrap();
    let (id, ..) = events.until_request();

    assert_eq!(
        pm_answer(chat.state(), id, SandboxAnswer::Project),
        Err(
            "docs.rs is not through this session's sandbox, so kelpie saved nothing for it"
                .to_owned()
        ),
    );
    assert_eq!(
        asked.answered().unwrap().0,
        "docs.rs was allowed, but the sandbox could not take it, so it is still refused"
    );

    let project = chat.state().project_id(None).unwrap();
    let plan = plan(chat.state().config(), &project).unwrap();
    assert!(
        plan.network_allowlist.is_empty(),
        "the project was not told"
    );
    assert!(network.allowed().is_empty(), "the sandbox took nothing");
}

#[test]
fn the_pms_session_is_given_start_worker() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
    let chat = Chat::with_tools(&agent, &FakeNetwork::allowing(&[]));
    chat.send("Start a worker on #12").until_turn_ends();
    let mut asked = chat.asking();

    assert_eq!(
        asked.listed().unwrap(),
        ["request_network", "start_worker"],
        "the PM's tools"
    );
}
