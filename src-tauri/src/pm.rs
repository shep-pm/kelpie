//! The PM's commands: what the chat box calls
//!
//! [`pm_send`] sends a message to the project's PM and streams the reply over a
//! [`Channel`], one [`SessionEvent`] after another. Nothing here starts a
//! process: the engine's session does, through the process tree (ADR 0001), and
//! the app only forwards what it says.
//!
//! The PM runs on what the settings resolve to at each send ([`plan`]): the
//! global `pm` agent, then the project's own setting, over [`PM_DEFAULT`], the
//! built-in layer (ADR 0006). Its bridge is installed on the first send, and
//! whatever goes wrong before the session can start reaches the chat as a
//! [`SessionEvent::Failed`], like a failure mid-turn.

use std::fs;
use std::sync::Arc;

use kelpie_engine::agents::{Agents, Binding, Origin, Sourced};
use kelpie_engine::bridge;
use kelpie_engine::harness::claude;
use kelpie_engine::session::{Choice, Session, SessionEvent};
use kelpie_engine::settings::{ConfigDir, Harness, Settings};
use tauri::State;
use tauri::ipc::Channel;

use crate::state::{AppState, Opening};

/// What the PM runs on when no layer says
#[derive(Debug, Clone, Copy)]
pub struct PmDefault {
    /// The harness
    pub harness: Harness,

    /// The model, as the harness names it
    pub model: &'static str,

    /// The effort, as the harness names it
    pub effort: &'static str,
}

/// The built-in layer (ADR 0006): kelpie ships it, and the global `pm` agent
/// and the project's setting replace it
///
/// The model and effort belong to the harness here. They stand only while the
/// PM is on this harness, since another harness names its models its own way.
pub const PM_DEFAULT: PmDefault = PmDefault {
    harness: Harness::ClaudeCode,
    model: "opus",
    effort: "high",
};

/// What a PM session is opened on, resolved from the settings
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PmPlan {
    /// The harness
    pub harness: Harness,

    /// The model and effort the harness is asked for
    pub choice: Choice,

    /// The PM's instructions with the reply style after them, `None` when both are blank
    pub instructions: Option<String>,

    /// Where the harness's model runs, for a harness that reaches one through a server.
    /// No harness kelpie runs the PM on reads it yet.
    pub provider: Option<String>,

    /// The host the provider runs on
    pub host: Option<String>,
}

/// The PM's harness, model and effort once [`PM_DEFAULT`] fills what no layer set
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaulted {
    /// The harness
    pub harness: Sourced<Harness>,

    /// The model
    pub model: Sourced<Option<String>>,

    /// The effort
    pub effort: Sourced<Option<String>>,
}

/// `binding` with the built-in layer under it, each value still naming the layer it came from
///
/// The default model and effort stand only while the PM is on the default
/// harness: another harness names its models its own way.
#[must_use]
pub fn with_defaults(binding: &Binding) -> Defaulted {
    let harness = match binding.harness.value {
        Some(value) => Sourced {
            value,
            from: binding.harness.from.clone(),
        },
        None => Sourced {
            value: PM_DEFAULT.harness,
            from: Origin::BuiltIn,
        },
    };
    let on_default = harness.value == PM_DEFAULT.harness;
    let or_default = |own: &Sourced<Option<String>>, default: &str| match &own.value {
        Some(_) => own.clone(),
        None if on_default => Sourced {
            value: Some(default.to_owned()),
            from: Origin::BuiltIn,
        },
        None => own.clone(),
    };
    Defaulted {
        model: or_default(&binding.model, PM_DEFAULT.model),
        effort: or_default(&binding.effort, PM_DEFAULT.effort),
        harness,
    }
}

/// What the PM runs on for the project `project_id`, from the files in `config`
///
/// Read on every send, so a change made on the settings page applies to the next
/// message. The layers are kelpie's built-in default, the global `pm` agent
/// (`~/.config/kelpie/agents/pm.md`), then the project's own setting.
///
/// # Errors
///
/// A settings file or the PM's agent file cannot be used. The text names the
/// file and what is wrong with it.
pub fn plan(config: &ConfigDir, project_id: &str) -> Result<PmPlan, String> {
    let settings = Settings::load(config, project_id).map_err(|error| error.to_string())?;
    let agents = Agents::load(config).map_err(|error| error.to_string())?;
    let binding = agents
        .pm_binding(&settings)
        .map_err(|error| error.to_string())?;
    let Defaulted {
        harness,
        model,
        effort,
    } = with_defaults(&binding);
    let style = settings.reply_style.value.instructions();
    let instructions = [binding.body.value.as_deref(), style]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(PmPlan {
        harness: harness.value,
        choice: Choice {
            model: model.value,
            effort: effort.value,
        },
        instructions: (!instructions.trim().is_empty()).then_some(instructions),
        provider: settings.provider.value,
        host: settings.host.value,
    })
}

/// The app's [`Opener`](crate::state::Opener): the PM on what `opening.pm` says
///
/// Installs the bridge if it is not yet, and builds the session on it.
///
/// # Errors
///
/// The state folder cannot be made, the bridge cannot be installed, or the
/// harness is not installed or has no login. The text says which.
pub fn open(opening: &Opening<'_>) -> Result<Session, String> {
    let Opening {
        tree,
        data_dir,
        project,
        state_dir,
        logins,
        pm,
    } = *opening;
    // parity: harness.pm
    let harness = match pm.harness {
        Harness::ClaudeCode => {
            fs::create_dir_all(state_dir)
                .map_err(|error| format!("could not make {}: {error}", state_dir.display()))?;
            let bridge = bridge::ensure(data_dir, claude::pin()).map_err(|e| e.to_string())?;
            claude::harness(bridge, state_dir, logins).map_err(|e| e.to_string())?
        }
        // `Harness` grows with each harness's slice, and none but Claude Code is built here
        other => return Err(format!("{other:?} is not wired to the PM yet")),
    };
    let session = Session::new(
        Arc::clone(tree),
        harness,
        project.path.clone(),
        project.pm_session.clone(),
        pm.choice.clone(),
    );
    Ok(match &pm.instructions {
        Some(instructions) => session.with_instructions(instructions.clone()),
        None => session,
    })
}

/// The folder of the project the maintainer picked, `None` before they pick one
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn project_get(state: State<'_, AppState>) -> Option<String> {
    state
        .project_path()
        .map(|path| path.to_string_lossy().into_owned())
}

/// Ask the maintainer for the project's folder, and keep it
///
/// Returns the folder, or `None` when they closed the dialog. A folder picked
/// while one is kept replaces it, and the PM starts over on the new one. The
/// dialog blocks, so this runs off the main thread.
///
/// # Errors
///
/// The dialog failed, or the folder cannot be kept. The text is for the
/// maintainer to read.
#[tauri::command(async)]
#[specta::specta]
pub fn project_pick(state: State<'_, AppState>) -> Result<Option<String>, String> {
    Ok(state
        .pick_project()?
        .map(|path| path.to_string_lossy().into_owned()))
}

/// Send `text` to the PM and stream its reply into `on_event`
///
/// Returns once the message is handed to the session, and every event of the
/// turn follows over the channel. The first send installs the bridge and starts
/// the harness, which can take a while, so this runs off the main thread. The
/// session id is saved to `projects.json` when the session starts, so the next
/// launch resumes it. A failure before the session exists is the turn's only
/// event, a `Failed`.
#[tauri::command(async)]
#[specta::specta]
pub fn pm_send(state: State<'_, AppState>, text: String, on_event: Channel<SessionEvent>) {
    let (session, project) = match state.pm_session() {
        Ok(held) => held,
        Err(message) => {
            let _ = on_event.send(SessionEvent::Failed { message });
            return;
        }
    };
    session.send(text, move |event| {
        // Saved before it is forwarded, so whoever hears `Started` finds it kept
        if let SessionEvent::Started { session_id } = &event
            && let Err(error) = project.keep_session(session_id.clone())
        {
            let _ = on_event.send(SessionEvent::Failed {
                message: format!("could not save the session id: {error}"),
            });
        }
        let _ = on_event.send(event);
    });
}

/// Stop the turn in flight
///
/// The turn ends as the harness reports it, normally with `TurnEnded` and
/// `Cancelled`. Between turns, and while the first send is still opening the
/// session, it does nothing: it never builds a session, and never waits for one,
/// since a sync command runs on the main thread.
#[tauri::command]
#[specta::specta]
pub fn pm_cancel(state: State<'_, AppState>) {
    if let Some(session) = state.held_session() {
        session.cancel();
    }
}

#[cfg(test)]
mod tests {
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
    use kelpie_protocol::{StopReason, ToolStatus};
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
        let (events, heard) = channel();
        let channel = Channel::new(move |body| {
            let InvokeResponseBody::Json(json) = body else {
                panic!("an event should be JSON");
            };
            let _ = events.send(serde_json::from_str::<SessionEvent>(&json).unwrap());
            Ok(())
        });
        pm_send(state, text.to_owned(), channel);
        Events(heard)
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
                },
                SessionEvent::ToolStep {
                    id: "t1".to_owned(),
                    title: "Read decisions.md".to_owned(),
                    status: ToolStatus::Done,
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
}
