//! The ACP client behind a session
//!
//! One worker thread drives one connection: `initialize`, a session, then a
//! turn for each message. Updates that arrive during a turn become
//! [`SessionEvent`]s in the order they came, because the connection hands
//! them over one at a time and before the turn's response.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::ConnectionTo;
use agent_client_protocol::v1::schema::{
    CancelNotification, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    LoadSessionRequest, NewSessionRequest, PermissionOptionKind, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption,
    SessionConfigOptionCategory, SessionConfigSelectOptions, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, StopReason as AcpStopReason, SystemPrompt, TextContent,
    ToolCallStatus,
};
use agent_client_protocol::{
    Agent, Client, Error, UntypedMessage, is_connection_terminated, is_incoming_transport_closed,
    on_receive_notification, on_receive_request,
};
use futures::StreamExt as _;
use futures::future::{Either, select};
use kelpie_protocol::{CacheLifetime, StopReason, ToolStatus, Usage};

use super::raw;
use super::seed;
use super::{
    Choice, Command, Extension, Inbox, Launch, Link, STOP_GRACE, SessionEvent, SessionId, Setup,
    Sink, lock,
};

/// The divider a refused resume leaves in the log
pub const COULDNT_RESUME: &str = "Couldn't resume, started a new session";

/// Run one worker: wait for the first send, start the harness, serve turns
/// until the session is dropped or the harness is gone
pub(super) async fn run(
    launcher: Arc<Mutex<Box<dyn Launch>>>,
    setup: Setup,
    mut commands: Inbox,
    id: Arc<Mutex<Option<SessionId>>>,
) {
    let shared = Arc::new(Shared::default());
    let mut turns: Turns = VecDeque::new();
    let first = loop {
        match commands.next().await {
            Some(Command::Send { text, sink }) => break (text, sink),
            Some(Command::Cancel) => {}
            None => return,
        }
    };
    shared.start_turn(first.1);

    let link = lock(&launcher).launch();
    let mut stop = None;
    match link {
        Err(error) => {
            commands.close();
            shared.emit(failed(format!("could not start the harness: {error}")));
        }
        Ok(Link {
            transport,
            mut explain,
            stop: stopper,
        }) => {
            stop = Some(stopper);
            let result = connect(
                &shared,
                transport,
                &setup,
                first.0,
                &mut turns,
                &mut commands,
                &id,
            )
            .await;
            if let Err(error) = result {
                // From here a send finds the inbox closed and starts a new
                // worker, which can take a while to explain
                commands.close();
                let mut message = describe(&error);
                if !shared.state().request_failed
                    && let Some(explained) = explain()
                {
                    message = format!("{message}: {explained}");
                }
                shared.emit(failed(message));
            }
        }
    }

    // A send that raced the end of this worker is answered, not dropped
    commands.close();
    for (_, sink) in turns {
        if let Some(mut sink) = sink {
            sink(failed(ENDED_BEFORE.to_owned()));
        }
    }
    while let Ok(command) = commands.try_recv() {
        if let Command::Send { mut sink, .. } = command {
            sink(failed(ENDED_BEFORE.to_owned()));
        }
    }
    // Whatever ended the worker, the harness stops with it: a session that is
    // replaced or dropped leaves nothing running
    if let Some(stop) = stop {
        stop(STOP_GRACE);
    }
}

const ENDED_BEFORE: &str = "the session ended before this message ran";

/// Messages waiting for the turn in flight, each with the sink it streams to
type Turns = VecDeque<(String, Option<Sink>)>;

fn failed(message: String) -> SessionEvent {
    SessionEvent::Failed { message }
}

pub(super) fn describe(error: &Error) -> String {
    if is_connection_terminated(error) || is_incoming_transport_closed(error) {
        "the harness ended the connection".to_owned()
    } else {
        error.to_string()
    }
}

/// What the connection's handlers share with the worker
#[derive(Default)]
struct Shared(Mutex<State>);

#[derive(Default)]
struct State {
    /// The turn's sink, `None` between turns
    sink: Option<Sink>,
    /// Updates are dropped: the agent is replaying a loaded session
    replaying: bool,
    /// Each tool call's title and status as last sent
    steps: HashMap<String, (String, ToolStatus)>,
    /// Calls kelpie refused, which stay refused when the agent reports them failed
    refused: HashSet<String>,
    /// A request failed on a connection that still works, such as an error the
    /// harness answered with, so the harness's end is not what to explain
    request_failed: bool,
    /// How long this turn's prompt cache write lasts, from a harness that says
    cache_lifetime: Option<CacheLifetime>,
}

impl Shared {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set_sink(&self, sink: Option<Sink>) {
        self.state().sink = sink;
    }

    /// A turn begins: its events go to `sink`, and what its raw messages said starts empty
    fn start_turn(&self, sink: Sink) {
        let mut state = self.state();
        state.sink = Some(sink);
        state.cache_lifetime = None;
    }

    /// The turn is over: its usage, with whatever its raw messages said about the cache
    fn turn_ended(&self, response: PromptResponse) -> SessionEvent {
        let lifetime = self.state().cache_lifetime.take();
        SessionEvent::TurnEnded {
            stop_reason: stop_reason(response.stop_reason),
            usage: response.usage.map(|usage| Usage {
                input: usage.input_tokens,
                output: usage.output_tokens,
                cache_read: usage.cached_read_tokens.unwrap_or(0),
                cache_write: usage.cached_write_tokens.unwrap_or(0),
                cache_lifetime: lifetime,
            }),
        }
    }

    /// Fold one extension notification the harness named into the turn in flight
    fn read_extension(&self, read: &[Extension], method: &str, params: &serde_json::Value) {
        let Some(extension) = read.iter().find(|extension| extension.method() == method) else {
            return;
        };
        if let Some(lifetime) = raw::cache_lifetime(*extension, params) {
            self.state().cache_lifetime = Some(lifetime);
        }
    }

    fn set_replaying(&self, replaying: bool) {
        self.state().replaying = replaying;
    }

    fn emit(&self, event: SessionEvent) {
        if let Some(sink) = self.state().sink.as_mut() {
            sink(event);
        }
    }

    fn update(&self, update: SessionUpdate) {
        let mut state = self.state();
        if state.replaying {
            return;
        }
        let event = match update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                text_of(chunk).map(|delta| SessionEvent::Text { delta })
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                text_of(chunk).map(|delta| SessionEvent::Thought { delta })
            }
            SessionUpdate::ToolCall(call) => {
                let id = call.tool_call_id.0.to_string();
                state.step(id, Some(call.title), Some(call.status))
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let id = update.tool_call_id.0.to_string();
                state.step(id, update.fields.title, update.fields.status)
            }
            _ => None,
        };
        if let (Some(event), Some(sink)) = (event, state.sink.as_mut()) {
            sink(event);
        }
    }

    /// Refuse a permission request: the reject option, and a refused step
    fn refuse(&self, request: &RequestPermissionRequest) -> RequestPermissionOutcome {
        let id = request.tool_call.tool_call_id.0.to_string();
        let mut state = self.state();
        state.refused.insert(id.clone());
        let event = state.step(id, request.tool_call.fields.title.clone(), None);
        if let (Some(event), Some(sink)) = (event, state.sink.as_mut()) {
            sink(event);
        }
        [
            PermissionOptionKind::RejectOnce,
            PermissionOptionKind::RejectAlways,
        ]
        .iter()
        .find_map(|kind| request.options.iter().find(|option| option.kind == *kind))
        .map_or(RequestPermissionOutcome::Cancelled, |option| {
            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            ))
        })
    }
}

impl State {
    /// Fold a call's new title and status into what is known, and say what
    /// changed, `None` when nothing did
    fn step(
        &mut self,
        id: String,
        title: Option<String>,
        status: Option<ToolCallStatus>,
    ) -> Option<SessionEvent> {
        let refused = self.refused.contains(&id);
        let (known_title, known_status) = self
            .steps
            .get(&id)
            .cloned()
            .unwrap_or_else(|| (String::new(), ToolStatus::Pending));
        let title = title.unwrap_or_else(|| known_title.clone());
        let status = if refused {
            ToolStatus::Refused
        } else {
            status.map_or(known_status, tool_status)
        };
        let changed = self.steps.get(&id) != Some(&(title.clone(), status));
        self.steps.insert(id.clone(), (title.clone(), status));
        changed.then_some(SessionEvent::ToolStep { id, title, status })
    }
}

fn text_of(chunk: ContentChunk) -> Option<String> {
    match chunk.content {
        ContentBlock::Text(text) => Some(text.text),
        _ => None,
    }
}

fn tool_status(status: ToolCallStatus) -> ToolStatus {
    match status {
        ToolCallStatus::Pending => ToolStatus::Pending,
        ToolCallStatus::Completed => ToolStatus::Done,
        ToolCallStatus::Failed => ToolStatus::Failed,
        _ => ToolStatus::Running,
    }
}

fn stop_reason(reason: AcpStopReason) -> StopReason {
    match reason {
        AcpStopReason::MaxTokens => StopReason::MaxTokens,
        AcpStopReason::MaxTurnRequests => StopReason::MaxTurnRequests,
        AcpStopReason::Refusal => StopReason::Refusal,
        AcpStopReason::Cancelled => StopReason::Cancelled,
        _ => StopReason::EndTurn,
    }
}

/// Connect over `transport` and serve the session until it ends
async fn connect(
    shared: &Arc<Shared>,
    transport: agent_client_protocol::DynConnectTo<Client>,
    setup: &Setup,
    first: String,
    turns: &mut Turns,
    commands: &mut Inbox,
    id: &Mutex<Option<SessionId>>,
) -> Result<(), Error> {
    let updates = Arc::clone(shared);
    let permissions = Arc::clone(shared);
    let reads = Arc::clone(shared);
    let named = setup.extensions.clone();
    Client
        .v1()
        .name("kelpie")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                updates.update(notification.update);
                Ok(())
            },
            on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                let outcome = permissions.refuse(&request);
                responder.respond(RequestPermissionResponse::new(outcome))
            },
            on_receive_request!(),
        )
        // Last, so a typed handler claims its own message first and this one
        // takes what a harness sends beyond ACP
        .on_receive_notification(
            async move |message: UntypedMessage, _cx| {
                reads.read_extension(&named, message.method(), message.params());
                Ok(())
            },
            on_receive_notification!(),
        )
        .connect_with(transport, async |cx: ConnectionTo<Agent>| {
            let result = serve(&cx, shared, setup, first, turns, commands, id).await;
            if let Err(error) = &result
                && !(cx.is_incoming_closed() || is_gone(error))
            {
                shared.state().request_failed = true;
            }
            result
        })
        .await
}

/// Open the session, then run a turn per message
async fn serve(
    cx: &ConnectionTo<Agent>,
    shared: &Shared,
    setup: &Setup,
    first: String,
    turns: &mut Turns,
    commands: &mut Inbox,
    id: &Mutex<Option<SessionId>>,
) -> Result<(), Error> {
    let init = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let Opened {
        session,
        options,
        fresh,
        refused,
    } = open(cx, &init, setup, shared).await?;
    let session_id = SessionId::from(&*session.0);
    *lock(id) = Some(session_id.clone());
    shared.emit(SessionEvent::Started { session_id });
    apply_choice(cx, &session, &options, &setup.choice, shared).await;
    let mut applied = setup.choice.clone();

    // A new session after a refused resume, or one that replaces another
    // harness's, has the seed ahead of its first message
    let first = match &setup.seed {
        Some(wanted) if fresh && (refused || wanted.at_start) => seeded(wanted, setup, &first),
        _ => first,
    };
    // Instructions a harness cannot take as a system prompt lead the first
    // message of a session that has no history to hold them yet
    let first = match &setup.instructions {
        Some(instructions) if fresh && !offers_system_prompt(&init) => {
            format!("{instructions}\n\n{first}")
        }
        _ => first,
    };
    turns.push_back((first, None));
    loop {
        let (text, sink) = match turns.pop_front() {
            Some(turn) => turn,
            None => match idle(cx, commands).await? {
                Some(Command::Send { text, sink }) => (text, Some(sink)),
                Some(Command::Cancel) => continue,
                None => return Ok(()),
            },
        };
        if let Some(sink) = sink {
            shared.start_turn(sink);
        }
        // A choice made since the last turn applies from this message on
        let now = lock(&setup.wanted).clone();
        let changed = Choice {
            model: now
                .model
                .clone()
                .filter(|model| applied.model.as_ref() != Some(model)),
            effort: now
                .effort
                .clone()
                .filter(|effort| applied.effort.as_ref() != Some(effort)),
        };
        if changed != Choice::default() {
            apply_choice(cx, &session, &options, &changed, shared).await;
        }
        applied = now;
        let prompt = PromptRequest::new(
            session.clone(),
            vec![ContentBlock::Text(TextContent::new(text))],
        );
        let mut response = std::pin::pin!(cx.send_request(prompt).block_task());
        let result = loop {
            match select(response.as_mut(), commands.next()).await {
                Either::Left((result, _)) => {
                    // A cancel that lost the race with the turn's end is meant
                    // for this turn, not the next one
                    while let Ok(command) = commands.try_recv() {
                        if let Command::Send { text, sink } = command {
                            turns.push_back((text, Some(sink)));
                        }
                    }
                    break result;
                }
                Either::Right((Some(Command::Send { text, sink }), _)) => {
                    turns.push_back((text, Some(sink)));
                }
                Either::Right((Some(Command::Cancel), _)) => {
                    cx.send_notification(CancelNotification::new(session.clone()))?;
                }
                // The session was dropped: its turn is abandoned
                Either::Right((None, _)) => return Ok(()),
            }
        };
        match result {
            Ok(response) => shared.emit(shared.turn_ended(response)),
            Err(error) if cx.is_incoming_closed() || is_gone(&error) => return Err(error),
            Err(error) => shared.emit(failed(format!("the turn failed: {error}"))),
        }
        shared.set_sink(None);
    }
}

pub(super) fn is_gone(error: &Error) -> bool {
    is_connection_terminated(error) || is_incoming_transport_closed(error)
}

/// The next command, or an error when the harness ends the connection first
async fn idle(cx: &ConnectionTo<Agent>, commands: &mut Inbox) -> Result<Option<Command>, Error> {
    match select(std::pin::pin!(cx.incoming_closed()), commands.next()).await {
        Either::Left(_) => Err(Error::internal_error().data("the harness exited between turns")),
        Either::Right((command, _)) => Ok(command),
    }
}

/// `message` with the seed ahead of it: the decisions file, and the log up to the message
///
/// The log already holds `message` as its last user entry, since a send writes
/// it before the worker runs, so what the seed reads stops before it. A log
/// that cannot be read seeds with the decisions alone.
fn seeded(wanted: &super::Seed, setup: &Setup, message: &str) -> String {
    let entries = setup
        .log
        .as_ref()
        .and_then(|log| super::log::read(log.path()).ok())
        .unwrap_or_default();
    let before = entries
        .iter()
        .rposition(|entry| matches!(entry, kelpie_protocol::LogEntry::User { .. }))
        .map_or(&entries[..], |last| &entries[..last]);
    seed::ahead_of(&seed::seed(&wanted.decisions, before), message)
}

/// Whether the harness takes instructions as `systemPrompt`
fn offers_system_prompt(init: &InitializeResponse) -> bool {
    init.agent_capabilities
        .session_capabilities
        .system_prompt
        .is_some()
}

/// The instructions as a `systemPrompt` in `append` mode, for a harness that offers one
fn system_prompt(init: &InitializeResponse, setup: &Setup) -> Option<SystemPrompt> {
    let instructions = setup.instructions.as_ref()?;
    offers_system_prompt(init).then(|| {
        SystemPrompt::new(vec![ContentBlock::Text(TextContent::new(
            instructions.clone(),
        ))])
    })
}

/// The session a connection opened
struct Opened {
    session: agent_client_protocol::v1::schema::SessionId,
    options: Vec<SessionConfigOption>,
    /// Whether it is a new session, not a resumed one
    fresh: bool,
    /// Whether it is new because the harness refused to resume the saved id
    refused: bool,
}

/// Resume the session, or start one
///
/// A resume that fails starts a new session and says so first, with a
/// `Failed` for the turn and a divider in the log. A harness that replays a
/// loaded session has its history dropped: kelpie keeps its own.
async fn open(
    cx: &ConnectionTo<Agent>,
    init: &InitializeResponse,
    setup: &Setup,
    shared: &Shared,
) -> Result<Opened, Error> {
    let mut refused = false;
    if let Some(wanted) = &setup.resume {
        let capabilities = &init.agent_capabilities;
        let resumed = if capabilities.session_capabilities.resume.is_some() {
            let request = ResumeSessionRequest::new(wanted.to_string(), setup.project.clone())
                .system_prompt(system_prompt(init, setup));
            cx.send_request(request)
                .block_task()
                .await
                .map(|response| response.config_options)
        } else if capabilities.load_session {
            shared.set_replaying(true);
            let request = LoadSessionRequest::new(wanted.to_string(), setup.project.clone())
                .system_prompt(system_prompt(init, setup));
            let loaded = cx.send_request(request).block_task().await;
            shared.set_replaying(false);
            loaded.map(|response| response.config_options)
        } else {
            Err(Error::internal_error().data("the harness cannot resume a session"))
        };
        match resumed {
            Ok(options) => {
                return Ok(Opened {
                    session: wanted.to_string().into(),
                    options: options.unwrap_or_default(),
                    fresh: false,
                    refused: false,
                });
            }
            Err(error) if is_gone(&error) => return Err(error),
            Err(error) => {
                refused = true;
                shared.emit(failed(format!(
                    "could not resume session {wanted}, so this is a new one: {error}"
                )));
                // Only a session that seeds its successor says so in its log
                if setup.seed.is_some()
                    && let Some(log) = &setup.log
                    && let Err(error) = log.append(&kelpie_protocol::LogEntry::Divider {
                        text: COULDNT_RESUME.to_owned(),
                    })
                {
                    shared.emit(failed(format!("could not write the session log: {error}")));
                }
            }
        }
    }
    let request = new_session(setup.project.clone(), setup.meta.as_ref())?
        .system_prompt(system_prompt(init, setup));
    let response = cx.send_request(request).block_task().await?;
    Ok(Opened {
        session: response.session_id,
        options: response.config_options.unwrap_or_default(),
        fresh: true,
        refused,
    })
}

/// `session/new` for `project`, with the harness's `_meta` on it
pub(super) fn new_session(
    project: PathBuf,
    meta: Option<&serde_json::Value>,
) -> Result<NewSessionRequest, Error> {
    let mut request = NewSessionRequest::new(project);
    match meta {
        None => {}
        Some(serde_json::Value::Object(meta)) => request = request.meta(meta.clone()),
        Some(_) => {
            return Err(
                Error::invalid_params().data("the harness's new_session_meta is not a JSON object")
            );
        }
    }
    Ok(request)
}

/// Set the model and effort the person chose, where the harness lists them
///
/// Anything the harness will not take is a `Failed` event, and the session
/// keeps its default.
async fn apply_choice(
    cx: &ConnectionTo<Agent>,
    session: &agent_client_protocol::v1::schema::SessionId,
    options: &[SessionConfigOption],
    choice: &Choice,
    shared: &Shared,
) {
    let wanted = [
        ("model", SessionConfigOptionCategory::Model, &choice.model),
        (
            "effort",
            SessionConfigOptionCategory::ThoughtLevel,
            &choice.effort,
        ),
    ];
    for (what, category, value) in wanted {
        let Some(value) = value else { continue };
        let Some(option) = options
            .iter()
            .find(|option| option.category.as_ref() == Some(&category))
        else {
            shared.emit(failed(format!("the harness offers no {what} to choose")));
            continue;
        };
        let listed = match &option.kind {
            SessionConfigKind::Select(select) => match &select.options {
                SessionConfigSelectOptions::Ungrouped(list) => {
                    list.iter().any(|o| &*o.value.0 == value.as_str())
                }
                SessionConfigSelectOptions::Grouped(groups) => groups
                    .iter()
                    .flat_map(|group| &group.options)
                    .any(|o| &*o.value.0 == value.as_str()),
                _ => false,
            },
            _ => false,
        };
        if !listed {
            shared.emit(failed(format!(
                "the harness does not list {what} {value:?}, so it keeps its default"
            )));
            continue;
        }
        let request = SetSessionConfigOptionRequest::new(
            session.clone(),
            option.id.clone(),
            agent_client_protocol::v1::schema::SessionConfigOptionValue::value_id(value.clone()),
        );
        if let Err(error) = cx.send_request(request).block_task().await {
            shared.emit(failed(format!(
                "the harness did not take {what} {value:?}, so it keeps its default: {error}"
            )));
        }
    }
}
