//! A session the engine holds, reached over ACP, whose events no harness owns
//!
//! [`Session::send`] streams one turn as [`SessionEvent`]s into a sink. The
//! first send starts the harness through the process tree (ADR 0001), speaks
//! ACP over its stdin and stdout, and opens a session, or resumes one. Nothing
//! runs before that. Nothing here names a harness: a [`Harness`] says what to
//! run, and each harness's own slice builds one (ADR 0007).
//!
//! - A send during a turn waits for the turn to end, and then runs.
//! - Until crabpot sandboxes sessions (ADR 0005), every permission request is
//!   refused, and the step shows as refused.
//! - When the harness exits, the turn in flight ends with a
//!   [`SessionEvent::Failed`]. The next send starts it again and resumes the
//!   session.
//! - Notes mid-turn and fork are not here yet.
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

pub use kelpie_protocol::{SessionEvent, SessionId};

use crate::tree::{Child, ProcessTree};

mod acp;
#[cfg(any(test, feature = "testing"))]
pub mod fake;
mod stderr;

/// How long a harness that closed its pipes is given to show its exit status
const EXIT_GRACE: Duration = Duration::from_millis(500);
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
    meta: Option<serde_json::Value>,
    instructions: Option<String>,
}

/// One conversation with a harness
///
/// Dropping it closes the connection, which a harness takes as its end. The
/// process tree stops it for certain when the run ends.
pub struct Session {
    launcher: Arc<Mutex<Box<dyn Launch>>>,
    project: PathBuf,
    choice: Choice,
    meta: Option<serde_json::Value>,
    instructions: Option<String>,
    id: Arc<Mutex<Option<SessionId>>>,
    /// The running worker's inbox, `None` before the first send
    worker: Mutex<Option<UnboundedSender<Command>>>,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("project", &self.project)
            .field("choice", &self.choice)
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
        let meta = harness.new_session_meta.clone();
        let launcher = ProcessLauncher {
            tree,
            harness,
            project: project.clone(),
        };
        Self::with_launcher(Box::new(launcher), project, resume, choice, meta)
    }

    pub(crate) fn with_launcher(
        launcher: Box<dyn Launch>,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
        meta: Option<serde_json::Value>,
    ) -> Self {
        Self {
            launcher: Arc::new(Mutex::new(launcher)),
            project,
            choice,
            meta,
            instructions: None,
            id: Arc::new(Mutex::new(resume)),
            worker: Mutex::new(None),
        }
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
        let mut command = Command::Send {
            text: text.into(),
            sink: Box::new(sink),
        };
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
            choice: self.choice.clone(),
            meta: self.meta.clone(),
            instructions: self.instructions.clone(),
        };
        let launcher = Arc::clone(&self.launcher);
        let id = Arc::clone(&self.id);
        let started = thread::Builder::new()
            .name("kelpie-session".to_owned())
            .spawn(move || futures::executor::block_on(acp::run(launcher, setup, commands, id)));
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
    project: PathBuf,
}

impl Launch for ProcessLauncher {
    fn launch(&mut self) -> io::Result<Link> {
        let mut command = Process::new(&self.harness.program);
        command
            .args(&self.harness.args)
            .envs(self.harness.env.iter().map(|(name, value)| (name, value)))
            .current_dir(&self.project)
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
        Ok(Link {
            transport: DynConnectTo::new(transport),
            explain: Box::new(move || explain_exit(&mut child, &tail)),
        })
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
    let lines = tail.lines();
    let mut text = match status {
        Some(status) => format!("the harness exited with {status}"),
        None => "the harness closed its output but is still running".to_owned(),
    };
    if !lines.is_empty() {
        text.push_str("; it last wrote: ");
        text.push_str(&lines);
    }
    Some(text)
}

#[cfg(test)]
mod tests;
