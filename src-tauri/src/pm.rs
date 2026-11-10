//! The PM's commands: what the chat box calls
//!
//! [`pm_send`] sends a message to the project's PM and streams the reply over a
//! [`Channel`], one [`SessionEvent`] after another. Nothing here starts a
//! process: the engine's session does, through the process tree (ADR 0001), and
//! the app only forwards what it says.
//!
//! [`pm_history`] reads kelpie's own log of the PM and starts nothing, and
//! [`pm_retry`] sends the last message again after a quit cut a turn off.
//!
//! The PM runs on [`PM_DEFAULT`] until the settings page holds the choice
//! (ADR 0006). Its bridge is installed on the first send, and whatever goes
//! wrong before the session can start reaches the chat as a
//! [`SessionEvent::Failed`], like a failure mid-turn.

use std::fs;
use std::sync::Arc;

use kelpie_engine::agents::{Agents, PM};
use kelpie_engine::bridge;
use kelpie_engine::harness::claude;
use kelpie_engine::session::log;
use kelpie_engine::session::{Choice, LogEntry, Session, SessionEvent};
use kelpie_engine::settings::Harness;
use tauri::State;
use tauri::ipc::Channel;

use crate::state::{AppState, Opening};

/// What the PM runs on
#[derive(Debug, Clone, Copy)]
pub struct PmDefault {
    /// The harness
    pub harness: Harness,

    /// The model, as the harness names it
    pub model: &'static str,

    /// The effort, as the harness names it
    pub effort: &'static str,
}

/// The built-in default (ADR 0006): kelpie ships it, and the settings page
/// replaces it
pub const PM_DEFAULT: PmDefault = PmDefault {
    harness: Harness::ClaudeCode,
    model: "opus",
    effort: "high",
};

/// The app's [`Opener`](crate::state::Opener): the PM on [`PM_DEFAULT`]
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
    } = *opening;
    // parity: harness.pm
    let harness = match PM_DEFAULT.harness {
        Harness::ClaudeCode => {
            fs::create_dir_all(state_dir)
                .map_err(|error| format!("could not make {}: {error}", state_dir.display()))?;
            let bridge = bridge::ensure(data_dir, claude::pin()).map_err(|e| e.to_string())?;
            claude::harness(bridge, state_dir, logins).map_err(|e| e.to_string())?
        }
        // `Harness` grows with each harness's slice, and none but the default is built here
        other => return Err(format!("{other:?} is not wired to the PM yet")),
    };
    let choice = Choice {
        model: Some(PM_DEFAULT.model.to_owned()),
        effort: Some(PM_DEFAULT.effort.to_owned()),
    };
    let session = Session::new(
        Arc::clone(tree),
        harness,
        project.path.clone(),
        project.pm_session.clone(),
        choice,
    );
    // kelpie:pm is the PM's instructions until the agent editor holds them
    Ok(match Agents::built_in(PM).and_then(|pm| pm.body.value) {
        Some(body) => session.with_instructions(body),
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
    send(&state, text, on_event);
}

/// Send the last message the maintainer sent again, and stream the reply into `on_event`
///
/// For a turn a quit cut off. The message is read from kelpie's log and goes
/// through the same path as [`pm_send`], so the log gets it again and the
/// session resumes. Nothing in the log to send is a `Failed` on the channel.
#[tauri::command(async)]
#[specta::specta]
pub fn pm_retry(state: State<'_, AppState>, on_event: Channel<SessionEvent>) {
    let last = state
        .pm_log_path()
        .map_or(Ok(Vec::new()), log::read)
        .map(|entries| {
            entries.into_iter().rev().find_map(|entry| match entry {
                LogEntry::User { text } => Some(text),
                _ => None,
            })
        });
    match last {
        Ok(Some(text)) => send(&state, text, on_event),
        Ok(None) => {
            let _ = on_event.send(SessionEvent::Failed {
                message: "There is no message to send again.".to_owned(),
            });
        }
        Err(error) => {
            let _ = on_event.send(SessionEvent::Failed {
                message: format!("could not read the PM's log: {error}"),
            });
        }
    }
}

fn send(state: &AppState, text: String, on_event: Channel<SessionEvent>) {
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

/// The history kelpie logged for the PM, oldest first, empty before a project or a message
///
/// Reads kelpie's own log and starts no process: the harness is not asked, and
/// no session is built. A torn last line is dropped.
///
/// # Errors
///
/// The log cannot be read, or a line of it before the last is not an entry. The
/// text is for the maintainer to read.
#[tauri::command(async)]
#[specta::specta]
pub fn pm_history(state: State<'_, AppState>) -> Result<Vec<LogEntry>, String> {
    let Some(path) = state.pm_log_path() else {
        return Ok(Vec::new());
    };
    log::read(&path).map_err(|error| format!("could not read the PM's history: {error}"))
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
    use kelpie_protocol::{LogEntry, StopReason, ToolStatus};
    use tauri::ipc::InvokeResponseBody;
    use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
    use tauri::{App, Manager as _};

    use super::*;
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
            let state = AppState::open(dir.clone(), Store::memory(), opener, picker).unwrap();
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
}
