use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use kelpie_protocol::{StopReason, ToolStatus, Usage};
use serde_json::json;

use super::fake::{FakeAgent, Turn};
use super::*;

/// How long a test waits for an event it forced to happen
const WAIT: Duration = Duration::from_secs(10);

/// A sink, and the events it was given
fn events() -> (
    impl FnMut(SessionEvent) + Send + 'static,
    Receiver<SessionEvent>,
) {
    let (sender, receiver) = channel();
    (
        move |event| {
            let _ = sender.send(event);
        },
        receiver,
    )
}

fn next(events: &Receiver<SessionEvent>) -> SessionEvent {
    events
        .recv_timeout(WAIT)
        .expect("the session sent no more events")
}

/// Everything up to and including the event that ends the turn
fn until_turn_ends(events: &Receiver<SessionEvent>) -> Vec<SessionEvent> {
    let mut heard = Vec::new();
    loop {
        let event = next(events);
        let ends = matches!(event, SessionEvent::TurnEnded { .. });
        heard.push(event);
        if ends {
            return heard;
        }
    }
}

fn started(id: &str) -> SessionEvent {
    SessionEvent::Started {
        session_id: id.into(),
    }
}

fn text(delta: &str) -> SessionEvent {
    SessionEvent::Text {
        delta: delta.to_owned(),
    }
}

fn ended() -> SessionEvent {
    SessionEvent::TurnEnded {
        stop_reason: StopReason::EndTurn,
        usage: None,
    }
}

fn session(agent: &FakeAgent) -> Session {
    agent.session(PathBuf::from("/project"), None, Choice::default())
}

/// One message and its whole turn
fn say(session: &Session, message: &str) -> Vec<SessionEvent> {
    let (sink, heard) = events();
    session.send(message, sink);
    until_turn_ends(&heard)
}

#[test]
fn the_reply_streams_as_text_events_in_order() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hel").text("lo, ").text("world"));

    let heard = say(&session(&agent), "Hi");

    assert_eq!(
        heard,
        [
            started("fake-1"),
            text("Hel"),
            text("lo, "),
            text("world"),
            ended()
        ]
    );
}

#[test]
fn nothing_runs_until_the_first_send() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hi"));

    let session = session(&agent);

    assert_eq!(agent.launches(), 0);
    assert_eq!(session.session_id(), None);
    say(&session, "Hello");
    assert_eq!(agent.launches(), 1);
}

fn step(id: &str, title: &str, status: ToolStatus) -> SessionEvent {
    SessionEvent::ToolStep {
        id: id.to_owned(),
        title: title.to_owned(),
        status,
    }
}

#[test]
fn tool_calls_become_tool_steps() {
    let agent = FakeAgent::new().turn(
        Turn::new()
            .tool_call("t1", "Read Cargo.toml")
            .tool_running("t1")
            .tool_done("t1")
            .text("Done"),
    );

    let heard = say(&session(&agent), "Look");

    assert_eq!(
        heard,
        [
            started("fake-1"),
            step("t1", "Read Cargo.toml", ToolStatus::Pending),
            step("t1", "Read Cargo.toml", ToolStatus::Running),
            step("t1", "Read Cargo.toml", ToolStatus::Done),
            text("Done"),
            ended(),
        ]
    );
}

#[test]
fn a_permission_request_is_refused() {
    let agent = FakeAgent::new().turn(Turn::new().permission("t1", "Write notes.md"));

    let heard = say(&session(&agent), "Write it");

    assert_eq!(
        heard,
        [
            started("fake-1"),
            step("t1", "Write notes.md", ToolStatus::Refused),
            ended(),
        ]
    );
    assert_eq!(
        agent.params("session/request_permission"),
        [json!({"outcome": {"outcome": "selected", "optionId": "reject"}})]
    );
}

#[test]
fn the_turn_ends_with_its_usage() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hi").usage(10, 20, 30, 40));

    let heard = say(&session(&agent), "Hello");

    assert_eq!(
        heard.last(),
        Some(&SessionEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            usage: Some(Usage {
                input: 10,
                output: 20,
                cache_read: 30,
                cache_write: 40,
                cache_lifetime: None,
            }),
        })
    );
}

#[test]
fn resume_prefers_session_resume_over_load() {
    let agent = FakeAgent::new().can_resume().can_load().knows("old");

    let heard = say(
        &agent.session(
            PathBuf::from("/project"),
            Some("old".into()),
            Choice::default(),
        ),
        "Back again",
    );

    assert_eq!(heard, [started("old"), ended()]);
    assert_eq!(
        agent.methods(),
        ["initialize", "session/resume", "session/prompt"]
    );
}

#[test]
fn load_drops_the_replayed_history() {
    let agent = FakeAgent::new()
        .can_load()
        .knows("old")
        .replays("an earlier answer");

    let heard = say(
        &agent.session(
            PathBuf::from("/project"),
            Some("old".into()),
            Choice::default(),
        ),
        "Back again",
    );

    assert_eq!(heard, [started("old"), ended()]);
    assert_eq!(
        agent.methods(),
        ["initialize", "session/load", "session/prompt"]
    );
}

#[test]
fn an_unknown_session_starts_fresh_and_says_so() {
    let agent = FakeAgent::new().can_resume().knows("other");

    let heard = say(
        &agent.session(
            PathBuf::from("/project"),
            Some("gone".into()),
            Choice::default(),
        ),
        "Hello",
    );

    assert!(
        matches!(&heard[0], SessionEvent::Failed { message } if message.contains("gone")),
        "{heard:?}"
    );
    assert_eq!(heard[1..], [started("fake-1"), ended()]);
}

#[test]
fn model_and_effort_use_config_options() {
    let agent = FakeAgent::new()
        .offers_models("small", &["small", "large"])
        .offers_efforts("low", &["low", "high"]);
    let choice = Choice {
        model: Some("large".to_owned()),
        effort: Some("high".to_owned()),
    };

    let heard = say(
        &agent.session(PathBuf::from("/project"), None, choice),
        "Hello",
    );

    assert_eq!(heard, [started("fake-1"), ended()]);
    assert_eq!(
        agent.params("session/set_config_option"),
        [
            json!({"sessionId": "fake-1", "configId": "model", "value": "large"}),
            json!({"sessionId": "fake-1", "configId": "effort", "value": "high"}),
        ]
    );
}

#[test]
fn an_unlisted_model_fails_and_keeps_the_default() {
    let agent = FakeAgent::new().offers_models("small", &["small", "large"]);
    let choice = Choice {
        model: Some("huge".to_owned()),
        effort: None,
    };

    let heard = say(
        &agent.session(PathBuf::from("/project"), None, choice),
        "Hello",
    );

    assert_eq!(heard[0], started("fake-1"));
    assert!(
        matches!(&heard[1], SessionEvent::Failed { message } if message.contains("huge")),
        "{heard:?}"
    );
    assert_eq!(heard[2], ended());
    assert!(agent.params("session/set_config_option").is_empty());
}

#[test]
fn cancel_ends_the_turn_cancelled() {
    let agent = FakeAgent::new().turn(Turn::new().text("Working").until_cancelled());
    let session = session(&agent);
    let (sink, heard) = events();
    session.send("Go", sink);
    assert_eq!(next(&heard), started("fake-1"));
    assert_eq!(next(&heard), text("Working"));

    session.cancel();

    assert_eq!(
        next(&heard),
        SessionEvent::TurnEnded {
            stop_reason: StopReason::Cancelled,
            usage: None,
        }
    );
    assert_eq!(
        agent.methods().last().map(String::as_str),
        Some("session/cancel")
    );
}

#[test]
fn the_agent_exiting_mid_turn_ends_with_failed() {
    let agent = FakeAgent::new().turn(Turn::new().text("Starting").exit());

    let session = session(&agent);
    let (sink, heard) = events();
    session.send("Go", sink);

    assert_eq!(next(&heard), started("fake-1"));
    assert_eq!(next(&heard), text("Starting"));
    assert!(matches!(next(&heard), SessionEvent::Failed { .. }));
}

#[test]
fn a_second_send_reuses_the_connection() {
    let agent = FakeAgent::new()
        .turn(Turn::new().text("one"))
        .turn(Turn::new().text("two"));
    let session = session(&agent);

    let first = say(&session, "First");
    let second = say(&session, "Second");

    assert_eq!(first, [started("fake-1"), text("one"), ended()]);
    assert_eq!(second, [text("two"), ended()]);
    assert_eq!(agent.launches(), 1);
    assert_eq!(
        agent.methods(),
        [
            "initialize",
            "session/new",
            "session/prompt",
            "session/prompt"
        ]
    );
}

#[test]
fn a_send_after_the_agent_left_resumes_the_session() {
    let agent = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().exit())
        .turn(Turn::new().text("back"));
    let session = session(&agent);
    let (sink, heard) = events();
    session.send("Go", sink);
    assert_eq!(next(&heard), started("fake-1"));
    assert!(matches!(next(&heard), SessionEvent::Failed { .. }));

    let second = say(&session, "Again");

    assert_eq!(second, [started("fake-1"), text("back"), ended()]);
    assert_eq!(agent.launches(), 2);
}

#[test]
fn a_harness_shows_the_names_of_its_environment_and_no_values() {
    let mut harness = Harness::new("agent");
    harness.env.push(("TOKEN".into(), "hunter2".into()));
    harness.new_session_meta = Some(json!({"secret": "hunter2"}));

    assert_eq!(
        format!("{harness:?}"),
        r#"Harness { program: "agent", args: [], env: ["TOKEN"], new_session_meta: true }"#
    );
}

#[cfg(unix)]
#[test]
fn a_harness_that_exits_is_explained_from_its_status_and_stderr() {
    use crate::run::RunId;
    use crate::tree::ProcessTree;

    let dir = std::env::temp_dir().join(format!("kelpie-session-{}", RunId::new().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    let tree = Arc::new(ProcessTree::new(RunId::new().unwrap(), dir.join("run.json")).unwrap());
    let mut harness = Harness::new("sh");
    harness.args = vec!["-c".into(), "echo no login >&2; exit 3".into()];
    let session = Session::new(tree, harness, dir.clone(), None, Choice::default());

    let (sink, heard) = events();
    session.send("Hello", sink);

    let SessionEvent::Failed { message } = next(&heard) else {
        panic!("the harness exiting did not end the turn with Failed");
    };
    assert!(message.contains("exit status: 3"), "{message}");
    assert!(message.contains("no login"), "{message}");
    std::fs::remove_dir_all(dir).unwrap();
}
