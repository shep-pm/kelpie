//! The PM's commands: what the chat box calls
//!
//! [`pm_send`] sends a message to the project's PM and streams the reply over a
//! [`Channel`], one [`SessionEvent`] after another. Nothing here starts a
//! process: the engine's session does, through the process tree (ADR 0001), and
//! the app only forwards what it says.
//!
//! A send whose harness differs from the session's does not move the session: it
//! stops it, logs a divider and starts a seeded one ([`AppState::pm_session`]),
//! and a resume the harness refuses starts a seeded session the same way
//! (`docs/design/pm-chat.md`, Switching harness or model).
//!
//! [`pm_history`] reads kelpie's own log of the PM and starts nothing, and
//! [`pm_retry`] sends the last message again after a quit cut a turn off.
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
use kelpie_engine::session::log;
use kelpie_engine::session::{Choice, LogEntry, Session, SessionEvent};
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

/// How the maintainer's divider names a harness and the model it runs, such as `goose (qwen)`
#[must_use]
pub fn describe(harness: Harness, model: Option<&str>) -> String {
    // parity: harness.pm
    let name = match harness {
        Harness::ClaudeCode => "Claude Code",
        Harness::Goose => "goose",
        // `Harness` grows with each harness's slice
        other => return format!("{other:?}"),
    };
    match model {
        Some(model) => format!("{name} ({model})"),
        None => name.to_owned(),
    }
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
///
/// A send whose settings name another harness than the session runs on stops
/// that session and starts a new one with a seed ahead of the message. It
/// returns the divider's text, which is in the log already, and `None` for any
/// other send. A resume the harness refuses does the same, and its divider is
/// in the log and not returned: the turn's `Failed` says it live.
#[tauri::command(async)]
#[specta::specta]
pub fn pm_send(
    state: State<'_, AppState>,
    text: String,
    on_event: Channel<SessionEvent>,
) -> Option<String> {
    send(&state, text, on_event)
}

/// Send the last message the maintainer sent again, and stream the reply into `on_event`
///
/// For a turn a quit cut off. The message is read from kelpie's log and goes
/// through the same path as [`pm_send`], so the log gets it again and the
/// session resumes. Nothing in the log to send is a `Failed` on the channel.
/// Returns what [`pm_send`] does.
#[tauri::command(async)]
#[specta::specta]
pub fn pm_retry(state: State<'_, AppState>, on_event: Channel<SessionEvent>) -> Option<String> {
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
            None
        }
        Err(error) => {
            let _ = on_event.send(SessionEvent::Failed {
                message: format!("could not read the PM's log: {error}"),
            });
            None
        }
    }
}

fn send(state: &AppState, text: String, on_event: Channel<SessionEvent>) -> Option<String> {
    let (session, project, divider) = match state.pm_session() {
        Ok(held) => held,
        Err(message) => {
            let _ = on_event.send(SessionEvent::Failed { message });
            return None;
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
    divider
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
mod tests;
