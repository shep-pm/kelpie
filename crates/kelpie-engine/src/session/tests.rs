use std::io;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kelpie_protocol::{CacheLifetime, StopReason, ToolStatus, Usage};
use serde_json::json;

use super::fake::{EXPLANATION, FakeAgent, Turn};
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

/// The `_claude/sdkMessage` params a real Claude Code turn sent, with its
/// one-hour cache (`docs/acp.md`, Checked by hand)
fn sdk_result() -> serde_json::Value {
    json!({
        "sessionId": "fake-1",
        "message": {
            "type": "result",
            "subtype": "success",
            "is_error": false,
            "duration_ms": 4123,
            "num_turns": 1,
            "result": "Hello",
            "session_id": "fake-1",
            "total_cost_usd": 0.031,
            "usage": {
                "input_tokens": 6,
                "cache_creation_input_tokens": 28157,
                "cache_read_input_tokens": 0,
                "output_tokens": 12,
                "cache_creation": {
                    "ephemeral_1h_input_tokens": 28157,
                    "ephemeral_5m_input_tokens": 0
                }
            }
        }
    })
}

/// A turn's end, with the cache lifetime its usage names
fn ended_with(lifetime: Option<CacheLifetime>) -> SessionEvent {
    SessionEvent::TurnEnded {
        stop_reason: StopReason::EndTurn,
        usage: Some(Usage {
            input: 10,
            output: 20,
            cache_read: 30,
            cache_write: 40,
            cache_lifetime: lifetime,
        }),
    }
}

/// A turn that reports usage and sends `params` as one raw Claude Code message
fn raw_turn(params: serde_json::Value) -> Turn {
    Turn::new()
        .text("Hi")
        .extension(Extension::ClaudeSdkMessage.method(), params)
        .usage(10, 20, 30, 40)
}

#[test]
fn the_raw_result_gives_the_cache_lifetime() {
    let agent = FakeAgent::new()
        .reads(Extension::ClaudeSdkMessage)
        .turn(raw_turn(sdk_result()));

    let heard = say(&session(&agent), "Hello");

    assert_eq!(
        heard.last(),
        Some(&ended_with(Some(CacheLifetime::OneHour)))
    );
}

#[test]
fn the_shorter_cache_wins_when_it_holds_more_of_the_write() {
    let mut result = sdk_result();
    result["message"]["usage"]["cache_creation"]["ephemeral_1h_input_tokens"] = json!(4000);
    result["message"]["usage"]["cache_creation"]["ephemeral_5m_input_tokens"] = json!(24157);
    let agent = FakeAgent::new()
        .reads(Extension::ClaudeSdkMessage)
        .turn(raw_turn(result));

    let heard = say(&session(&agent), "Hello");

    assert_eq!(
        heard.last(),
        Some(&ended_with(Some(CacheLifetime::FiveMinutes)))
    );
}

#[test]
fn a_raw_message_that_is_not_the_result_names_no_lifetime() {
    let init = json!({
        "sessionId": "fake-1",
        "message": { "type": "system", "subtype": "init", "output_style": "Explanatory" }
    });
    let agent = FakeAgent::new()
        .reads(Extension::ClaudeSdkMessage)
        .turn(raw_turn(init));

    let heard = say(&session(&agent), "Hello");

    assert_eq!(heard.last(), Some(&ended_with(None)));
}

#[test]
fn an_extension_the_harness_does_not_name_is_not_read() {
    let agent = FakeAgent::new().turn(raw_turn(sdk_result()));

    let heard = say(&session(&agent), "Hello");

    assert_eq!(heard.last(), Some(&ended_with(None)));
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

/// One value the fake agent lists, which names it the same way it values it
fn listed(value: &str) -> kelpie_protocol::Choice {
    kelpie_protocol::Choice {
        id: value.to_owned(),
        name: value.to_owned(),
        description: None,
    }
}

/// A folder of this test's own, for what kelpie keeps of a harness's answers
fn data_folder() -> PathBuf {
    use crate::run::RunId;

    let dir = std::env::temp_dir().join(format!("kelpie-options-{}", RunId::new().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn asking_options_sends_no_prompt_and_stops_the_harness() {
    let mut agent = FakeAgent::new()
        .can_close()
        .offers_models("small", &["small", "large"])
        .offers_efforts("low", &["low", "high"]);
    let data = data_folder();
    let meta = json!({ "claudeCode": { "emitRawSDKMessages": true } });

    let options = options::offerings_at(
        &mut agent,
        "fake",
        &data,
        &PathBuf::from("/project"),
        Some(&meta),
        true,
    )
    .unwrap();

    assert_eq!(options.models, [listed("small"), listed("large")]);
    assert_eq!(options.efforts, [listed("low"), listed("high")]);
    assert_eq!(
        agent.methods(),
        ["initialize", "session/new", "session/close"],
        "the harness was asked, never prompted"
    );
    assert_eq!(
        agent.params("session/new")[0]["_meta"],
        meta,
        "the harness was asked the way a session would ask it"
    );
    assert_eq!(agent.launches(), 1);
    assert_eq!(
        agent.stopped(),
        1,
        "the harness was stopped, not left running"
    );
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn options_are_read_from_the_cache_until_a_refresh() {
    let mut agent = FakeAgent::new().offers_models("small", &["small", "large"]);
    let data = data_folder();
    let project = PathBuf::from("/project");

    let asked = options::offerings_at(&mut agent, "fake", &data, &project, None, false).unwrap();
    assert_eq!(agent.launches(), 1);

    let kept = options::offerings_at(&mut agent, "fake", &data, &project, None, false).unwrap();
    assert_eq!(kept, asked);
    assert_eq!(
        agent.launches(),
        1,
        "the kept answer is read, not asked again"
    );

    let fresh = options::offerings_at(&mut agent, "fake", &data, &project, None, true).unwrap();
    assert_eq!(fresh, asked);
    assert_eq!(agent.launches(), 2, "a refresh asks the harness again");

    let kept = std::fs::read_to_string(data.join("options").join("fake.json")).unwrap();
    let kept: serde_json::Value = serde_json::from_str(&kept).unwrap();
    assert!(
        kept["asked_at"].as_u64().is_some_and(|at| at > 0),
        "the answer says when it was asked: {kept}"
    );
    assert_eq!(kept["models"][1]["id"], "large");
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn a_harness_with_no_model_option_lists_none() {
    let mut agent = FakeAgent::new();

    let options = options::asked(&mut agent, &PathBuf::from("/project"), None).unwrap();

    assert_eq!(options, kelpie_protocol::Options::default());
    assert!(options.models.is_empty());
    assert!(options.efforts.is_empty());
}

#[test]
fn a_harness_id_that_names_no_file_starts_nothing() {
    let mut agent = FakeAgent::new();
    let data = data_folder();

    let error = options::offerings_at(
        &mut agent,
        "../escape",
        &data,
        &PathBuf::from("/project"),
        None,
        true,
    )
    .expect_err("no answer can be kept under a path");

    assert!(
        matches!(&error, SessionError::Name { name } if name == "../escape"),
        "{error}"
    );
    assert_eq!(agent.launches(), 0, "a bad name started no harness");
    std::fs::remove_dir_all(data).unwrap();
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
    let SessionEvent::Failed { message } = next(&heard) else {
        panic!("the agent exiting did not end the turn with Failed");
    };
    assert!(message.contains(EXPLANATION), "{message}");
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
        concat!(
            r#"Harness { program: "agent", args: [], env: ["TOKEN"], "#,
            r#"new_session_meta: true, extensions: [] }"#
        )
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

#[test]
fn a_send_queued_behind_a_turn_that_dies_is_answered() {
    let agent = FakeAgent::new().turn(Turn::new().text("a").exit());
    let session = Arc::new(session(&agent));
    let (second, second_heard) = events();
    let (first, first_heard) = {
        let session = Arc::clone(&session);
        let mut second = Some(second);
        let (sender, heard) = channel();
        // The worker is inside this sink, so the second message is queued
        // while the first turn runs
        let sink = move |event: SessionEvent| {
            if matches!(event, SessionEvent::Text { .. })
                && let Some(second) = second.take()
            {
                session.send("B", second);
            }
            let _ = sender.send(event);
        };
        (sink, heard)
    };

    session.send("A", first);

    assert_eq!(next(&first_heard), started("fake-1"));
    assert_eq!(next(&first_heard), text("a"));
    assert!(matches!(next(&first_heard), SessionEvent::Failed { .. }));
    assert!(matches!(next(&second_heard), SessionEvent::Failed { .. }));
}

#[test]
fn a_cancel_that_lost_the_race_with_its_turn_does_not_reach_the_next() {
    let agent = FakeAgent::new()
        .turn(Turn::new().text("a"))
        .turn(Turn::new().text("b"));
    let session = Arc::new(session(&agent));
    let (second, second_heard) = events();
    let (sender, first_heard) = channel();
    let mut second = Some(second);
    let inside = Arc::clone(&session);
    // The worker is inside this sink, so both land before it looks again
    let first = move |event: SessionEvent| {
        if matches!(event, SessionEvent::Text { .. })
            && let Some(second) = second.take()
        {
            inside.send("B", second);
            inside.cancel();
        }
        let _ = sender.send(event);
    };

    session.send("A", first);

    until_turn_ends(&first_heard);
    assert_eq!(until_turn_ends(&second_heard).last(), Some(&ended()));
    assert_ne!(
        agent.methods().last().map(String::as_str),
        Some("session/cancel")
    );
}

/// Blocks inside `explain` until told, and says when it got there
struct Slow {
    agent: FakeAgent,
    entered: Sender<()>,
    release: Arc<Mutex<Receiver<()>>>,
}

impl Launch for Slow {
    fn launch(&mut self) -> io::Result<Link> {
        let mut link = self.agent.launch()?;
        let (entered, release) = (self.entered.clone(), Arc::clone(&self.release));
        link.explain = Box::new(move || {
            let _ = entered.send(());
            let _ = release.lock().unwrap().recv_timeout(WAIT);
            None
        });
        Ok(link)
    }
}

#[test]
fn a_send_while_the_worker_explains_a_dead_harness_starts_a_new_worker() {
    let agent = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().exit())
        .turn(Turn::new().text("back"));
    let (entered, in_explain) = channel();
    let (release, released) = channel();
    let launcher = Slow {
        agent: agent.clone(),
        entered,
        release: Arc::new(Mutex::new(released)),
    };
    let session = Session::with_launcher(
        Box::new(launcher),
        PathBuf::from("/project"),
        None,
        Choice::default(),
        None,
        Vec::new(),
    );
    let (sink, first) = events();
    session.send("A", sink);
    in_explain.recv_timeout(WAIT).unwrap();

    let (sink, second) = events();
    session.send("B", sink);
    release.send(()).unwrap();

    assert_eq!(next(&first), started("fake-1"));
    assert!(matches!(next(&first), SessionEvent::Failed { .. }));
    assert_eq!(
        until_turn_ends(&second),
        [started("fake-1"), text("back"), ended()]
    );
}

#[test]
fn a_failure_that_is_not_the_harness_ending_does_not_blame_it() {
    let agent = FakeAgent::new();
    let session = Session::with_launcher(
        Box::new(agent.clone()),
        PathBuf::from("/project"),
        None,
        Choice::default(),
        Some(json!("not an object")),
        Vec::new(),
    );

    let (sink, heard) = events();
    session.send("Hello", sink);

    let SessionEvent::Failed { message } = next(&heard) else {
        panic!("a bad `_meta` did not end the send with Failed");
    };
    assert!(!message.contains(EXPLANATION), "{message}");
}

/// A pipe whose reader has gone: writes fail, and nothing ever arrives
struct Broken;

impl futures::io::AsyncWrite for Broken {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::task::Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl futures::io::AsyncRead for Broken {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        _: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::task::Poll::Pending
    }
}

struct DeadOnWrite;

impl Launch for DeadOnWrite {
    fn launch(&mut self) -> io::Result<Link> {
        Ok(Link {
            transport: agent_client_protocol::DynConnectTo::new(
                agent_client_protocol::ByteStreams::new(Broken, Broken),
            ),
            explain: Box::new(|| Some(EXPLANATION.to_owned())),
            stop: Box::new(|_| Some(EXPLANATION.to_owned())),
        })
    }
}

#[test]
fn a_harness_gone_before_the_first_write_is_explained() {
    let session = Session::with_launcher(
        Box::new(DeadOnWrite),
        PathBuf::from("/project"),
        None,
        Choice::default(),
        None,
        Vec::new(),
    );

    let (sink, heard) = events();
    session.send("Hello", sink);

    let SessionEvent::Failed { message } = next(&heard) else {
        panic!("a broken pipe did not end the send with Failed");
    };
    assert!(message.contains(EXPLANATION), "{message}");
}

const INSTRUCTIONS: &str = "You are the project manager.";

fn system_prompt(text: &str) -> serde_json::Value {
    json!({ "mode": "append", "content": [{ "type": "text", "text": text }] })
}

fn prompt_texts(agent: &FakeAgent) -> Vec<String> {
    agent
        .params("session/prompt")
        .iter()
        .map(|params| params["prompt"][0]["text"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn instructions_go_in_the_system_prompt_when_offered() {
    let agent = FakeAgent::new().accepts_system_prompt();

    say(&session(&agent).with_instructions(INSTRUCTIONS), "Hi");

    assert_eq!(
        agent.params("session/new")[0]["systemPrompt"],
        system_prompt(INSTRUCTIONS)
    );
    assert_eq!(prompt_texts(&agent), ["Hi"]);
}

#[test]
fn instructions_lead_the_first_message_otherwise() {
    let agent = FakeAgent::new();
    let session = session(&agent).with_instructions(INSTRUCTIONS);

    say(&session, "Hi");
    say(&session, "Again");

    assert!(agent.params("session/new")[0].get("systemPrompt").is_none());
    assert_eq!(
        prompt_texts(&agent),
        [format!("{INSTRUCTIONS}\n\nHi"), "Again".to_owned()]
    );
}

#[test]
fn instructions_go_in_the_system_prompt_on_resume_and_on_load() {
    let resumes = FakeAgent::new()
        .accepts_system_prompt()
        .can_resume()
        .knows("old");
    let loads = FakeAgent::new()
        .accepts_system_prompt()
        .can_load()
        .knows("old");

    for agent in [&resumes, &loads] {
        let session = agent
            .session(
                PathBuf::from("/project"),
                Some("old".into()),
                Choice::default(),
            )
            .with_instructions(INSTRUCTIONS);
        say(&session, "Back again");
    }

    assert_eq!(
        resumes.params("session/resume")[0]["systemPrompt"],
        system_prompt(INSTRUCTIONS)
    );
    assert_eq!(
        loads.params("session/load")[0]["systemPrompt"],
        system_prompt(INSTRUCTIONS)
    );
    assert_eq!(prompt_texts(&resumes), ["Back again"]);
}

#[test]
fn a_resumed_session_without_a_system_prompt_does_not_get_the_instructions_again() {
    let agent = FakeAgent::new().can_resume().knows("old");

    say(
        &agent
            .session(
                PathBuf::from("/project"),
                Some("old".into()),
                Choice::default(),
            )
            .with_instructions(INSTRUCTIONS),
        "Back again",
    );

    assert_eq!(prompt_texts(&agent), ["Back again"]);
}

#[test]
fn a_resume_that_starts_a_new_session_gives_it_the_instructions() {
    let agent = FakeAgent::new().can_resume();

    let heard = say(
        &agent
            .session(
                PathBuf::from("/project"),
                Some("gone".into()),
                Choice::default(),
            )
            .with_instructions(INSTRUCTIONS),
        "Back again",
    );

    assert!(matches!(heard[0], SessionEvent::Failed { .. }));
    assert_eq!(
        prompt_texts(&agent),
        [format!("{INSTRUCTIONS}\n\nBack again")]
    );
}

#[test]
fn blank_instructions_change_nothing() {
    let agent = FakeAgent::new().accepts_system_prompt();

    say(&session(&agent).with_instructions("  \n"), "Hi");

    assert!(agent.params("session/new")[0].get("systemPrompt").is_none());
    assert_eq!(prompt_texts(&agent), ["Hi"]);
}
