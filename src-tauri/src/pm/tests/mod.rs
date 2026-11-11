//! Tests for the PM commands

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kelpie_engine::login::Store;
use kelpie_engine::run::RunId;
use kelpie_engine::session::SessionId;
use kelpie_engine::session::fake::{FakeAgent, FakeNetwork, Turn};
use kelpie_engine::settings::ConfigDir;
use kelpie_engine::tools::DOOR_VAR;
use kelpie_engine::tools::SERVER_NAME;
use kelpie_engine::tools::TOKEN_VAR;
use kelpie_engine::tools::fake::FakeTools;
use kelpie_protocol::{LogEntry, StopReason, ToolStatus};
use tauri::ipc::InvokeResponseBody;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use tauri::{App, AppHandle, Manager as _};

use super::*;
use crate::settings::{Target, settings_set};
use crate::state::{Opener, Picker, Projects};

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(10);

/// An app on the mock runtime, with its state in a folder of its own
struct Chat {
    app: App<MockRuntime>,
    dir: PathBuf,
    /// The tools the opener gave the session, `None` when it gave none
    tools: Arc<Mutex<Option<SessionTools>>>,
}

impl Chat {
    /// A chat whose PM runs on `agent`, the project already picked
    fn on(agent: &FakeAgent) -> Self {
        let chat = Self::with(Self::opener(agent), Box::new(|| Ok(None)), None);
        chat.state().set_project(chat.dir.clone()).unwrap();
        chat
    }

    /// A chat that left off with the session `saved`, as the last launch left it
    fn resumed(agent: &FakeAgent, saved: &str) -> Self {
        Self::with(Self::opener(agent), Box::new(|| Ok(None)), Some(saved))
    }

    /// A chat whose PM is asked for the model and effort the settings resolve to
    fn planned(agent: &FakeAgent) -> Self {
        let agent = agent.clone();
        let opener: Opener = Box::new(move |opening| {
            Ok(agent.session(
                opening.project.path.clone(),
                opening.project.pm_session.clone(),
                opening.pm.choice.clone(),
            ))
        });
        let chat = Self::with(opener, Box::new(|| Ok(None)), None);
        chat.state().set_project(chat.dir.clone()).unwrap();
        chat
    }

    /// A chat whose PM is Claude Code on `claude` until the settings name goose, which runs on `goose`
    fn switching(claude: &FakeAgent, goose: &FakeAgent) -> Self {
        let chat = Self::with(
            Self::two_harnesses(claude, goose),
            Box::new(|| Ok(None)),
            None,
        );
        chat.state().set_project(chat.dir.clone()).unwrap();
        chat
    }

    /// A chat as a launch finds it after Claude Code ran the PM as `ran` and saved the session `s-claude`
    fn after_claude(claude: &FakeAgent, goose: &FakeAgent, ran: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-pm-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        let list = format!(
            r#"[{{"id":"p1","path":"{}","pm_session":"s-claude","pm_ran":{ran}}}]"#,
            dir.display()
        );
        fs::write(dir.join("projects.json"), list).unwrap();
        Self::at(
            dir,
            Self::two_harnesses(claude, goose),
            Box::new(|| Ok(None)),
        )
    }

    /// An opener that gives Claude Code's `claude` and every other harness's `goose`
    fn two_harnesses(claude: &FakeAgent, goose: &FakeAgent) -> Opener {
        let (claude, goose) = (claude.clone(), goose.clone());
        Box::new(move |opening| {
            let agent = match opening.pm.harness {
                Harness::ClaudeCode => &claude,
                _ => &goose,
            };
            Ok(agent.session(
                opening.project.path.clone(),
                opening.project.pm_session.clone(),
                opening.pm.choice.clone(),
            ))
        })
    }

    /// What the PM's log holds
    fn log(&self) -> Vec<LogEntry> {
        log::read(self.state().pm_log_path().unwrap()).unwrap()
    }

    /// A chat whose PM runs on `agent` with kelpie's tools, in a sandbox whose lists `network` holds
    ///
    /// The opener keeps the session's tools, so a test can dial the door the way
    /// the harness's relay does.
    fn with_tools(agent: &FakeAgent, network: &FakeNetwork) -> Self {
        let (agent, network) = (agent.clone(), network.clone());
        let kept: Arc<Mutex<Option<SessionTools>>> = Arc::new(Mutex::new(None));
        let opener: Opener = {
            let kept = Arc::clone(&kept);
            Box::new(move |opening| {
                let Some(tools) = opening.tools.map(Tools::session) else {
                    return Err("kelpie's tools door did not open".to_owned());
                };
                *kept.lock().unwrap() = Some(tools.clone());
                Ok(agent
                    .clone()
                    .sandbox_network(&network)
                    .session(
                        opening.project.path.clone(),
                        opening.project.pm_session.clone(),
                        Choice::default(),
                    )
                    .with_tools(tools))
            })
        };
        let mut chat = Self::with(opener, Box::new(|| Ok(None)), None);
        chat.tools = kept;
        chat.state().set_project(chat.dir.clone()).unwrap();
        chat
    }

    /// The tools the opener handed the session
    fn session_tools(&self) -> SessionTools {
        self.tools
            .lock()
            .unwrap()
            .clone()
            .expect("the session was given kelpie's tools")
    }

    /// The agent's side of kelpie's tools, dialled with the session's token
    fn asking(&self) -> FakeTools {
        FakeTools::connect(&self.session_tools()).unwrap()
    }

    /// An opener whose session asks the harness for no model or effort
    fn opener(agent: &FakeAgent) -> Opener {
        let agent = agent.clone();
        Box::new(move |opening| {
            Ok(agent.session(
                opening.project.path.clone(),
                opening.project.pm_session.clone(),
                Choice::default(),
            ))
        })
    }

    fn with(opener: Opener, picker: Picker, saved: Option<&str>) -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-pm-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        if let Some(session) = saved {
            let list = format!(
                r#"[{{"id":"p1","path":"{}","pm_session":"{session}"}}]"#,
                dir.display()
            );
            fs::write(dir.join("projects.json"), list).unwrap();
        }
        Self::at(dir, opener, picker)
    }

    /// A chat whose state folder is `dir`, which may already hold `projects.json`
    fn at(dir: PathBuf, opener: Opener, picker: Picker) -> Self {
        let config = ConfigDir::at(dir.join("config"));
        let state =
            AppState::open(dir.clone(), config, Store::memory(), opener, picker, None).unwrap();
        let app = mock_builder()
            .manage(state)
            .build(mock_context(noop_assets()))
            .unwrap();
        Self {
            app,
            dir,
            tools: Arc::new(Mutex::new(None)),
        }
    }

    /// What a command that reaches the window sees
    fn handle(&self) -> AppHandle<MockRuntime> {
        self.app.handle().clone()
    }

    /// What the commands see: the state Tauri manages
    fn state(&self) -> State<'_, AppState> {
        self.app.state::<AppState>()
    }

    /// `pm_send`, with the events it streams
    fn send(&self, text: &str) -> Events {
        send_to(self.state(), text)
    }

    fn projects(&self) -> Projects {
        Projects::load(self.dir.join("projects.json")).unwrap()
    }
}

impl Drop for Chat {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn send_to(state: State<'_, AppState>, text: &str) -> Events {
    let (events, channel) = listener();
    pm_send(state, text.to_owned(), channel);
    events
}

/// A channel, and the events it carries
fn listener() -> (Events, Channel<SessionEvent>) {
    let (events, heard) = channel();
    let channel = Channel::new(move |body| {
        let InvokeResponseBody::Json(json) = body else {
            panic!("an event should be JSON");
        };
        let _ = events.send(serde_json::from_str::<SessionEvent>(&json).unwrap());
        Ok(())
    });
    (Events(heard), channel)
}

/// The events a channel carried, in order
struct Events(Receiver<SessionEvent>);

impl Events {
    fn next(&self) -> SessionEvent {
        self.0
            .recv_timeout(WAIT)
            .expect("the channel should carry the next event")
    }

    /// Everything up to and including the turn's end
    fn until_turn_ends(&self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        loop {
            let event = self.next();
            let done = matches!(
                event,
                SessionEvent::TurnEnded { .. } | SessionEvent::Failed { .. }
            );
            events.push(event);
            if done {
                return events;
            }
        }
    }

    /// Everything up to and including the sandbox request the maintainer is asked about,
    /// as its id, host and why
    fn until_request(&self) -> (String, String, String) {
        loop {
            match self.next() {
                SessionEvent::SandboxRequest { id, host, why } => return (id, host, why),
                SessionEvent::Failed { message } => panic!("the turn failed: {message}"),
                _ => {}
            }
        }
    }
}

#[test]
fn pm_send_forwards_every_event_to_the_channel() {
    let agent = FakeAgent::new().turn(
        Turn::new()
            .thought("Checking")
            .text("Hello, ")
            .text("maintainer")
            .tool_call("t1", "Read decisions.md")
            .tool_done("t1")
            .usage(3, 4, 5, 6),
    );
    let chat = Chat::on(&agent);

    let events = chat.send("Hi").until_turn_ends();

    assert!(
        matches!(events.first(), Some(SessionEvent::Started { .. })),
        "{events:?}"
    );
    assert_eq!(
        events[1..events.len() - 1],
        [
            SessionEvent::Thought {
                delta: "Checking".to_owned()
            },
            SessionEvent::Text {
                delta: "Hello, ".to_owned()
            },
            SessionEvent::Text {
                delta: "maintainer".to_owned()
            },
            SessionEvent::ToolStep {
                id: "t1".to_owned(),
                title: "Read decisions.md".to_owned(),
                status: ToolStatus::Pending,
                reason: None,
            },
            SessionEvent::ToolStep {
                id: "t1".to_owned(),
                title: "Read decisions.md".to_owned(),
                status: ToolStatus::Done,
                reason: None,
            },
        ]
    );
    assert!(
        matches!(
            events.last(),
            Some(SessionEvent::TurnEnded {
                stop_reason: StopReason::EndTurn,
                usage: Some(_),
            })
        ),
        "{events:?}"
    );
}

#[test]
fn the_session_id_is_saved_on_started() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
    let chat = Chat::on(&agent);

    let events = chat.send("Hi");
    let SessionEvent::Started { session_id } = events.next() else {
        panic!("the first event should be Started");
    };

    // `Started` came over the channel, so the id was kept before it
    assert_eq!(
        chat.projects().first().unwrap().pm_session,
        Some(session_id)
    );
}

#[test]
fn a_saved_session_is_resumed_on_the_next_launch() {
    let agent = FakeAgent::new()
        .can_resume()
        .knows("s-saved")
        .turn(Turn::new().text("Back"));
    let chat = Chat::resumed(&agent, "s-saved");

    let events = chat.send("Hi").until_turn_ends();

    assert_eq!(
        events[0],
        SessionEvent::Started {
            session_id: SessionId::from("s-saved")
        }
    );
    assert!(
        agent
            .methods()
            .iter()
            .any(|method| method == "session/resume")
    );
}

/// The text of the last prompt an agent got
fn last_prompt(agent: &FakeAgent) -> String {
    let prompts = agent.params("session/prompt");
    prompts.last().unwrap()["prompt"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn a_new_harness_stops_the_old_session_and_logs_a_divider() {
    let claude = FakeAgent::new()
        .turn(Turn::new().text("Claude answers"))
        .turn(Turn::new().text("never asked"));
    let goose = FakeAgent::new()
        .offers_models("other", &["other", "qwen"])
        .turn(Turn::new().text("goose answers"));
    let chat = Chat::switching(&claude, &goose);
    let project = chat.state().project_id(None).unwrap();
    let set = |key: &str, value: &str| {
        let target = Target::Project {
            id: project.clone(),
        };
        settings_set(
            chat.handle(),
            chat.state(),
            target,
            key.to_owned(),
            value.into(),
        )
        .unwrap();
    };
    chat.send("First").until_turn_ends();
    assert_eq!(claude.stopped(), 0, "the session runs between messages");

    set("pm-harness", "goose");
    set("pm-model", "qwen");
    chat.send("Second").until_turn_ends();

    assert_eq!(claude.stopped(), 1, "the old harness was stopped");
    assert_eq!(goose.launches(), 1);
    assert_eq!(
        claude.params("session/prompt").len(),
        1,
        "the old session was sent nothing more"
    );
    let divider = LogEntry::Divider {
        text: "Switched from Claude Code (opus) to goose (qwen)".to_owned(),
    };
    let log = chat.log();
    let at = log.iter().position(|entry| *entry == divider).unwrap();
    assert_eq!(
        log[at + 1],
        LogEntry::User {
            text: "Second".to_owned()
        },
        "the divider comes before the message that started the new session: {log:?}"
    );
    let prompt = last_prompt(&goose);
    assert!(prompt.contains("Maintainer: First"), "{prompt}");
    assert!(prompt.contains("PM: Claude answers"), "{prompt}");
    assert!(prompt.ends_with("Second"), "{prompt}");
}

#[test]
fn a_refused_resume_starts_a_seeded_session() {
    let agent = FakeAgent::new()
        .can_resume()
        .knows("other")
        .turn(Turn::new().text("Fresh"));
    let chat = Chat::resumed(&agent, "gone");
    let log_path = chat.state().pm_log_path().unwrap();
    let earlier = log::Log::open(&log_path).unwrap();
    for entry in [
        LogEntry::User {
            text: "Earlier question".to_owned(),
        },
        LogEntry::Event(SessionEvent::Text {
            delta: "Earlier answer".to_owned(),
        }),
    ] {
        earlier.append(&entry).unwrap();
    }
    fs::write(
        log_path.with_file_name("decisions.md"),
        "- Never squash merges\n",
    )
    .unwrap();

    let events = chat.send("Now");
    // The refusal is a `Failed` the turn goes on from, so wait for its end
    while !matches!(events.next(), SessionEvent::TurnEnded { .. }) {}

    let prompt = last_prompt(&agent);
    let decisions = prompt.find("Never squash merges").unwrap();
    let exchange = prompt.find("Maintainer: Earlier question").unwrap();
    assert!(decisions < exchange, "{prompt}");
    assert!(prompt.contains("PM: Earlier answer"), "{prompt}");
    assert!(prompt.ends_with("Now"), "{prompt}");
    assert!(
        !prompt.contains("Maintainer: Now"),
        "the message being sent is not part of its own seed: {prompt}"
    );
    let log = chat.log();
    let divider = LogEntry::Divider {
        text: "Couldn't resume, started a new session".to_owned(),
    };
    assert_eq!(log.iter().filter(|entry| **entry == divider).count(), 1);
}

#[test]
fn a_harness_changed_before_a_restart_starts_a_seeded_session_instead_of_resuming() {
    let claude = FakeAgent::new().can_resume().knows("s-claude");
    let goose = FakeAgent::new()
        .can_resume()
        .knows("s-claude")
        .offers_models("other", &["other", "qwen"])
        .turn(Turn::new().text("goose answers"));
    let chat = Chat::after_claude(
        &claude,
        &goose,
        r#"{"harness":"claude-code","model":"opus"}"#,
    );
    let log = log::Log::open(chat.state().pm_log_path().unwrap()).unwrap();
    for entry in [
        LogEntry::User {
            text: "Before the restart".to_owned(),
        },
        LogEntry::Event(SessionEvent::Text {
            delta: "Claude's answer".to_owned(),
        }),
    ] {
        log.append(&entry).unwrap();
    }
    let set = |key: &str, value: &str| {
        let target = Target::Project { id: "p1".into() };
        settings_set(
            chat.handle(),
            chat.state(),
            target,
            key.to_owned(),
            value.into(),
        )
        .unwrap();
    };
    set("pm-harness", "goose");
    set("pm-model", "qwen");

    chat.send("After").until_turn_ends();

    assert_eq!(
        goose.params("session/resume"),
        Vec::<serde_json::Value>::new(),
        "goose is not offered Claude Code's session id"
    );
    let prompt = last_prompt(&goose);
    assert!(prompt.contains("PM: Claude's answer"), "{prompt}");
    assert!(prompt.ends_with("After"), "{prompt}");
    let divider = |text: &str| LogEntry::Divider {
        text: text.to_owned(),
    };
    let log = chat.log();
    assert!(
        log.contains(&divider("Switched from Claude Code (opus) to goose (qwen)")),
        "{log:?}"
    );
    assert!(
        !log.contains(&divider("Couldn't resume, started a new session")),
        "{log:?}"
    );
    assert_eq!(
        chat.projects().first().unwrap().pm_session,
        Some("fake-1".into()),
        "the saved id is goose's own, not Claude Code's"
    );
}

#[test]
fn the_divider_names_the_model_the_old_session_last_ran_on() {
    let claude = FakeAgent::new()
        .offers_models("small", &["small", "large"])
        .turn(Turn::new().text("one"))
        .turn(Turn::new().text("two"));
    let goose = FakeAgent::new()
        .offers_models("other", &["other", "qwen"])
        .turn(Turn::new().text("three"));
    let chat = Chat::switching(&claude, &goose);
    let project = chat.state().project_id(None).unwrap();
    let set = |key: &str, value: &str| {
        let target = Target::Project {
            id: project.clone(),
        };
        settings_set(
            chat.handle(),
            chat.state(),
            target,
            key.to_owned(),
            value.into(),
        )
        .unwrap();
    };
    set("pm-model", "small");
    chat.send("First").until_turn_ends();
    set("pm-model", "large");
    chat.send("Second").until_turn_ends();

    set("pm-harness", "goose");
    set("pm-model", "qwen");
    chat.send("Third").until_turn_ends();

    let divider = LogEntry::Divider {
        text: "Switched from Claude Code (large) to goose (qwen)".to_owned(),
    };
    assert!(chat.log().contains(&divider), "{:?}", chat.log());
}

#[test]
fn pm_cancel_ends_the_turn() {
    let agent = FakeAgent::new().turn(Turn::new().text("Working").until_cancelled());
    let chat = Chat::on(&agent);

    let events = chat.send("Do a long thing");
    while !matches!(events.next(), SessionEvent::Text { .. }) {}
    pm_cancel(chat.state());

    assert!(matches!(
        events.next(),
        SessionEvent::TurnEnded {
            stop_reason: StopReason::Cancelled,
            ..
        }
    ));
}

#[test]
fn pm_cancel_with_no_session_opens_none() {
    let opened = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&opened);
    let chat = Chat::with(
        Box::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Err("not wanted".to_owned())
        }),
        Box::new(|| Ok(None)),
        None,
    );
    chat.state().set_project(chat.dir.clone()).unwrap();

    pm_cancel(chat.state());

    assert_eq!(opened.load(Ordering::SeqCst), 0);
}

#[test]
fn pm_cancel_does_not_wait_for_the_first_send_to_open() {
    let (opening, started) = channel();
    let (release, released) = channel::<()>();
    let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
    let make = Chat::opener(&agent);
    let released = Mutex::new(released);
    let chat = Chat::with(
        Box::new(move |opening_with| {
            opening.send(()).unwrap();
            released.lock().unwrap().recv_timeout(WAIT).unwrap();
            make(opening_with)
        }),
        Box::new(|| Ok(None)),
        None,
    );
    chat.state().set_project(chat.dir.clone()).unwrap();

    std::thread::scope(|scope| {
        let handle = chat.app.handle().clone();
        let sending = scope.spawn(move || send_to(handle.state::<AppState>(), "Hi"));
        started.recv_timeout(WAIT).unwrap();
        // The opener is running and holds the session slot's place: this returns
        pm_cancel(chat.state());
        release.send(()).unwrap();
        sending.join().unwrap().until_turn_ends();
    });
}

#[test]
fn a_send_before_a_project_is_picked_says_so_as_failed() {
    let chat = Chat::with(Chat::opener(&FakeAgent::new()), Box::new(|| Ok(None)), None);

    let SessionEvent::Failed { message } = chat.send("Hi").next() else {
        panic!("the chat should hear a Failed");
    };

    assert!(message.contains("Pick"), "{message}");
}

#[test]
fn a_send_while_an_install_waits_is_refused_and_opens_no_session() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
    let chat = Chat::on(&agent);
    let state = chat.state();
    let hold = state.hold_for_install().unwrap();

    let SessionEvent::Failed { message } = chat.send("Hi").next() else {
        panic!("the chat should hear a Failed");
    };

    assert!(message.contains("installing"), "{message}");
    assert!(
        chat.state().held_session().is_none(),
        "the refused send built no session for the install to end"
    );

    // Once the prompt closes the same send goes through
    drop(hold);
    let events = chat.send("Hi").until_turn_ends();
    assert!(
        matches!(events.last(), Some(SessionEvent::TurnEnded { .. })),
        "{events:?}"
    );
}

#[test]
fn an_opener_that_fails_reaches_the_chat_as_failed() {
    let chat = Chat::with(
        Box::new(|_| Err("the bridge could not be downloaded".to_owned())),
        Box::new(|| Ok(None)),
        None,
    );
    chat.state().set_project(chat.dir.clone()).unwrap();

    assert_eq!(
        chat.send("Hi").next(),
        SessionEvent::Failed {
            message: "the bridge could not be downloaded".to_owned()
        }
    );
}

#[test]
fn project_pick_keeps_the_folder_and_project_get_returns_it() {
    let picked = std::env::temp_dir().join("kelpie-picked");
    let answer = picked.clone();
    let chat = Chat::with(
        Chat::opener(&FakeAgent::new()),
        Box::new(move || Ok(Some(answer.clone()))),
        None,
    );
    assert_eq!(project_get(chat.state()), None);

    let folder = project_pick(chat.state()).unwrap();

    assert_eq!(folder, Some(picked.to_string_lossy().into_owned()));
    assert_eq!(project_get(chat.state()), folder);
    assert_eq!(chat.projects().first().unwrap().path, picked);
}

#[test]
fn a_closed_dialog_keeps_nothing() {
    let chat = Chat::with(Chat::opener(&FakeAgent::new()), Box::new(|| Ok(None)), None);

    assert_eq!(project_pick(chat.state()), Ok(None));
    assert_eq!(project_get(chat.state()), None);
}

fn user(text: &str) -> LogEntry {
    LogEntry::User {
        text: text.to_owned(),
    }
}

#[test]
fn pm_history_shows_what_a_send_logged() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
    let chat = Chat::on(&agent);
    let events = chat.send("Hi").until_turn_ends();

    let history = pm_history(chat.state()).unwrap();

    let mut expected = vec![user("Hi")];
    expected.extend(events.into_iter().map(LogEntry::Event));
    assert_eq!(history, expected);
}

#[test]
fn pm_history_before_a_project_or_a_message_is_empty() {
    let chat = Chat::with(Chat::opener(&FakeAgent::new()), Box::new(|| Ok(None)), None);
    assert_eq!(pm_history(chat.state()).unwrap(), []);

    chat.state().set_project(chat.dir.clone()).unwrap();
    assert_eq!(pm_history(chat.state()).unwrap(), []);
}

#[test]
fn pm_history_starts_no_process_and_opens_no_session() {
    let opened = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&opened);
    let chat = Chat::with(
        Box::new(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Err("not wanted".to_owned())
        }),
        Box::new(|| Ok(None)),
        Some("s-saved"),
    );
    let log = chat.dir.join("projects/p1/pm/log.jsonl");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, "{\"entry\":\"user\",\"text\":\"Before the quit\"}\n").unwrap();

    let history = pm_history(chat.state()).unwrap();

    assert_eq!(history, [user("Before the quit")]);
    assert_eq!(opened.load(Ordering::SeqCst), 0);
}

#[test]
fn pm_retry_sends_the_last_user_message_again() {
    let agent = FakeAgent::new()
        .can_resume()
        .knows("s-saved")
        .turn(Turn::new().text("Again"));
    let chat = Chat::resumed(&agent, "s-saved");
    // The last launch quit mid-turn: its log has the message and no end
    let log = chat.dir.join("projects/p1/pm/log.jsonl");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(
        &log,
        concat!(
            "{\"entry\":\"user\",\"text\":\"First\"}\n",
            "{\"entry\":\"event\",\"kind\":\"turn_ended\",\"stop_reason\":\"end_turn\",\"usage\":null}\n",
            "{\"entry\":\"user\",\"text\":\"Cut off\"}\n",
            "{\"entry\":\"event\",\"kind\":\"text\",\"delta\":\"Par\"}\n",
        ),
    )
    .unwrap();
    let (events, channel) = listener();

    pm_retry(chat.state(), channel);
    events.until_turn_ends();

    let prompts = agent.params("session/prompt");
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].to_string().contains("Cut off"), "{prompts:?}");
    let history = pm_history(chat.state()).unwrap();
    assert_eq!(history.iter().filter(|e| **e == user("Cut off")).count(), 2);
}

#[test]
fn pm_retry_with_nothing_sent_says_so_as_failed() {
    let chat = Chat::on(&FakeAgent::new());
    let (events, channel) = listener();

    pm_retry(chat.state(), channel);

    let SessionEvent::Failed { message } = events.next() else {
        panic!("the chat should hear a Failed");
    };
    assert!(message.contains("no message"), "{message}");
}

#[test]
fn quitting_mid_turn_leaves_the_turn_without_an_end_in_the_log() {
    let agent = FakeAgent::new().turn(Turn::new().text("Working").until_cancelled());
    let chat = Chat::on(&agent);
    let events = chat.send("Do a long thing");
    while !matches!(events.next(), SessionEvent::Text { .. }) {}

    // What the app does on a quit, then the harness's end reaches the session
    chat.state().stop_all().unwrap();
    pm_cancel(chat.state());
    events.until_turn_ends();

    let history = pm_history(chat.state()).unwrap();
    assert!(
        history
            .iter()
            .all(|entry| !matches!(entry, LogEntry::Event(SessionEvent::TurnEnded { .. }))),
        "{history:?}"
    );
    assert_eq!(history.first(), Some(&user("Do a long thing")));
}

#[test]
fn a_log_with_a_bad_line_in_the_middle_does_not_stop_a_send() {
    let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
    let chat = Chat::on(&agent);
    let log = chat.state().pm_log_path().unwrap();
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, "nonsense\n{\"entry\":\"user\",\"text\":\"Hi\"}\n").unwrap();

    let events = chat.send("Again").until_turn_ends();

    assert!(
        matches!(events.last(), Some(SessionEvent::TurnEnded { .. })),
        "{events:?}"
    );
    // The history cannot be read as a whole, and says why
    assert!(pm_history(chat.state()).unwrap_err().contains("line 1"));
}

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

mod confinement;
mod plan;
