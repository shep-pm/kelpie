//! A session the engine holds, reached over ACP, whose events no harness owns
//!
//! [`Session::send`] streams one turn as [`SessionEvent`]s into a sink. The
//! first send starts the harness through the process tree (ADR 0001), speaks
//! ACP over its stdin and stdout, and opens a session, or resumes one. Nothing
//! runs before that. Nothing here names a harness: a [`Harness`] says what to
//! run, and each harness's own slice builds one (ADR 0007).
//!
//! - A send during a turn waits for the turn to end, and then runs.
//! - Every permission request is refused, and the step shows as refused. A
//!   session built with [`Session::sandboxed`] runs inside crabpot (ADR 0005),
//!   so its harness is set to stop asking, and a command the sandbox refuses
//!   shows as a failed step.
//! - When the harness exits, the turn in flight ends with a
//!   [`SessionEvent::Failed`]. The next send starts it again and resumes the
//!   session.
//! - With a [`log::Log`], every message and event is also kept in kelpie's own
//!   log, which [`log::read`] gives back with no harness running.
//! - Notes mid-turn and fork are not here yet.
//! - [`options`] asks a harness what it offers, on a start that sends no prompt
//!   and stops the harness again.
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::PathBuf;
//! use std::sync::Arc;
//!
//! use kelpie_engine::run::RunId;
//! use kelpie_engine::session::{Choice, Harness, Session};
//! use kelpie_engine::tree::ProcessTree;
//!
//! let tree = Arc::new(ProcessTree::new(RunId::new()?, PathBuf::from("run.json"))?);
//! let harness = Harness::new("my-harness-acp");
//! let session = Session::new(tree, harness, PathBuf::from("."), None, Choice::default());
//! session.send("Hello", |event| println!("{event:?}"));
//! # Ok::<(), std::io::Error>(())
//! ```

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use agent_client_protocol::{Client, DynConnectTo};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};

pub use kelpie_protocol::{LogEntry, SessionEvent, SessionId};

use crate::sandbox::Sandbox;
use crate::tree::{Child, ProcessTree};

mod acp;
#[cfg(any(test, feature = "testing"))]
pub mod fake;
pub mod log;
pub mod options;
mod raw;
mod stderr;

/// How long a harness that closed its pipes is given to show its exit status
const EXIT_GRACE: Duration = Duration::from_millis(500);

/// How long a stopped harness's stderr is given to be read to its end
const TAIL_SETTLE: Duration = Duration::from_millis(200);
const EXIT_POLL: Duration = Duration::from_millis(10);

/// What starts a harness's ACP agent
///
/// Programs that speak ACP on stdin and stdout: a bridge such as
/// `claude-agent-acp`, or an agent that speaks it natively.
#[derive(Clone, PartialEq, Eq)]
pub struct Harness {
    /// The program to start
    pub program: PathBuf,
    /// Its arguments
    pub args: Vec<OsString>,
    /// Added to the environment the program inherits
    pub env: Vec<(OsString, OsString)>,
    /// Sent as `_meta` on `session/new`, a JSON object
    pub new_session_meta: Option<serde_json::Value>,
    /// The extension notifications this harness sends that kelpie reads
    pub extensions: Vec<Extension>,
    /// The hosts its model answers on, which a sandbox lets through
    pub hosts: Vec<String>,
    /// Folders outside the project that the harness reads to run, such as the
    /// one its own install sits in
    pub installs: Vec<PathBuf>,
    /// The value of the harness's `mode` option that turns its own permission
    /// prompts off, which a sandboxed session sets
    pub unprompted: Option<String>,
}

impl Harness {
    /// A harness that is `program` with no arguments, environment or `_meta`
    #[must_use]
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            env: Vec::new(),
            new_session_meta: None,
            extensions: Vec::new(),
            hosts: Vec::new(),
            installs: Vec::new(),
            unprompted: None,
        }
    }
}

/// A notification a harness sends beyond the ACP standard ones, which kelpie reads
///
/// ACP's own messages do not carry everything a harness knows. Each variant is
/// one extension notification, named by its JSON-RPC method, and what kelpie
/// takes from it (ADR 0007). A harness names the ones it sends, so kelpie reads
/// no notification it was not told about.
// kelpie-engine is a library: each harness slice adds its own
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Extension {
    /// `_claude/sdkMessage`: Claude Code's raw SDK messages, which carry the
    /// split between the one-hour and the five-minute prompt cache
    ClaudeSdkMessage,
}

impl Extension {
    /// The JSON-RPC method it names
    #[must_use]
    pub fn method(&self) -> &'static str {
        match self {
            Self::ClaudeSdkMessage => "_claude/sdkMessage",
        }
    }
}

// The environment and `_meta` carry login tokens, so `Debug` leaves out their
// values
impl fmt::Debug for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Harness")
            .field("program", &self.program)
            .field("args", &self.args)
            .field(
                "env",
                &self.env.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            )
            .field("new_session_meta", &self.new_session_meta.is_some())
            .field("extensions", &self.extensions)
            .field("hosts", &self.hosts)
            .field("installs", &self.installs)
            .field("unprompted", &self.unprompted)
            .finish()
    }
}

/// The model and effort a person picked, each applied when the harness lists it
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    /// A value of the harness's `model` option
    pub model: Option<String>,
    /// A value of the harness's `thought_level` option
    pub effort: Option<String>,
}

/// Why kelpie could not learn what a harness offers
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum SessionError {
    /// The harness process could not be started
    Start(io::Error),
    /// The harness ended the connection before it listed its options, with what it left behind
    Ended {
        /// The harness's own end, as the tree and its stderr can say it
        message: String,
    },
    /// The harness did not answer within the time kelpie waits, and was stopped
    TimedOut {
        /// How long kelpie waited
        after: Duration,
        /// How the harness ended once it was stopped, with what it last wrote
        message: String,
    },
    /// The harness answered with an error instead of its options
    Refused {
        /// The error the harness answered with
        message: String,
    },
    /// The harness id is not one file name, so no answer can be kept under it
    Name {
        /// The harness id kelpie was asked about
        name: String,
    },
    /// The answer could not be written to the data folder
    Cache {
        /// The file kelpie could not write
        path: PathBuf,
        /// What the OS said
        source: io::Error,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(error) => write!(f, "kelpie could not start the harness: {error}"),
            Self::Ended { message } => {
                write!(
                    f,
                    "the harness ended the connection before it answered: {message}"
                )
            }
            Self::TimedOut { after, message } => {
                write!(
                    f,
                    "the harness did not answer within {} s and was stopped",
                    after.as_secs_f32().ceil()
                )?;
                if message.is_empty() {
                    Ok(())
                } else {
                    write!(f, ": {message}")
                }
            }
            Self::Refused { message } => {
                write!(f, "the harness would not list its options: {message}")
            }
            Self::Name { name } => {
                write!(
                    f,
                    "{name} is not a name kelpie can keep a harness's answers under"
                )
            }
            Self::Cache { path, source } => {
                write!(f, "kelpie could not read or write {path:?}: {source}")
            }
        }
    }
}

impl core::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Start(error) | Self::Cache { source: error, .. } => Some(error),
            Self::Ended { .. }
            | Self::TimedOut { .. }
            | Self::Refused { .. }
            | Self::Name { .. } => None,
        }
    }
}

/// Where a turn's events go
type Sink = Box<dyn FnMut(SessionEvent) + Send>;

/// The receiving half a worker holds
type Inbox = UnboundedReceiver<Command>;

enum Command {
    Send { text: String, sink: Sink },
    Cancel,
}

/// A started agent: the pipe to speak ACP over, and how to say why it ended
pub(crate) struct Link {
    pub(crate) transport: DynConnectTo<Client>,
    /// What the harness left behind when it ended, if it can be known
    pub(crate) explain: Box<dyn FnMut() -> Option<String> + Send>,
    /// Stop the harness for certain, waiting `grace` for it to exit on its own,
    /// and say how it ended
    pub(crate) stop: Box<dyn FnOnce(Duration) -> Option<String> + Send>,
}

/// How a session gets an agent to talk to
pub(crate) trait Launch: Send + 'static {
    /// Start the agent
    ///
    /// # Errors
    ///
    /// The agent cannot be started.
    fn launch(&mut self) -> io::Result<Link>;
}

/// What a worker needs to open its session
struct Setup {
    project: PathBuf,
    resume: Option<SessionId>,
    choice: Choice,
    /// What [`Session::choose`] last set, read before each turn
    wanted: Arc<Mutex<Choice>>,
    meta: Option<serde_json::Value>,
    extensions: Vec<Extension>,
    instructions: Option<String>,
    mode: Option<String>,
}

/// One conversation with a harness
///
/// Dropping it closes the connection, which a harness takes as its end. The
/// process tree stops it for certain when the run ends.
pub struct Session {
    launcher: Arc<Mutex<Box<dyn Launch>>>,
    project: PathBuf,
    /// The choice the next message runs on, which [`Session::choose`] changes
    choice: Arc<Mutex<Choice>>,
    meta: Option<serde_json::Value>,
    extensions: Vec<Extension>,
    instructions: Option<String>,
    /// The value of the harness's `mode` option to set, `None` to leave it
    mode: Option<String>,
    log: Option<log::Log>,
    id: Arc<Mutex<Option<SessionId>>>,
    /// The running worker's inbox, `None` before the first send
    worker: Mutex<Option<UnboundedSender<Command>>>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("project", &self.project)
            .field("choice", &*lock(&self.choice))
            .field("id", &*lock(&self.id))
            .finish_non_exhaustive()
    }
}

impl Session {
    /// A session in `project` on `harness`, started by the first send
    ///
    /// `resume` is a session id a previous run kept. Starts nothing.
    #[must_use]
    pub fn new(
        tree: Arc<ProcessTree>,
        harness: Harness,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
    ) -> Self {
        Self::on_tree(tree, harness, None, project, resume, choice)
    }

    /// A session like [`Session::new`] that runs its harness inside `sandbox`
    ///
    /// Every start of the harness is wrapped by the sandbox before the tree
    /// spawns it (ADR 0001, ADR 0005), and the sandbox lasts as long as the
    /// session and its worker do. The sandbox is what confines the harness, so
    /// the session turns the harness's own permission prompts off by setting
    /// its [`Harness::unprompted`] mode. A request that still arrives is
    /// refused, and a command the sandbox refuses shows as a failed step.
    #[must_use]
    pub fn sandboxed(
        tree: Arc<ProcessTree>,
        harness: Harness,
        sandbox: Sandbox,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
    ) -> Self {
        Self::on_tree(tree, harness, Some(sandbox), project, resume, choice)
    }

    fn on_tree(
        tree: Arc<ProcessTree>,
        harness: Harness,
        sandbox: Option<Sandbox>,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
    ) -> Self {
        let meta = harness.new_session_meta.clone();
        let extensions = harness.extensions.clone();
        let mode = sandbox.as_ref().and(harness.unprompted.clone());
        let launcher = ProcessLauncher {
            tree,
            harness,
            sandbox,
            project: project.clone(),
        };
        let mut session = Self::with_launcher(
            Box::new(launcher),
            project,
            resume,
            choice,
            meta,
            extensions,
        );
        session.mode = mode;
        session
    }

    pub(crate) fn with_launcher(
        launcher: Box<dyn Launch>,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
        meta: Option<serde_json::Value>,
        extensions: Vec<Extension>,
    ) -> Self {
        Self {
            launcher: Arc::new(Mutex::new(launcher)),
            project,
            choice: Arc::new(Mutex::new(choice)),
            meta,
            extensions,
            instructions: None,
            mode: None,
            log: None,
            id: Arc::new(Mutex::new(resume)),
            worker: Mutex::new(None),
        }
    }

    /// Set the harness's `mode` option to `mode` once the session opens
    #[cfg(test)]
    fn with_mode(mut self, mode: &str) -> Self {
        self.mode = Some(mode.to_owned());
        self
    }

    /// Give the session instructions, such as its agent's body
    ///
    /// A harness that advertises `sessionCapabilities.systemPrompt` gets them
    /// as its `systemPrompt` in `append` mode, on a new session and on a load or
    /// a resume alike. For any other harness they lead the first message of a
    /// new session, and a resumed one has them in its history already. Blank
    /// instructions are none. Call this before the first send.
    #[must_use]
    pub fn with_instructions(mut self, instructions: impl Into<String>) -> Self {
        let instructions = instructions.into();
        self.instructions = (!instructions.trim().is_empty()).then_some(instructions);
        self
    }

    /// Keep every message sent and every event streamed in `log`
    ///
    /// The log is what shows the history again on a later launch, on any
    /// harness and with no process started (`docs/design/pm-chat.md`, History).
    /// A write that fails does not stop a turn: the turn says so with a
    /// `Failed`. Call this before the first send.
    #[must_use]
    pub fn with_log(mut self, log: log::Log) -> Self {
        self.log = Some(log);
        self
    }

    /// Stop writing to the session's log, for good
    ///
    /// For a quit: stopping the harness ends the turn in flight with a
    /// `Failed`, which is not something the maintainer saw. Left out of the log,
    /// the turn has no end there and shows as interrupted on the next launch.
    /// Does nothing without a log.
    pub fn close_log(&self) {
        if let Some(log) = &self.log {
            log.close();
        }
    }

    /// Change the model and effort the next message runs on
    ///
    /// The same session carries on: the harness is asked to set each option
    /// that differs from what it last set, before the next prompt. A turn in
    /// flight keeps what it started with. A field left empty is not asked, so
    /// clearing one leaves the harness on the value it last took.
    pub fn choose(&self, choice: Choice) {
        *lock(&self.choice) = choice;
    }

    /// The session's id: the one to resume, then the one the harness gave
    #[must_use]
    pub fn session_id(&self) -> Option<SessionId> {
        lock(&self.id).clone()
    }

    /// Send `text` as the next message, and stream the turn into `sink`
    ///
    /// Returns at once. Every send ends with one `TurnEnded`, or with a
    /// `Failed` when the harness could not carry on. A `Failed` can also come
    /// first and the send go on: the session started over because the id could
    /// not be resumed, or kept its default because a model or effort was not
    /// taken. Those come before the turn's own `TurnEnded`. `sink` runs on the
    /// session's own thread and must not block for long.
    pub fn send(&self, text: impl Into<String>, sink: impl FnMut(SessionEvent) + Send + 'static) {
        let text = text.into();
        let sink: Sink = match &self.log {
            Some(log) => log::logged(log, &text, sink),
            None => Box::new(sink),
        };
        let mut command = Command::Send { text, sink };
        let mut worker = lock(&self.worker);
        if let Some(inbox) = worker.as_ref() {
            match inbox.unbounded_send(command) {
                Ok(()) => return,
                // The worker ended, so the next one starts the harness again
                Err(unsent) => command = unsent.into_inner(),
            }
        }
        let (inbox, commands) = unbounded();
        let setup = Setup {
            project: self.project.clone(),
            resume: lock(&self.id).clone(),
            choice: lock(&self.choice).clone(),
            wanted: Arc::clone(&self.choice),
            meta: self.meta.clone(),
            extensions: self.extensions.clone(),
            instructions: self.instructions.clone(),
            mode: self.mode.clone(),
        };
        let launcher = Arc::clone(&self.launcher);
        let id = Arc::clone(&self.id);
        let started = thread::Builder::new()
            .name("kelpie-session".to_owned())
            .spawn(move || {
                futures::executor::block_on(acp::run(launcher, setup, commands, id));
            });
        match started {
            Ok(_) => {
                // The thread holds the receiver, so this cannot fail
                let _ = inbox.unbounded_send(command);
                *worker = Some(inbox);
            }
            Err(error) => {
                if let Command::Send { mut sink, .. } = command {
                    sink(SessionEvent::Failed {
                        message: format!("could not start the session's thread: {error}"),
                    });
                }
            }
        }
    }

    /// Ask the harness to stop the turn in flight
    ///
    /// The turn then ends as the harness reports it, normally `Cancelled`.
    /// Does nothing between turns.
    pub fn cancel(&self) {
        if let Some(inbox) = lock(&self.worker).as_ref() {
            let _ = inbox.unbounded_send(Command::Cancel);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the harness's program through the process tree
struct ProcessLauncher {
    tree: Arc<ProcessTree>,
    harness: Harness,
    /// Wraps the command before the tree spawns it
    sandbox: Option<Sandbox>,
    project: PathBuf,
}

impl Launch for ProcessLauncher {
    fn launch(&mut self) -> io::Result<Link> {
        let mut command = Process::new(&self.harness.program);
        command
            .args(&self.harness.args)
            .envs(self.harness.env.iter().map(|(name, value)| (name, value)))
            .current_dir(&self.project);
        // The wrapped command keeps the program, arguments, directory and
        // environment, and none of the streams
        let mut command = match &self.sandbox {
            Some(sandbox) => sandbox.command(&command)?,
            None => command,
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = self.tree.spawn(&mut command)?;
        let (stdin, stdout, stderr) = (
            opened(child.stdin.take())?,
            opened(child.stdout.take())?,
            opened(child.stderr.take())?,
        );
        let tail = stderr::Tail::collect(stderr);
        let transport = agent_client_protocol::ByteStreams::new(
            blocking::Unblock::new(stdin),
            blocking::Unblock::new(stdout),
        );
        // Both closures reach the same child: a session only asks how it ended,
        // an ask only stops it
        let ended = Arc::new(Mutex::new(Ended { child, tail }));
        let shared = Arc::clone(&ended);
        Ok(Link {
            transport: DynConnectTo::new(transport),
            explain: Box::new(move || lock(&shared).explain()),
            stop: Box::new(move |grace| lock(&ended).stop(grace)),
        })
    }
}

/// The harness process kelpie started, and what it wrote to stderr
struct Ended {
    child: Child,
    tail: stderr::Tail,
}

impl Ended {
    /// How the harness ended, if it can be known
    fn explain(&mut self) -> Option<String> {
        explain_exit(&mut self.child, &self.tail)
    }

    /// Stop the harness's group, and say how it ended and what it last wrote
    fn stop(&mut self, grace: Duration) -> Option<String> {
        match self.child.stop(grace) {
            Ok(status) => {
                self.tail.settle(TAIL_SETTLE);
                Some(with_stderr(
                    format!("the harness exited with {status}"),
                    &self.tail,
                ))
            }
            Err(error) => Some(format!("kelpie could not stop the harness: {error}")),
        }
    }
}

fn opened<T>(pipe: Option<T>) -> io::Result<T> {
    pipe.ok_or_else(|| io::Error::other("a pipe was not opened"))
}

/// The exit status of `child` and the last of what it wrote to stderr
fn explain_exit(child: &mut Child, tail: &stderr::Tail) -> Option<String> {
    let deadline = Instant::now() + EXIT_GRACE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(EXIT_POLL),
            Ok(None) | Err(_) => break None,
        }
    };
    let text = match status {
        Some(status) => format!("the harness exited with {status}"),
        None => "the harness closed its output but is still running".to_owned(),
    };
    Some(with_stderr(text, tail))
}

/// `text`, followed by the last of what the harness wrote to stderr, if it wrote any
fn with_stderr(mut text: String, tail: &stderr::Tail) -> String {
    let lines = tail.lines();
    if !lines.is_empty() {
        text.push_str("; it last wrote: ");
        text.push_str(&lines);
    }
    text
}

#[cfg(test)]
mod tests;
