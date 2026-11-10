//! A scripted ACP agent that runs in-process
//!
//! [`FakeAgent`] speaks ACP from a thread of the test's own process, so a test
//! of a [`Session`] runs no harness and no model. It answers from a script:
//! what it advertises, which sessions it knows, and the [`Turn`]s it plays,
//! one for each prompt. It records every request it gets ([`FakeAgent::calls`]).
//!
//! A turn can also run a command in a process group of its own, as a harness's
//! own shell tool does, which is how a test holds that a stop of the run reaches
//! what the harness started ([`Turn::command_in_a_group`]).
//!
//! # Quick start
//!
//! ```
//! use std::sync::mpsc;
//!
//! use kelpie_engine::session::fake::{FakeAgent, Turn};
//! use kelpie_engine::session::{Choice, SessionEvent};
//!
//! let agent = FakeAgent::new().turn(Turn::new().text("Hello"));
//! let session = agent.session(".".into(), None, Choice::default());
//! let (events, heard) = mpsc::channel();
//! session.send("Hi", move |event| {
//!     let _ = events.send(event);
//! });
//! let first = heard.recv()?;
//! assert!(matches!(first, SessionEvent::Started { .. }));
//! assert_eq!(heard.recv()?, SessionEvent::Text { delta: "Hello".to_owned() });
//! # Ok::<(), mpsc::RecvError>(())
//! ```

use std::collections::VecDeque;
#[cfg(unix)]
use std::ffi::OsString;
use std::io;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::Child;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, CloseSessionRequest, CloseSessionResponse, ContentBlock,
    ContentChunk, InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest,
    PromptResponse, ReadTextFileRequest, RequestPermissionOutcome, RequestPermissionRequest,
    ResumeSessionRequest, ResumeSessionResponse, SessionCapabilities, SessionCloseCapabilities,
    SessionConfigOption, SessionConfigOptionCategory, SessionConfigSelectOption,
    SessionNotification, SessionResumeCapabilities, SessionSystemPromptCapabilities, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, StopReason, TextContent,
    ToolCall, ToolCallLocation, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
    Usage,
};
use agent_client_protocol::{Agent, Channel, Client, DynConnectTo, Error, UntypedMessage};
use futures::channel::oneshot;
use futures::future::select;
use serde::Serialize;

use crate::permission::ReadCall;
// Only the unix step uses them: a harness's command, and the run it carries
#[cfg(unix)]
use crate::run::{ENV, RunId};

use super::{Choice, Extension, Harness, Launch, Link, Outdated, Session, SessionId};

mod bridge;
pub use bridge::{BRIDGE, FakeBridge};

/// What a session adds to the `Failed` message when it asks how the agent ended
pub const EXPLANATION: &str = "the fake agent is gone";

/// The name this agent's harness goes by
pub const HARNESS: &str = "fake-agent-acp";

/// One request the agent got, as the client sent it
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The JSON-RPC method, such as `session/new`
    pub method: String,
    /// Its parameters as sent. For `session/request_permission` these are the
    /// client's answer instead.
    pub params: serde_json::Value,
}

/// What an agent does while it plays a turn, in order
#[derive(Debug, Clone)]
enum Step {
    Text(String),
    Thought(String),
    ToolCall {
        id: String,
        title: String,
    },
    ToolUpdate {
        id: String,
        status: ToolCallStatus,
    },
    Permission {
        id: String,
        title: String,
        /// What the call is, which a judge reads
        kind: Option<ToolKind>,
        locations: Vec<PathBuf>,
        raw_input: Option<serde_json::Value>,
    },
    Extension {
        method: String,
        params: serde_json::Value,
    },
    UntilCancelled,
    Exit,
    /// A command run in a process group of its own, carrying the run's mark
    ///
    /// What a harness's own shell tool does: goose puts every command it runs in
    /// a group of its own (ADR 0007), which leaves the harness's group, so only
    /// the mark in its environment reaches it (ADR 0001).
    #[cfg(unix)]
    Command {
        /// What the child carries `KELPIE_RUN` as
        run: String,
        /// The program to run
        program: PathBuf,
        /// Its arguments
        args: Vec<OsString>,
    },
}

/// The reply to one prompt
#[derive(Debug, Clone)]
pub struct Turn {
    steps: Vec<Step>,
    stop: StopReason,
    usage: Option<Usage>,
}

impl Default for Turn {
    fn default() -> Self {
        Self::new()
    }
}

impl Turn {
    /// A turn that ends with `end_turn` and no usage
    #[must_use]
    pub fn new() -> Self {
        Self {
            steps: Vec::new(),
            stop: StopReason::EndTurn,
            usage: None,
        }
    }

    /// Stream a piece of the reply
    #[must_use]
    pub fn text(mut self, delta: &str) -> Self {
        self.steps.push(Step::Text(delta.to_owned()));
        self
    }

    /// Stream a piece of reasoning
    #[must_use]
    pub fn thought(mut self, delta: &str) -> Self {
        self.steps.push(Step::Thought(delta.to_owned()));
        self
    }

    /// Announce a tool call, pending
    #[must_use]
    pub fn tool_call(mut self, id: &str, title: &str) -> Self {
        self.steps.push(Step::ToolCall {
            id: id.to_owned(),
            title: title.to_owned(),
        });
        self
    }

    /// Report a tool call running
    #[must_use]
    pub fn tool_running(mut self, id: &str) -> Self {
        self.steps.push(Step::ToolUpdate {
            id: id.to_owned(),
            status: ToolCallStatus::InProgress,
        });
        self
    }

    /// Report a tool call finished
    #[must_use]
    pub fn tool_done(mut self, id: &str) -> Self {
        self.steps.push(Step::ToolUpdate {
            id: id.to_owned(),
            status: ToolCallStatus::Completed,
        });
        self
    }

    /// Ask the client for permission to run a tool call, offering to allow it
    /// or to reject it, and report the call failed once it is answered
    #[must_use]
    pub fn permission(mut self, id: &str, title: &str) -> Self {
        self.steps.push(Step::Permission {
            id: id.to_owned(),
            title: title.to_owned(),
            kind: None,
            locations: Vec::new(),
            raw_input: None,
        });
        self
    }

    /// As [`Turn::permission`], for a call that writes the file at `path`
    #[must_use]
    pub fn permission_to_write(mut self, id: &str, title: &str, path: &str) -> Self {
        self.steps.push(Step::Permission {
            id: id.to_owned(),
            title: title.to_owned(),
            kind: Some(ToolKind::Edit),
            locations: vec![PathBuf::from(path)],
            raw_input: None,
        });
        self
    }

    /// As [`Turn::permission`], for a call that runs `command` in a shell
    #[must_use]
    pub fn permission_to_run(mut self, id: &str, title: &str, command: &str) -> Self {
        self.steps.push(Step::Permission {
            id: id.to_owned(),
            title: title.to_owned(),
            kind: Some(ToolKind::Execute),
            locations: Vec::new(),
            raw_input: Some(serde_json::json!({ "command": command })),
        });
        self
    }

    /// Send `params` as the extension notification `method` while the turn runs
    #[must_use]
    pub fn extension(mut self, method: &str, params: serde_json::Value) -> Self {
        self.steps.push(Step::Extension {
            method: method.to_owned(),
            params,
        });
        self
    }

    /// Run `program` with `args` in a process group of its own, carrying the
    /// run's mark
    ///
    /// As a harness's own shell tool does: the child is its group's leader, so
    /// it is out of reach of a stop of the harness's group, and only
    /// `KELPIE_RUN=<run>` names it (ADR 0001, ADR 0007). [`FakeAgent::spawned`]
    /// and [`FakeAgent::wait_spawned`] are what a test sees it through.
    ///
    /// Unix only: kelpie has no job object yet (`crate::tree`).
    #[cfg(unix)]
    #[must_use]
    pub fn command_in_a_group(
        mut self,
        run: &RunId,
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Self {
        self.steps.push(Step::Command {
            run: run.to_string(),
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// Wait for `session/cancel`, then end the turn `cancelled`
    #[must_use]
    pub fn until_cancelled(mut self) -> Self {
        self.steps.push(Step::UntilCancelled);
        self
    }

    /// Go away without a word: the agent's end of the connection closes
    #[must_use]
    pub fn exit(mut self) -> Self {
        self.steps.push(Step::Exit);
        self
    }

    /// Report these token counts when the turn ends
    #[must_use]
    pub fn usage(mut self, input: u64, output: u64, cache_read: u64, cache_write: u64) -> Self {
        self.usage = Some(
            Usage::new(input + output + cache_read + cache_write, input, output)
                .cached_read_tokens(cache_read)
                .cached_write_tokens(cache_write),
        );
        self
    }
}

/// A select option the agent offers
#[derive(Debug, Clone)]
struct Offer {
    id: &'static str,
    category: SessionConfigOptionCategory,
    current: String,
    values: Vec<String>,
}

impl Offer {
    fn option(&self) -> SessionConfigOption {
        let values = self
            .values
            .iter()
            .map(|value| SessionConfigSelectOption::new(value.clone(), value.clone()))
            .collect::<Vec<_>>();
        SessionConfigOption::select(self.id, self.id, self.current.clone(), values)
            .category(self.category.clone())
    }
}

/// What the agent advertises and knows
#[derive(Debug, Clone, Default)]
struct Script {
    can_resume: bool,
    can_load: bool,
    can_close: bool,
    can_take_system_prompt: bool,
    silent_on_new: bool,
    known: Vec<String>,
    history: Vec<String>,
    offers: Vec<Offer>,
    extensions: Vec<Extension>,
    /// `_meta` the agent's `session/new` carries
    meta: Option<serde_json::Value>,
    /// What its `initialize` answers with instead of its capabilities
    initialize_error: Option<String>,
    /// What its `session/new` answers with instead of a session
    new_error: Option<String>,
    /// What its harness says when the harness behind its bridge is too old
    outdated: Option<Outdated>,
    /// The mode its harness names to stop its own prompts, and that it runs in
    /// a sandbox, as a sandboxed launcher says
    sandboxed: Option<String>,
    /// The mode its harness names to ask before each tool call
    prompting: Option<String>,
}

#[derive(Debug, Default)]
struct Shared {
    calls: Mutex<Vec<Call>>,
    turns: Mutex<VecDeque<Turn>>,
    launches: AtomicUsize,
    sessions: AtomicUsize,
    stopped: AtomicUsize,
    /// The children a turn started in groups of their own, oldest first
    #[cfg(unix)]
    spawned: Mutex<Vec<Child>>,
}

/// A scripted ACP agent
///
/// Cloning gives another handle on the same agent: the same script, the same
/// record of calls.
#[derive(Debug, Clone, Default)]
pub struct FakeAgent {
    script: Script,
    shared: Arc<Shared>,
}

impl FakeAgent {
    /// An agent that offers nothing and answers every prompt with an empty turn
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advertise `session/resume`
    #[must_use]
    pub fn can_resume(mut self) -> Self {
        self.script.can_resume = true;
        self
    }

    /// Advertise `session/load`
    #[must_use]
    pub fn can_load(mut self) -> Self {
        self.script.can_load = true;
        self
    }

    /// Advertise `sessionCapabilities.close`
    #[must_use]
    pub fn can_close(mut self) -> Self {
        self.script.can_close = true;
        self
    }

    /// Advertise `sessionCapabilities.systemPrompt`
    #[must_use]
    pub fn accepts_system_prompt(mut self) -> Self {
        self.script.can_take_system_prompt = true;
        self
    }

    /// Take `session/new` and never answer it
    #[must_use]
    pub fn never_answers_session_new(mut self) -> Self {
        self.script.silent_on_new = true;
        self
    }

    /// Know a session that a resume or a load may name
    #[must_use]
    pub fn knows(mut self, session_id: &str) -> Self {
        self.script.known.push(session_id.to_owned());
        self
    }

    /// Replay `message` as history when a session is loaded
    #[must_use]
    pub fn replays(mut self, message: &str) -> Self {
        self.script.history.push(message.to_owned());
        self
    }

    /// Offer a `model` option
    #[must_use]
    pub fn offers_models(mut self, current: &str, values: &[&str]) -> Self {
        self.script.offers.push(offer(
            "model",
            SessionConfigOptionCategory::Model,
            current,
            values,
        ));
        self
    }

    /// Offer a `thought_level` option
    #[must_use]
    pub fn offers_efforts(mut self, current: &str, values: &[&str]) -> Self {
        self.script.offers.push(offer(
            "effort",
            SessionConfigOptionCategory::ThoughtLevel,
            current,
            values,
        ));
        self
    }

    /// Offer a `mode` option
    #[must_use]
    pub fn offers_modes(mut self, current: &str, values: &[&str]) -> Self {
        self.script.offers.push(offer(
            "mode",
            SessionConfigOptionCategory::Mode,
            current,
            values,
        ));
        self
    }

    /// Name an extension notification this agent sends that the session should
    /// read, as a harness names its own
    #[must_use]
    pub fn reads(mut self, extension: Extension) -> Self {
        self.script.extensions.push(extension);
        self
    }

    /// Put `meta` on the agent's `session/new`, as a harness's `_meta`
    #[must_use]
    pub fn meta(mut self, meta: serde_json::Value) -> Self {
        self.script.meta = Some(meta);
        self
    }

    /// Answer `initialize` with `message` rather than capabilities
    #[must_use]
    pub fn fails_initialize(mut self, message: &str) -> Self {
        self.script.initialize_error = Some(message.to_owned());
        self
    }

    /// Answer `session/new` with `message` rather than a session
    #[must_use]
    pub fn fails_new(mut self, message: &str) -> Self {
        self.script.new_error = Some(message.to_owned());
        self
    }

    /// Say this agent runs in a sandbox, and its harness stops its own prompts
    /// with `mode`, as a sandboxed harness does
    #[must_use]
    pub fn sandboxed(mut self, mode: &str) -> Self {
        self.script.sandboxed = Some(mode.to_owned());
        self
    }

    /// Say its harness asks before each tool call when it is in `mode`
    #[must_use]
    pub fn prompting(mut self, mode: &str) -> Self {
        self.script.prompting = Some(mode.to_owned());
        self
    }

    /// Say the harness this agent stands for is reported too old with `says`,
    /// and that `update` updates it, as a bridge's harness does
    #[must_use]
    pub fn says_outdated(mut self, says: &[&str], update: &str) -> Self {
        self.script.outdated = Some(Outdated {
            harness: HARNESS.to_owned(),
            says: says.iter().map(|phrase| (*phrase).to_owned()).collect(),
            update: update.to_owned(),
        });
        self
    }

    /// What this agent's harness offers, as a session holds one
    #[must_use]
    pub fn harness(&self) -> Harness {
        Harness {
            program: PathBuf::from(HARNESS),
            args: Vec::new(),
            env: Vec::new(),
            new_session_meta: self.script.meta.clone(),
            extensions: self.script.extensions.clone(),
            hosts: Vec::new(),
            installs: Vec::new(),
            unprompted: self.script.sandboxed.clone(),
            prompting: self.script.prompting.clone(),
            read_call: ReadCall::standard(),
            outdated: self.script.outdated.clone(),
        }
    }

    /// Play `turn` for the next prompt nothing else has claimed
    #[must_use]
    pub fn turn(self, turn: Turn) -> Self {
        lock(&self.shared.turns).push_back(turn);
        self
    }

    /// A session on this agent, with the same signature as [`Session::new`]
    /// but no tree and no process
    #[must_use]
    pub fn session(&self, project: PathBuf, resume: Option<SessionId>, choice: Choice) -> Session {
        Session::with_launcher(
            Box::new(self.clone()),
            self.harness(),
            project,
            resume,
            choice,
        )
    }

    /// Every request the agent got, oldest first
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        lock(&self.shared.calls).clone()
    }

    /// The methods of [`FakeAgent::calls`], oldest first
    #[must_use]
    pub fn methods(&self) -> Vec<String> {
        self.calls().into_iter().map(|call| call.method).collect()
    }

    /// The parameters of each call to `method`, oldest first
    #[must_use]
    pub fn params(&self, method: &str) -> Vec<serde_json::Value> {
        self.calls()
            .into_iter()
            .filter(|call| call.method == method)
            .map(|call| call.params)
            .collect()
    }

    /// How many times a session started the agent
    #[must_use]
    pub fn launches(&self) -> usize {
        self.shared.launches.load(Ordering::SeqCst)
    }

    /// How many times kelpie stopped one of the agent's connections
    #[must_use]
    pub fn stopped(&self) -> usize {
        self.shared.stopped.load(Ordering::SeqCst)
    }

    /// The pid of every child a turn started with [`Turn::command_in_a_group`],
    /// oldest first
    #[cfg(unix)]
    #[must_use]
    pub fn spawned(&self) -> Vec<u32> {
        lock(&self.shared.spawned).iter().map(Child::id).collect()
    }

    /// Wait for every child a turn started to exit, oldest first
    ///
    /// Takes them out of the agent, so a second call waits for nothing.
    ///
    /// # Errors
    ///
    /// The OS cannot report a child's exit.
    #[cfg(unix)]
    pub fn wait_spawned(&self) -> Vec<io::Result<std::process::ExitStatus>> {
        lock(&self.shared.spawned)
            .drain(..)
            .map(|mut child| child.wait())
            .collect()
    }

    fn record(&self, method: &str, params: &impl Serialize) {
        let params = serde_json::to_value(params).unwrap_or(serde_json::Value::Null);
        lock(&self.shared.calls).push(Call {
            method: method.to_owned(),
            params,
        });
    }

    fn options(&self) -> Vec<SessionConfigOption> {
        self.script.offers.iter().map(Offer::option).collect()
    }

    fn check_known(&self, session_id: &str) -> Result<(), Error> {
        if self.script.known.iter().any(|known| known == session_id) {
            Ok(())
        } else {
            Err(
                agent_client_protocol::v1::schema::Error::session_not_found(session_id.to_owned())
                    .into(),
            )
        }
    }
}

fn offer(
    id: &'static str,
    category: SessionConfigOptionCategory,
    current: &str,
    values: &[&str],
) -> Offer {
    Offer {
        id,
        category,
        current: current.to_owned(),
        values: values.iter().map(|value| (*value).to_owned()).collect(),
    }
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Launch for FakeAgent {
    fn sandboxed(&self) -> bool {
        self.script.sandboxed.is_some()
    }

    fn launch(&mut self) -> io::Result<Link> {
        self.shared.launches.fetch_add(1, Ordering::SeqCst);
        let (client_side, agent_side) = Channel::duplex();
        let (leave, left) = oneshot::channel();
        let connection = Arc::new(Connection {
            cancel: Mutex::new(None),
            leave: Mutex::new(Some(leave)),
        });
        let agent = self.clone();
        let shared = Arc::clone(&self.shared);
        let gone = Arc::clone(&connection);
        thread::Builder::new()
            .name("kelpie-fake-agent".to_owned())
            .spawn(move || {
                let serving = Box::pin(agent.serve(agent_side, connection));
                // The agent leaving drops its end, which the client sees as EOF
                let _ = futures::executor::block_on(select(serving, left));
            })?;
        Ok(Link {
            transport: DynConnectTo::new(client_side),
            explain: Box::new(|| Some(EXPLANATION.to_owned())),
            stop: Box::new(move |_| {
                if let Some(leave) = lock(&gone.leave).take() {
                    let _ = leave.send(());
                }
                shared.stopped.fetch_add(1, Ordering::SeqCst);
                Some(EXPLANATION.to_owned())
            }),
        })
    }
}

/// What one launch of the agent shares between its handlers
struct Connection {
    /// Answers the turn that waits for `session/cancel`
    cancel: Mutex<Option<oneshot::Sender<()>>>,
    /// Makes the agent go away
    leave: Mutex<Option<oneshot::Sender<()>>>,
}

impl FakeAgent {
    async fn serve(self, channel: Channel, connection: Arc<Connection>) -> Result<(), Error> {
        let (init, new, load, resume) = (self.clone(), self.clone(), self.clone(), self.clone());
        let (config, prompt, cancel, close) =
            (self.clone(), self.clone(), self.clone(), self.clone());
        let (on_cancel, on_prompt) = (Arc::clone(&connection), connection);
        Agent
            .v1()
            .name("fake")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    init.record("initialize", &request);
                    if let Some(message) = &init.script.initialize_error {
                        return responder
                            .respond_with_error(Error::invalid_request().data(message.clone()));
                    }
                    let mut offered = SessionCapabilities::new();
                    if init.script.can_resume {
                        offered = offered.resume(SessionResumeCapabilities::new());
                    }
                    if init.script.can_take_system_prompt {
                        offered = offered.system_prompt(SessionSystemPromptCapabilities::new());
                    }
                    if init.script.can_close {
                        offered = offered.close(SessionCloseCapabilities::new());
                    }
                    let capabilities = AgentCapabilities::new()
                        .load_session(init.script.can_load)
                        .session_capabilities(offered);
                    responder.respond(
                        InitializeResponse::new(ProtocolVersion::V1)
                            .agent_capabilities(capabilities),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: NewSessionRequest, responder, cx: ConnectionTo<Client>| {
                    new.record("session/new", &request);
                    if new.script.silent_on_new {
                        // Held, not dropped, so the client waits for an answer that never comes
                        return cx.spawn(async move {
                            let _held = responder;
                            futures::future::pending::<Result<(), Error>>().await
                        });
                    }
                    if let Some(message) = &new.script.new_error {
                        return responder
                            .respond_with_error(Error::invalid_request().data(message.clone()));
                    }
                    let number = new.shared.sessions.fetch_add(1, Ordering::SeqCst) + 1;
                    responder.respond(
                        NewSessionResponse::new(format!("fake-{number}"))
                            .config_options(new.options()),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: ResumeSessionRequest, responder, _cx| {
                    resume.record("session/resume", &request);
                    match resume.check_known(&request.session_id.0) {
                        Ok(()) => responder
                            .respond(ResumeSessionResponse::new().config_options(resume.options())),
                        Err(error) => responder.respond_with_error(error),
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: LoadSessionRequest, responder, cx: ConnectionTo<Client>| {
                    load.record("session/load", &request);
                    match load.check_known(&request.session_id.0) {
                        Ok(()) => {
                            for message in &load.script.history {
                                cx.send_notification(SessionNotification::new(
                                    request.session_id.clone(),
                                    SessionUpdate::AgentMessageChunk(text(message)),
                                ))?;
                            }
                            responder
                                .respond(LoadSessionResponse::new().config_options(load.options()))
                        }
                        Err(error) => responder.respond_with_error(error),
                    }
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: CloseSessionRequest, responder, _cx| {
                    close.record("session/close", &request);
                    responder.respond(CloseSessionResponse::new())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: SetSessionConfigOptionRequest, responder, _cx| {
                    config.record("session/set_config_option", &request);
                    responder.respond(SetSessionConfigOptionResponse::new(config.options()))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: PromptRequest, responder, cx: ConnectionTo<Client>| {
                    prompt.record("session/prompt", &request);
                    let turn = lock(&prompt.shared.turns).pop_front().unwrap_or_default();
                    let (cancel, cancelled) = oneshot::channel();
                    *lock(&on_prompt.cancel) = Some(cancel);
                    let connection = Arc::clone(&on_prompt);
                    let agent = prompt.clone();
                    // A turn that waits must not hold the dispatch loop, or
                    // the cancel it waits for could never arrive
                    cx.clone().spawn(async move {
                        let result = agent
                            .play(turn, request.session_id, &cx, cancelled, &connection)
                            .await;
                        responder.respond_with_result(result)
                    })
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(
                async move |notification: CancelNotification, _cx| {
                    cancel.record("session/cancel", &notification);
                    if let Some(cancel) = lock(&on_cancel.cancel).take() {
                        let _ = cancel.send(());
                    }
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            )
            .connect_to(channel)
            .await
    }

    async fn play(
        &self,
        turn: Turn,
        session_id: agent_client_protocol::v1::schema::SessionId,
        cx: &ConnectionTo<Client>,
        cancelled: oneshot::Receiver<()>,
        connection: &Connection,
    ) -> Result<PromptResponse, Error> {
        let update =
            |update| cx.send_notification(SessionNotification::new(session_id.clone(), update));
        let mut cancelled = Some(cancelled);
        for step in turn.steps {
            match step {
                Step::Text(delta) => update(SessionUpdate::AgentMessageChunk(text(&delta)))?,
                Step::Thought(delta) => update(SessionUpdate::AgentThoughtChunk(text(&delta)))?,
                Step::ToolCall { id, title } => {
                    update(SessionUpdate::ToolCall(ToolCall::new(id, title)))?
                }
                Step::ToolUpdate { id, status } => update(SessionUpdate::ToolCallUpdate(
                    ToolCallUpdate::new(id, ToolCallUpdateFields::new().status(status)),
                ))?,
                Step::Permission {
                    id,
                    title,
                    kind,
                    locations,
                    raw_input,
                } => {
                    let mut fields = ToolCallUpdateFields::new().title(title).kind(kind);
                    if !locations.is_empty() {
                        fields.locations =
                            Some(locations.into_iter().map(ToolCallLocation::new).collect());
                    }
                    fields.raw_input = raw_input;
                    let call = ToolCallUpdate::new(id.clone(), fields);
                    let options = vec![
                        PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
                        PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
                    ];
                    let answer = cx
                        .send_request(RequestPermissionRequest::new(
                            session_id.clone(),
                            call,
                            options,
                        ))
                        .block_task()
                        .await?;
                    self.record("session/request_permission", &answer);
                    // A call that was allowed runs, and one that was not fails
                    let allowed = matches!(
                        &answer.outcome,
                        RequestPermissionOutcome::Selected(chosen)
                            if chosen.option_id.0.as_ref() == "allow"
                    );
                    let status = if allowed {
                        ToolCallStatus::Completed
                    } else {
                        ToolCallStatus::Failed
                    };
                    update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                        id,
                        ToolCallUpdateFields::new().status(status),
                    )))?;
                }
                Step::Extension { method, params } => {
                    let message = UntypedMessage::new(&method, &params)?;
                    cx.send_notification(message)?
                }
                #[cfg(unix)]
                Step::Command { run, program, args } => {
                    self.spawn_command(&run, &program, &args)?
                }
                Step::UntilCancelled => {
                    if let Some(cancelled) = cancelled.take() {
                        let _ = cancelled.await;
                    }
                    return Ok(PromptResponse::new(StopReason::Cancelled));
                }
                Step::Exit => {
                    // The client answers in the order it reads, so an answer
                    // shows it has read everything sent so far
                    let _ = cx
                        .send_request(ReadTextFileRequest::new(session_id.clone(), "."))
                        .block_task()
                        .await;
                    if let Some(leave) = lock(&connection.leave).take() {
                        let _ = leave.send(());
                    }
                    return std::future::pending().await;
                }
            }
        }
        Ok(PromptResponse::new(turn.stop).usage(turn.usage))
    }

    /// Start `program args` in a group of its own, carrying the run's mark
    ///
    /// The group is the point: the child leaves the group a stop of the harness
    /// reaches, and only `KELPIE_RUN` names it (ADR 0001, ADR 0007).
    #[cfg(unix)]
    fn spawn_command(&self, run: &str, program: &Path, args: &[OsString]) -> Result<(), Error> {
        use std::os::unix::process::CommandExt as _;

        let mut command = std::process::Command::new(program);
        command.args(args).env(ENV, run).process_group(0);
        let child = command.spawn().map_err(|error| {
            Error::internal_error().data(serde_json::Value::from(format!(
                "the fake could not run {}: {error}",
                program.display()
            )))
        })?;
        lock(&self.shared.spawned).push(child);
        Ok(())
    }
}

fn text(text: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
}
