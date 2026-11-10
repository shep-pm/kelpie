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
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::settings::ConfigDir;
use kelpie_protocol::{LogEntry, StopReason, ToolStatus};
use tauri::ipc::InvokeResponseBody;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use tauri::{App, Manager as _};

use super::*;
use crate::settings::{Target, settings_set};
use crate::state::{Opener, Picker, Projects};

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(10);

/// An app on the mock runtime, with its state in a folder of its own
struct Chat {
    app: App<MockRuntime>,
    dir: PathBuf,
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
        Self { app, dir }
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
fn the_next_send_uses_the_new_model() {
    let agent = FakeAgent::new()
        .offers_models("small", &["small", "large"])
        .turn(Turn::new().text("one"))
        .turn(Turn::new().text("two"));
    let chat = Chat::planned(&agent);
    let project = chat.state().project_id(None).unwrap();
    let layer = || Target::Project {
        id: project.clone(),
    };
    settings_set(chat.state(), layer(), "pm-model".to_owned(), "small".into()).unwrap();
    chat.send("First").until_turn_ends();

    settings_set(chat.state(), layer(), "pm-model".to_owned(), "large".into()).unwrap();
    chat.send("Second").until_turn_ends();

    assert_eq!(agent.launches(), 1, "the same session carries on");
    assert_eq!(
        agent.params("session/set_config_option"),
        [
            serde_json::json!({"sessionId": "fake-1", "configId": "model", "value": "small"}),
            serde_json::json!({"sessionId": "fake-1", "configId": "model", "value": "large"}),
        ]
    );
}

#[test]
fn a_model_the_global_pm_agent_sets_reaches_the_first_send() {
    let agent = FakeAgent::new()
        .offers_models("small", &["small", "large"])
        .turn(Turn::new().text("one"));
    let chat = Chat::planned(&agent);
    settings_set(
        chat.state(),
        Target::Global,
        "pm-model".to_owned(),
        "large".into(),
    )
    .unwrap();

    chat.send("First").until_turn_ends();

    assert_eq!(
        agent.params("session/set_config_option"),
        [serde_json::json!({"sessionId": "fake-1", "configId": "model", "value": "large"})]
    );
}

#[test]
fn with_nothing_set_the_pm_runs_on_kelpies_default() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();

    let plan = plan(chat.state().config(), &project).unwrap();

    assert_eq!(plan.harness, PM_DEFAULT.harness);
    assert_eq!(plan.choice.model.as_deref(), Some(PM_DEFAULT.model));
    assert_eq!(plan.choice.effort.as_deref(), Some(PM_DEFAULT.effort));
}

#[test]
fn another_harness_gets_no_model_kelpie_named_for_the_default_one() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();
    settings_set(
        chat.state(),
        Target::Project {
            id: project.clone(),
        },
        "pm-harness".to_owned(),
        "goose".into(),
    )
    .unwrap();

    let plan = plan(chat.state().config(), &project).unwrap();

    assert_eq!(plan.harness, Harness::Goose);
    assert_eq!(plan.choice, Choice::default());
}

#[test]
fn with_nothing_set_the_sandbox_has_no_extra_domain_and_reads_the_project_alone() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();

    let plan = plan(chat.state().config(), &project).unwrap();

    assert!(plan.network_allowlist.is_empty());
    assert_eq!(plan.reads, Reads::Project);
}

#[test]
fn the_projects_allowlist_and_read_anywhere_settings_reach_the_plan() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();
    let set = |key: &str, value: serde_json::Value| {
        settings_set(
            chat.state(),
            Target::Project {
                id: project.clone(),
            },
            key.to_owned(),
            value,
        )
        .unwrap();
    };
    set(
        "network-allowlist",
        serde_json::json!(["docs.rs", "crates.io"]),
    );
    set("read-anywhere", serde_json::json!(true));

    let plan = plan(chat.state().config(), &project).unwrap();

    assert_eq!(plan.network_allowlist, ["docs.rs", "crates.io"]);
    assert_eq!(plan.reads, Reads::Anywhere);
}

#[test]
fn the_reply_style_is_part_of_the_instructions_unless_the_harness_default_is_chosen() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();
    let instructions = || {
        plan(chat.state().config(), &project)
            .unwrap()
            .instructions
            .unwrap()
    };
    assert!(instructions().contains("## Replies"));
    assert!(instructions().contains("decisions.md"));

    settings_set(
        chat.state(),
        Target::Global,
        "reply-style".to_owned(),
        "harness-default".into(),
    )
    .unwrap();

    assert!(!instructions().contains("## Replies"));
    assert!(instructions().contains("decisions.md"));
}

#[test]
fn a_settings_file_that_cannot_be_used_reaches_the_chat_as_failed() {
    let chat = Chat::on(&FakeAgent::new());
    let file = chat.dir.join("config").join("settings.json");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "{ not json").unwrap();

    let SessionEvent::Failed { message } = chat.send("Hi").next() else {
        panic!("the chat should hear a Failed");
    };

    assert!(message.contains("settings.json"), "{message}");
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
        settings_set(chat.state(), target, key.to_owned(), value.into()).unwrap();
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
        settings_set(chat.state(), target, key.to_owned(), value.into()).unwrap();
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
        settings_set(chat.state(), target, key.to_owned(), value.into()).unwrap();
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
