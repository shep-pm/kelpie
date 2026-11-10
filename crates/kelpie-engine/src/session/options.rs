//! The brief start that asks a harness what it offers
//!
//! `initialize`, `session/new`, the config options it answers with, and
//! `session/close` where the harness offers one, then the stop ladder. No prompt
//! is sent, so no model runs and nothing is spent (`docs/design/settings.md`).
//! The same start is what probes a new bridge (`docs/design/bridge-updates.md`).
//!
//! Kelpie keeps what each harness was last asked at
//! `<data_dir>/options/<id>.json` and answers from there until someone asks for
//! a refresh, so opening the settings page starts no harness.

use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    CloseSessionRequest, InitializeRequest, InitializeResponse, SessionConfigKind,
    SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    SessionConfigSelectOptions,
};
use agent_client_protocol::{Agent, Client, DynConnectTo, Error};
use futures::future::{Either, select};
use serde::{Deserialize, Serialize};

use kelpie_protocol::{Choice, Options};

use crate::bridge::is_name;
use crate::sandbox::Sandbox;
use crate::tree::ProcessTree;

use super::acp::{describe, is_gone, new_session};
use super::{Harness, Launch, Link, ProcessLauncher, SessionError};

/// The folder under the data dir that holds what each harness was last asked
const OPTIONS_FOLDER: &str = "options";

/// How long a harness kelpie has asked is given to exit before its group is killed
///
/// One that has had its session closed and its input closed exits at once; 5 s
/// covers a bridge still waiting on the harness behind it.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// How long a harness is given to answer `initialize` and `session/new`
///
/// A bridge that has to reach the network or wait on a login can be slow, so
/// this is generous, and a harness that is still silent is stopped instead of
/// waited on for good.
const ANSWER_WITHIN: Duration = Duration::from_secs(60);

/// Ask `harness` what it offers, and stop it
///
/// Starts it through `tree` inside `sandbox` (ADR 0005), sends `initialize` and
/// `session/new` in `project`, reads the config options it answers with, closes
/// the session and stops the process. It sends no prompt, so no model runs and
/// nothing is spent. This blocks until the harness has answered or is gone, and
/// names no model or effort of its own: the harness's defaults stand. The
/// sandbox lasts until the ask returns.
///
/// # Errors
///
/// [`SessionError`] says whether the harness could not be started, ended the
/// connection before answering, or answered with an error.
pub fn ask(
    tree: &Arc<ProcessTree>,
    harness: &Harness,
    sandbox: Sandbox,
    project: &Path,
) -> Result<Options, SessionError> {
    let mut launcher = launcher(tree, harness, sandbox, project);
    asked(&mut launcher, project, harness.new_session_meta.as_ref())
}

/// [`ask`], on a harness `launcher` starts instead of one from the process tree
pub(crate) fn asked(
    launcher: &mut dyn Launch,
    project: &Path,
    meta: Option<&serde_json::Value>,
) -> Result<Options, SessionError> {
    asked_within(launcher, project, meta, ANSWER_WITHIN)
}

/// [`asked`], giving the harness `deadline` to answer before it is stopped
pub(crate) fn asked_within(
    launcher: &mut dyn Launch,
    project: &Path,
    meta: Option<&serde_json::Value>,
    deadline: Duration,
) -> Result<Options, SessionError> {
    asking(launcher, deadline, async |cx| {
        talk(cx, project.to_path_buf(), meta).await
    })
}

/// What a harness answers to `initialize` alone, and then it is stopped
///
/// Starts it through `launcher`, sends `initialize` and no `session/new`, no
/// prompt, and stops it, giving it `deadline` to answer. This is the probe's
/// start (`docs/design/bridge-updates.md`, The probe).
pub(crate) fn initialized_within(
    launcher: &mut dyn Launch,
    deadline: Duration,
) -> Result<InitializeResponse, SessionError> {
    asking(launcher, deadline, async |cx| {
        cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
            .block_task()
            .await
    })
}

/// Start the harness, run `talk` on its connection, and stop it
///
/// The harness is stopped whether `talk` finished, failed or ran past `deadline`.
async fn asking_with<T>(
    transport: DynConnectTo<Client>,
    talk: impl AsyncFnOnce(&ConnectionTo<Agent>) -> Result<T, Error>,
) -> Result<T, Failure> {
    Client
        .v1()
        .name("kelpie")
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            Ok(match talk(&cx).await {
                Ok(done) => Ok(done),
                Err(error) => {
                    let gone = is_gone(&error)
                        || cx.is_incoming_closed()
                        || until(CLOSE_FOLLOWS, cx.incoming_closed()).await.is_some();
                    Err(Failure { error, gone })
                }
            })
        })
        .await
        // The connection failing outside the ask, such as a write to a pipe
        // nobody reads, is the harness going; its own errors arrive inside
        .map_err(|error| Failure { gone: true, error })
        .and_then(|asked| asked)
}

/// [`asking_with`] on a harness `launcher` starts, naming how it failed
fn asking<T>(
    launcher: &mut dyn Launch,
    deadline: Duration,
    talk: impl AsyncFnOnce(&ConnectionTo<Agent>) -> Result<T, Error>,
) -> Result<T, SessionError> {
    let Link {
        transport, stop, ..
    } = launcher.launch().map_err(SessionError::Start)?;
    let answer = futures::executor::block_on(until(deadline, asking_with(transport, talk)));
    // The harness is kelpie's to stop, whether it closed its own session or not
    let said = stop(STOP_GRACE);
    match answer {
        None => Err(SessionError::TimedOut {
            after: deadline,
            message: said.unwrap_or_default(),
        }),
        Some(answer) => answer.map_err(|Failure { error, gone }| match (gone, said) {
            (true, Some(said)) => SessionError::Ended {
                message: format!("{}: {said}", describe(&error)),
            },
            (true, None) => SessionError::Ended {
                message: describe(&error),
            },
            (false, _) => SessionError::Refused {
                message: describe(&error),
            },
        }),
    }
}

/// What `work` gives, or `None` when `deadline` passes first, and then `work` is dropped
///
/// The clock is a thread that wakes when the work is over, so a finished ask
/// leaves nothing sleeping.
async fn until<T>(deadline: Duration, work: impl Future<Output = T>) -> Option<T> {
    let (over, passed) = futures::channel::oneshot::channel();
    let (finished, finish) = mpsc::channel::<()>();
    let clock = thread::Builder::new()
        .name("kelpie-ask-deadline".to_owned())
        .spawn(move || {
            if finish.recv_timeout(deadline) == Err(mpsc::RecvTimeoutError::Timeout) {
                let _ = over.send(());
            }
        });
    // A clock that cannot start leaves the ask as it was: it waits for the harness
    let outcome = if clock.is_ok() {
        match select(pin!(work), passed).await {
            Either::Left((done, _)) => Some(done),
            Either::Right(_) => None,
        }
    } else {
        Some(work.await)
    };
    drop(finished);
    outcome
}

/// What harness `id` offers: what kelpie was last told, or a fresh ask
///
/// The answer kept under `data_dir` answers until `refresh` is set. A fresh one
/// is asked as [`ask`] does, inside `sandbox`, and is kept at
/// `options/<id>.json` for the next time. Building the harness has its own cost,
/// such as installing its bridge, so a caller that must start nothing reads
/// [`kept`] before it builds one.
///
/// # Errors
///
/// [`SessionError`], as [`ask`], or [`SessionError::Name`] when `id` names no
/// file, or [`SessionError::Cache`] when the answer cannot be written.
pub fn offerings(
    tree: &Arc<ProcessTree>,
    harness: &Harness,
    id: &str,
    sandbox: Sandbox,
    data_dir: &Path,
    project: &Path,
    refresh: bool,
) -> Result<Options, SessionError> {
    let mut launcher = launcher(tree, harness, sandbox, project);
    offerings_at(
        &mut launcher,
        id,
        data_dir,
        project,
        harness.new_session_meta.as_ref(),
        refresh,
    )
}

/// [`offerings`], asking through `launcher` instead of the process tree
pub(crate) fn offerings_at(
    launcher: &mut dyn Launch,
    id: &str,
    data_dir: &Path,
    project: &Path,
    meta: Option<&serde_json::Value>,
    refresh: bool,
) -> Result<Options, SessionError> {
    if !is_name(id) {
        return Err(SessionError::Name {
            name: id.to_owned(),
        });
    }
    if !refresh && let Some(kept) = kept(data_dir, id) {
        return Ok(kept);
    }
    let options = asked(launcher, project, meta)?;
    keep(data_dir, id, &options)?;
    Ok(options)
}

/// The launcher that starts `harness` through `tree` inside `sandbox`, in `project`
pub(crate) fn launcher(
    tree: &Arc<ProcessTree>,
    harness: &Harness,
    sandbox: Sandbox,
    project: &Path,
) -> impl Launch {
    ProcessLauncher {
        tree: Arc::clone(tree),
        harness: harness.clone(),
        sandbox: Some(sandbox),
        project: project.to_path_buf(),
    }
}

/// Why an ask failed, and whether the harness itself ended
struct Failure {
    error: Error,
    /// The harness ended the connection, as opposed to answering with an error
    gone: bool,
}

/// How long to wait for a failed write to be followed by the harness's output closing
///
/// A harness that exited fails the write that reaches it, and its closed output
/// follows a moment after. A harness that is alive and answered with an error
/// never closes, so this is the most that failure costs.
const CLOSE_FOLLOWS: Duration = Duration::from_millis(200);

/// What the harness lists, from `initialize`, `session/new` and `session/close` on `cx`
async fn talk(
    cx: &ConnectionTo<Agent>,
    project: PathBuf,
    meta: Option<&serde_json::Value>,
) -> Result<Options, Error> {
    // A harness answers nothing before `initialize`
    let init = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let session = cx
        .send_request(new_session(project, meta)?)
        .block_task()
        .await?;
    let listed = listed(&session.config_options.unwrap_or_default());
    if init.agent_capabilities.session_capabilities.close.is_some() {
        // A harness that answers the close with an error is stopped all the same
        let _ = cx
            .send_request(CloseSessionRequest::new(session.session_id))
            .block_task()
            .await;
    }
    Ok(listed)
}

/// The models and efforts among the options a harness answered with
fn listed(options: &[SessionConfigOption]) -> Options {
    Options {
        models: choices(options, SessionConfigOptionCategory::Model),
        efforts: choices(options, SessionConfigOptionCategory::ThoughtLevel),
    }
}

/// The choices under `category`, in the order the harness lists them
///
/// A grouped selector is read in group order, and an option that is not a
/// selector lists nothing.
fn choices(options: &[SessionConfigOption], category: SessionConfigOptionCategory) -> Vec<Choice> {
    options
        .iter()
        .filter(|option| option.category.as_ref() == Some(&category))
        .flat_map(|option| match &option.kind {
            SessionConfigKind::Select(select) => match &select.options {
                SessionConfigSelectOptions::Ungrouped(list) => list.iter().map(choice).collect(),
                SessionConfigSelectOptions::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| &group.options)
                    .map(choice)
                    .collect(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        })
        .collect()
}

/// One listed value, with the label and the note the harness gives it
fn choice(option: &SessionConfigSelectOption) -> Choice {
    Choice {
        id: option.value.0.to_string(),
        name: option.name.clone(),
        description: option.description.clone(),
    }
}

/// What a harness was last asked, as kept in the data folder
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    /// When kelpie asked, as whole seconds since the Unix epoch
    asked_at: u64,
    /// What it answered
    #[serde(flatten)]
    options: Options,
}

/// The answer kept for harness `id`, with no harness started
///
/// `None` when there is none kelpie can use, including a file it cannot read or
/// a file it cannot make sense of: a cache that is wrong is asked over again.
pub fn kept(data_dir: &Path, id: &str) -> Option<Options> {
    let text = fs::read_to_string(file(data_dir, id)?).ok()?;
    serde_json::from_str::<Kept>(&text)
        .ok()
        .map(|kept| kept.options)
}

/// Keep `options` as harness `id`'s answer, with the time it was asked
fn keep(data_dir: &Path, id: &str, options: &Options) -> Result<(), SessionError> {
    let path = file(data_dir, id).ok_or_else(|| SessionError::Name {
        name: id.to_owned(),
    })?;
    let kept = Kept {
        asked_at: asked_now(),
        options: options.clone(),
    };
    let written = serde_json::to_vec(&kept)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        .and_then(|text| {
            fs::create_dir_all(data_dir.join(OPTIONS_FOLDER))?;
            fs::write(&path, text)
        });
    match written {
        Ok(()) => Ok(()),
        Err(source) => Err(SessionError::Cache { path, source }),
    }
}

/// The file harness `id`'s answer is kept in, `None` when `id` names no file
fn file(data_dir: &Path, id: &str) -> Option<PathBuf> {
    is_name(id).then(|| data_dir.join(OPTIONS_FOLDER).join(format!("{id}.json")))
}

/// This moment, as whole seconds since the Unix epoch
fn asked_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}
