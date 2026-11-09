//! A scripted ACP agent that runs in-process
//!
//! [`FakeAgent`] speaks ACP from a thread of the test's own process, so a test
//! of a [`Session`] runs no process and no model. It answers from a script:
//! what it advertises, which sessions it knows, and the [`Turn`]s it plays,
//! one for each prompt. It records every request it gets ([`FakeAgent::calls`]).
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
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, InitializeRequest,
    InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest,
    NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest, PromptResponse,
    ReadTextFileRequest, RequestPermissionRequest, ResumeSessionRequest, ResumeSessionResponse,
    SessionCapabilities, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelectOption, SessionNotification, SessionResumeCapabilities, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, StopReason, TextContent,
    ToolCall, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, Usage,
};
use agent_client_protocol::{Agent, Channel, Client, DynConnectTo, Error};
use futures::channel::oneshot;
use futures::future::select;
use serde::Serialize;

use super::{Choice, Launch, Link, Session, SessionId};

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
    ToolCall { id: String, title: String },
    ToolUpdate { id: String, status: ToolCallStatus },
    Permission { id: String, title: String },
    UntilCancelled,
    Exit,
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
    known: Vec<String>,
    history: Vec<String>,
    offers: Vec<Offer>,
}

#[derive(Debug, Default)]
struct Shared {
    calls: Mutex<Vec<Call>>,
    turns: Mutex<VecDeque<Turn>>,
    launches: AtomicUsize,
    sessions: AtomicUsize,
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
        Session::with_launcher(Box::new(self.clone()), project, resume, choice, None)
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

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Launch for FakeAgent {
    fn launch(&mut self) -> io::Result<Link> {
        self.shared.launches.fetch_add(1, Ordering::SeqCst);
        let (client_side, agent_side) = Channel::duplex();
        let (leave, left) = oneshot::channel();
        let connection = Arc::new(Connection {
            cancel: Mutex::new(None),
            leave: Mutex::new(Some(leave)),
        });
        let agent = self.clone();
        thread::Builder::new()
            .name("kelpie-fake-agent".to_owned())
            .spawn(move || {
                let serving = Box::pin(agent.serve(agent_side, connection));
                // The agent leaving drops its end, which the client sees as EOF
                let _ = futures::executor::block_on(select(serving, left));
            })?;
        Ok(Link {
            transport: DynConnectTo::new(client_side),
            explain: Box::new(|| None),
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
        let (config, prompt, cancel) = (self.clone(), self.clone(), self.clone());
        let (on_cancel, on_prompt) = (Arc::clone(&connection), connection);
        Agent
            .v1()
            .name("fake")
            .on_receive_request(
                async move |request: InitializeRequest, responder, _cx| {
                    init.record("initialize", &request);
                    let mut capabilities =
                        AgentCapabilities::new().load_session(init.script.can_load);
                    if init.script.can_resume {
                        capabilities = capabilities.session_capabilities(
                            SessionCapabilities::new().resume(SessionResumeCapabilities::new()),
                        );
                    }
                    responder.respond(
                        InitializeResponse::new(ProtocolVersion::V1)
                            .agent_capabilities(capabilities),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async move |request: NewSessionRequest, responder, _cx| {
                    new.record("session/new", &request);
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
                Step::Permission { id, title } => {
                    let call =
                        ToolCallUpdate::new(id.clone(), ToolCallUpdateFields::new().title(title));
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
                    update(SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                        id,
                        ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
                    )))?;
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
}

fn text(text: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
}
