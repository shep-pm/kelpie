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
//! The PM runs inside a crabpot sandbox: it reads the project and the folders
//! beside it, writes its state folder alone, and reaches its model's host
//! (ADR 0005), plus the domains the project's `network-allowlist` names. The
//! `read-anywhere` setting opens its reads to the whole disk, keeping every
//! harness's login, every secret and every other session's state closed, and it
//! lands at the session's next start, since a running sandbox is never widened.
//! `harness-global-folder` links the entries the session's harness reads from
//! the person's own global config folder into the session's config folder.
//!
//! Where the sandbox cannot start (bwrap refuses), the PM runs without one only
//! when the project's `run-without-sandbox` is on, and the chat marks it
//! (`docs/design/sandbox-setup.md`, Running without a sandbox). A project in the
//! home folder is refused whichever way it is set, and workers never run without
//! a sandbox.
//!
//! The PM asks for anything else through kelpie's own tools, and [`pm_answer`]
//! is the maintainer's reply: it widens the running sandbox for the host the PM
//! named, and keeps a *Allow for this project* answer to the project's settings
//! (`docs/design/kelpie-tools.md`, The sandbox request).
//!
//! The PM runs on what the settings resolve to at each send ([`plan`]): the
//! global `pm` agent, then the project's own setting, over [`PM_DEFAULT`], the
//! built-in layer (ADR 0006). Its bridge is installed on the first send, and
//! whatever goes wrong before the session can start reaches the chat as a
//! [`SessionEvent::Failed`], like a failure mid-turn.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use kelpie_engine::agents::{Agents, Binding, Origin, Sourced};
use kelpie_engine::bridge::{self, Pin, Versions};
use kelpie_engine::harness::{HarnessError, claude};
use kelpie_engine::login::Store;
use kelpie_engine::sandbox::{self, Reads, Sandbox, SandboxError};
use kelpie_engine::session::{
    Bridge, Choice, Harness as SessionHarness, LogEntry, SandboxAnswer, SandboxOutcome, Session,
    SessionEvent, Started, Swap, launcher, log, sandboxed_launcher,
};
use kelpie_engine::settings::{
    BridgeUpdates, ConfigDir, Field, Harness, LayerFile, Settings, SettingsError,
};
use kelpie_engine::tools::{Door, SessionTools, Tools};
use semver::Version;
use serde_json::Value;
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

    /// Domains the sandbox lets the PM reach besides its model's host
    /// (`network-allowlist`)
    pub network_allowlist: Vec<String>,

    /// How far the PM reads (`read-anywhere`)
    pub reads: Reads,

    /// Whether the harness reads the person's own global folder (`harness-global-folder`)
    pub harness_global_folder: bool,

    /// Whether the PM may run without a sandbox where none can start
    /// (`run-without-sandbox`)
    pub run_without_sandbox: bool,
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
        network_allowlist: settings.network_allowlist.value,
        reads: Reads::from_setting(settings.read_anywhere.value),
        harness_global_folder: settings.harness_global_folder.value,
        run_without_sandbox: settings.run_without_sandbox.value,
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
/// Installs the bridge if it is not yet, and builds the session on it, so the
/// harness is built from the version [`Versions::current`] names at each send
/// rather than from kelpie's pinned path. Every version is confined in a
/// sandbox of its own, started with the version, so a bridge swapped in at an
/// idle point runs inside crabpot as the first one did. Each sandbox lives as
/// long as the launcher that holds it.
///
/// Before it builds the session it checks the project's folder, then starts
/// the PM's sandbox once and runs a command in it, and records in
/// `opening.sandbox_setup` whether bwrap refused, so the chat can say why. A
/// refusal runs the session without a sandbox when the project allows it
/// (`confinement`), and the home-folder refusal is never lifted.
///
/// # Errors
///
/// The state folder cannot be made, the bridge cannot be installed, the
/// harness is not installed or has no login, or the sandbox cannot start and
/// the project does not allow running without one. The text says which.
pub fn open(opening: &Opening<'_>) -> Result<Session, String> {
    let Opening {
        data_dir,
        project,
        state_dir,
        pm,
        ..
    } = *opening;
    // One token for the session, shared by its sandbox's door and its harness's server
    let tools = opening.tools.map(Tools::session);
    // parity: harness.pm
    let session = match pm.harness {
        Harness::ClaudeCode => {
            fs::create_dir_all(state_dir)
                .map_err(|error| format!("could not make {}: {error}", state_dir.display()))?;
            // kelpie's own pin is always there to fall back to
            let pinned =
                bridge::ensure(data_dir, claude::pin()).map_err(|error| error.to_string())?;
            let confinement = confined(opening, tools.as_ref(), pinned)?;
            Session::on_bridge(
                pm_bridge(opening, tools.clone(), confinement),
                project.path.clone(),
                project.pm_session.clone(),
                pm.choice.clone(),
            )
        }
        // `Harness` grows with each harness's slice, and none but Claude Code is built here
        other => return Err(format!("{other:?} is not wired to the PM yet")),
    };
    let session = match tools {
        Some(tools) => session.with_tools(tools),
        None => session,
    };
    Ok(match &pm.instructions {
        Some(instructions) => session.with_instructions(instructions.clone()),
        None => session,
    })
}

/// The `claude-acp` bridge a PM session runs on, which it can move to a newer
/// version of between turns
///
/// Each version's harness starts inside a sandbox of its own, as the first
/// one's does (ADR 0005), so no version runs outside crabpot, unless the
/// session was opened [`Unsandboxed`](Confinement::Unsandboxed).
fn pm_bridge(
    opening: &Opening<'_>,
    tools: Option<SessionTools>,
    confinement: Confinement,
) -> Bridge {
    let Opening {
        tree,
        data_dir,
        project,
        config,
        ..
    } = *opening;
    let (pm_sandbox, folder) = (PmSandbox::of(opening), project.path.clone());
    let tree = Arc::clone(tree);
    claude_bridge(data_dir, config, &project.id, move |cmd| {
        let tree = Arc::clone(&tree);
        let started = match confinement {
            Confinement::Sandboxed => {
                let (harness, sandbox) = pm_sandbox
                    .start(cmd, tools.as_ref().map(SessionTools::door))
                    .map_err(|error| error.to_string())?;
                Started {
                    launch: sandboxed_launcher(tree, harness.clone(), sandbox, folder.clone()),
                    harness,
                }
            }
            Confinement::Unsandboxed => {
                let harness = pm_sandbox.harness(cmd).map_err(|error| error.to_string())?;
                Started {
                    launch: launcher(tree, harness.clone(), folder.clone()),
                    harness,
                }
            }
        };
        Ok(started)
    })
}

/// The `claude-acp` bridge a session runs on, which it can move to a newer
/// version of between turns, started by `start` on the installed binary of
/// each version
///
/// The PM and a worker share it, and differ only in how `start` builds the
/// harness and its sandbox. Every look at which version to run reads
/// `versions.json` and the settings of the project `project_id`, so a newer
/// version a bridge check installed and probed takes over at the session's next
/// idle point, and a version that fails a session is marked bad there and the
/// one before it is used again (`docs/design/bridge-updates.md`, Swapping).
pub(crate) fn claude_bridge(
    data_dir: &Path,
    config: &ConfigDir,
    project_id: &str,
    start: impl Fn(PathBuf) -> Result<Started, String> + Send + Sync + 'static,
) -> Bridge {
    let pin = claude::pin();
    let (config, project_id) = (config.clone(), project_id.to_owned());
    // Each closure holds its own copy of the folder, since a closure owns what it captures
    let (current_dir, start_dir, failed_dir) = (
        data_dir.to_path_buf(),
        data_dir.to_path_buf(),
        data_dir.to_path_buf(),
    );
    Bridge {
        id: claude::CMD.to_owned(),
        current: Box::new(move || {
            let mut now = versions(&current_dir, pin);
            let was = now.in_use.clone();
            let version = now.current(policy(&config, &project_id, pin.id));
            // Only a look that took a ready version into use swapped anything
            let swapped = version != was;
            if swapped {
                keep(&current_dir, pin.id, &now);
            }
            Swap {
                from: swapped.then_some(was),
                version,
            }
        }),
        start: Box::new(move |version| {
            let cmd = bridge::installed(&start_dir, pin.id, version, claude::CMD)
                .ok_or_else(|| format!("{} {version} is not installed", claude::CMD))?;
            start(cmd)
        }),
        failed: Box::new(move |version, reason| {
            let mut versions = versions(&failed_dir, pin);
            let back = versions.failed(version, reason);
            keep(&failed_dir, pin.id, &versions);
            back
        }),
    }
}

/// What the PM's harness and sandbox are built from
///
/// One value builds both the probe ([`probe`]) and every start of the session,
/// so the sandbox the probe tries is the one the session runs in.
struct PmSandbox {
    state_dir: PathBuf,
    folder: PathBuf,
    sessions_root: PathBuf,
    logins: Store,
    network_allowlist: Vec<String>,
    reads: Reads,
    harness_global_folder: bool,
}

/// Whether the PM's session runs inside its sandbox
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confinement {
    /// Every start of the session is inside a sandbox
    Sandboxed,

    /// The sandbox could not start and the project allows going on without it
    /// (`docs/design/sandbox-setup.md`, Running without a sandbox)
    Unsandboxed,
}

/// Why the PM's harness or its sandbox could not be made
enum Unstarted {
    Harness(HarnessError),
    Sandbox(SandboxError),
}

impl fmt::Display for Unstarted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Harness(error) => error.fmt(f),
            Self::Sandbox(error) => write!(f, "could not start the PM's sandbox: {error}"),
        }
    }
}

impl PmSandbox {
    fn of(opening: &Opening<'_>) -> Self {
        let Opening {
            project,
            state_dir,
            sessions_root,
            logins,
            pm,
            ..
        } = *opening;
        Self {
            state_dir: state_dir.to_path_buf(),
            folder: project.path.clone(),
            sessions_root: sessions_root.to_path_buf(),
            logins: logins.clone(),
            network_allowlist: pm.network_allowlist.clone(),
            reads: pm.reads,
            harness_global_folder: pm.harness_global_folder,
        }
    }

    /// The project the sandbox is asked for
    fn project(&self) -> sandbox::Project<'_> {
        sandbox::Project {
            folder: &self.folder,
            network_allowlist: &self.network_allowlist,
            reads: self.reads,
            sessions_root: &self.sessions_root,
        }
    }

    /// The PM's harness on the bridge `cmd`
    fn harness(&self, cmd: PathBuf) -> Result<SessionHarness, Unstarted> {
        claude::harness(
            cmd,
            &self.state_dir,
            &self.logins,
            self.harness_global_folder,
        )
        .map_err(Unstarted::Harness)
    }

    /// The PM's harness on the bridge `cmd`, and the sandbox it runs in
    fn start(
        &self,
        cmd: PathBuf,
        door: Option<&Door>,
    ) -> Result<(SessionHarness, Sandbox), Unstarted> {
        let harness = self.harness(cmd)?;
        let sandbox = Sandbox::start_with_door(&harness, &self.project(), &self.state_dir, door)
            .map_err(Unstarted::Sandbox)?;
        Ok((harness, sandbox))
    }
}

/// Whether the PM runs in a sandbox, from the folder check and the probe
///
/// The folder check's error always wins, so a project in the home folder is
/// refused whichever way `run_without` is set. A passing probe is
/// [`Confinement::Sandboxed`]. A [`SandboxError::Refused`] is
/// [`Confinement::Unsandboxed`] only when `run_without` is on, since the
/// setting covers a machine that cannot sandbox and nothing else. Every other
/// error stays an error.
///
/// # Errors
///
/// The folder check's error, else the probe's unless it is a refusal the
/// setting covers.
fn confinement(
    folder: Result<(), SandboxError>,
    probe: Result<(), SandboxError>,
    run_without: bool,
) -> Result<Confinement, SandboxError> {
    folder?;
    match probe {
        Ok(()) => Ok(Confinement::Sandboxed),
        Err(SandboxError::Refused(_)) if run_without => Ok(Confinement::Unsandboxed),
        Err(error) => Err(error),
    }
}

/// Decide how the PM runs: check the folder, probe the sandbox, and apply
/// [`confinement`]
///
/// The probe starts the PM's sandbox once, runs a command in it, and drops it,
/// so the session is not built on a sandbox that cannot start. What it finds
/// goes to `opening.sandbox_setup`: a bwrap [`Refusal`] is kept for the chat,
/// along with whether the session runs without a sandbox because of it, and any
/// other outcome clears what an earlier start kept. The probe sits here and not
/// in the bridge's `start` closure, since a failure there on a freshly swapped
/// version reads as that version's fault and marks it bad (`Bridge::start`,
/// `fell_back`).
///
/// # Errors
///
/// The harness cannot start, or [`confinement`] refuses. The text names the
/// refusal when bwrap gave one.
///
/// [`Refusal`]: kelpie_engine::sandbox::Refusal
fn confined(
    opening: &Opening<'_>,
    tools: Option<&SessionTools>,
    pinned: PathBuf,
) -> Result<Confinement, String> {
    let Opening {
        data_dir,
        tree,
        pm,
        sandbox_setup,
        ..
    } = *opening;
    let pm_sandbox = PmSandbox::of(opening);
    let folder = sandbox::check_project(&pm_sandbox.project());
    // A refused folder has no sandbox worth trying, and the harness is not asked either
    let probed = if folder.is_ok() {
        // The version a session starts on now, else kelpie's own pin, which is always there
        let pin = claude::pin();
        let in_use = versions(data_dir, pin).in_use;
        let cmd = bridge::installed(data_dir, pin.id, &in_use, claude::CMD).unwrap_or(pinned);
        match pm_sandbox
            .start(cmd, tools.map(SessionTools::door))
            .and_then(|(_, sandbox)| sandbox.ready(tree).map_err(Unstarted::Sandbox))
        {
            Err(Unstarted::Harness(error)) => {
                sandbox_setup.record(None);
                return Err(Unstarted::Harness(error).to_string());
            }
            Err(Unstarted::Sandbox(error)) => Err(error),
            Ok(()) => Ok(()),
        }
    } else {
        Ok(())
    };
    let refusal = match &probed {
        Err(SandboxError::Refused(refusal)) => Some(*refusal),
        _ => None,
    };
    let decided = confinement(folder, probed, pm.run_without_sandbox);
    match (&decided, refusal) {
        (Ok(Confinement::Unsandboxed), Some(refusal)) => {
            sandbox_setup.record_unsandboxed(refusal);
        }
        (_, refusal) => sandbox_setup.record(refusal),
    }
    decided.map_err(|error| Unstarted::Sandbox(error).to_string())
}

/// The bridge-updates policy for `bridge`, from the project's own settings
///
/// Settings that cannot be read leave the default, which is *Registry, automatic*.
fn policy(config: &ConfigDir, project_id: &str, bridge: &str) -> BridgeUpdates {
    Settings::load(config, project_id).map_or_else(
        |error| {
            eprintln!("kelpie: could not read the settings for {bridge}: {error}");
            BridgeUpdates::default()
        },
        |settings| settings.bridge_updates_for(bridge),
    )
}

/// The versions of `pin`'s bridge, or kelpie's own pin when its file cannot be read
fn versions(data_dir: &Path, pin: &Pin<'_>) -> Versions {
    Versions::load(data_dir, pin).unwrap_or_else(|error| {
        eprintln!("kelpie: {error}");
        Versions {
            // A pin that will not parse is kelpie's own bug. 0.0.0 installs
            // nothing, so the session says the version is missing rather than
            // running some other binary.
            in_use: Version::parse(pin.version).unwrap_or(Version::new(0, 0, 0)),
            previous: None,
            ready: None,
            approved: None,
            held_back: Vec::new(),
            bad: Vec::new(),
        }
    })
}

/// Write `versions` back, saying so on stderr when it cannot be written
fn keep(data_dir: &Path, id: &str, versions: &Versions) {
    if let Err(error) = versions.save(data_dir, id) {
        eprintln!("kelpie: {error}");
    }
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
    // A loaded profile ends the session, so a turn begun now would be cut off
    if state.installing() {
        let _ = on_event.send(SessionEvent::Failed {
            message: "kelpie is installing its sandbox profile: send again once the password prompt closes."
                .to_owned(),
        });
        return None;
    }
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

/// Answer the sandbox request `id`, and say what went wrong when the answer did not land
///
/// `Once` and `Project` let the host through the PM's running sandbox, and the
/// agent hears so in the turn it is still waiting in. `Project` is kept to the
/// project's `network-allowlist` too, where the settings page can remove it, and
/// the PM's next session start builds it into the sandbox. `Deny` changes
/// nothing. A request the turn's end or a cancel already answered is over, and
/// this says so (`docs/design/kelpie-tools.md`, The sandbox request).
///
/// # Errors
///
/// No request is waiting for `id`; the running sandbox could not take the host,
/// which saves nothing; or the host could not be kept to the project, in which
/// case it stays allowed for the session alone. The text is for the maintainer
/// to read.
#[tauri::command(async)]
#[specta::specta]
pub fn pm_answer(
    state: State<'_, AppState>,
    id: String,
    answer: SandboxAnswer,
) -> Result<(), String> {
    let Some(outcome) = state
        .held_session()
        .and_then(|session| session.answer(&id, answer))
    else {
        return Err("That request is over.".to_owned());
    };
    match outcome {
        SandboxOutcome::Allowed { host } => match answer {
            // Only a kept answer has a place outside the session running now
            SandboxAnswer::Project => allow_for_the_project(&state, &host),
            _ => Ok(()),
        },
        // A denial has nothing to keep
        SandboxOutcome::Denied { .. } => Ok(()),
        // The maintainer said yes and the sandbox could not take it, so nothing
        // was widened: the project is not told a host its sessions cannot reach
        SandboxOutcome::NotWidened { host } => Err(not_through(&host)),
    }
}

/// Add `host` to the project's own `network-allowlist`
///
/// A project's file replaces the global list rather than adding to it, so what
/// the PM may already reach is written into it along with `host`. The resolve
/// and the write are one step ([`Settings::update`]), since two answers can be
/// on their way at once and each adds one host to the same list.
///
/// # Errors
///
/// No project is picked, or its settings file cannot be read or written.
fn allow_for_the_project(state: &AppState, host: &str) -> Result<(), String> {
    let id = state.project_id(None)?;
    Settings::update(
        state.config(),
        LayerFile::Project(&id),
        Field::NetworkAllowlist,
        |resolved| {
            let mut hosts = resolved.network_allowlist.value.clone();
            if !hosts.iter().any(|allowed| allowed == host) {
                hosts.push(host.to_owned());
            }
            Value::Array(hosts.into_iter().map(Value::String).collect())
        },
    )
    .map_err(|error| not_saved(host, error))
}

/// Why `host` never reached the sandbox at all, said to the maintainer
///
/// The agent was told the same in its own line. Nothing is saved for the project,
/// so the settings page never lists a host its sessions cannot reach.
fn not_through(host: &str) -> String {
    format!("{host} is not through this session's sandbox, so kelpie saved nothing for it")
}

/// Why `host` could not be kept, said to the maintainer
///
/// The host is through the running sandbox already, so the answer took effect;
/// only the saving did not. The chat shows this on the request's own line.
fn not_saved(host: &str, error: SettingsError) -> String {
    format!(
        "{host} is allowed for this session, but kelpie could not save it to the project: {error}"
    )
}

#[cfg(test)]
mod tests;
