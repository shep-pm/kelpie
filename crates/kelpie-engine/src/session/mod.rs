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
//! - A session given [`Session::with_judge`] is the other way: its harness is
//!   set to ask, and each request is judged in process, so a refusal's reason
//!   stays on the refused step.
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
use semver::Version;

pub use kelpie_protocol::{LogEntry, SessionEvent, SessionId};

use crate::permission::{Judge, ReadCall};
use crate::sandbox::Sandbox;
use crate::tools::SessionTools;
use crate::tree::{Child, ProcessTree};

mod acp;
#[cfg(any(test, feature = "testing"))]
pub mod fake;
pub mod log;
pub mod options;
mod raw;
pub mod seed;
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
    /// The value of the harness's `mode` option that makes it ask before each
    /// tool call, which a judged session sets so every call reaches the judge
    ///
    /// A harness set to [`Harness::unprompted`] never asks, so the judge would
    /// see nothing.
    pub prompting: Option<String>,
    /// How the harness's adapter reads a tool call it asks about
    pub read_call: ReadCall,
    /// What its bridge says when the harness behind it is too old, and how the
    /// person updates that harness
    pub outdated: Option<Outdated>,
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
            prompting: None,
            read_call: ReadCall::standard(),
            outdated: None,
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
            .field("prompting", &self.prompting)
            .field("read_call", &self.read_call)
            .field("outdated", &self.outdated)
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
///
/// The engine makes every [`Link`] itself, so a launcher outside it cannot be
/// written.
pub struct Link {
    pub(crate) transport: DynConnectTo<Client>,
    /// What the harness left behind when it ended, if it can be known
    pub(crate) explain: Box<dyn FnMut() -> Option<String> + Send>,
    /// Stop the harness for certain, waiting `grace` for it to exit on its own,
    /// and say how it ended
    pub(crate) stop: Box<dyn FnOnce(Duration) -> Option<String> + Send>,
}

impl fmt::Debug for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}

/// How a session gets an agent to talk to
///
/// Kelpie writes every launcher itself: [`launcher`] for a harness's program,
/// and whatever a [`Bridge`] hands a session for one of its versions.
pub trait Launch: Send + 'static {
    /// Start the agent
    ///
    /// # Errors
    ///
    /// The agent cannot be started.
    fn launch(&mut self) -> io::Result<Link>;

    /// Whether the agent it starts runs inside a sandbox
    ///
    /// A session whose launcher says so sets the harness's
    /// [`Harness::unprompted`] mode on the version it runs, so a bridge's every
    /// version is covered by what that version's launcher says, not by what the
    /// first one did (ADR 0005).
    fn sandboxed(&self) -> bool {
        false
    }
}

/// The launcher that starts `harness` through `tree`, in `project`
#[must_use]
pub fn launcher(tree: Arc<ProcessTree>, harness: Harness, project: PathBuf) -> Box<dyn Launch> {
    Box::new(ProcessLauncher {
        tree,
        harness,
        sandbox: None,
        project,
    })
}

/// The launcher that starts `harness` through `tree` inside `sandbox`, in `project`
///
/// The sandbox wraps the command before the tree spawns it, and lives as long
/// as the launcher. A [`Bridge`] builds one per version it starts.
#[must_use]
pub fn sandboxed_launcher(
    tree: Arc<ProcessTree>,
    harness: Harness,
    sandbox: Sandbox,
    project: PathBuf,
) -> Box<dyn Launch> {
    Box::new(ProcessLauncher {
        tree,
        harness,
        sandbox: Some(sandbox),
        project,
    })
}

/// What a harness's bridge says when the harness behind it is too old, and how
/// the person updates it
///
/// A bridge needs a harness new enough for it. Kelpie never updates a harness
/// itself (`docs/design/bridge-updates.md`, Scope), so a bridge saying so ends
/// the turn with [`crate::harness::HarnessError::TooOld`], which names the
/// harness's own update command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outdated {
    /// The harness, as kelpie names it to the person
    pub harness: String,

    /// Phrases in the bridge's error that say so, matched without regard to case
    pub says: Vec<String>,

    /// The command the person runs to update the harness, such as `claude update`
    pub update: String,
}

/// One bridge version's agent: the harness it runs, and what starts it
pub struct Started {
    /// What to run, with the `_meta`, extension notifications and [`Outdated`]
    /// its harness offers
    pub harness: Harness,

    /// The launcher that starts it
    pub launch: Box<dyn Launch>,
}

impl fmt::Debug for Started {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Started")
            .field("harness", &self.harness)
            .finish_non_exhaustive()
    }
}

/// The swap [`Bridge::current`] offers: the version to start on, and the one it
/// takes over from
///
/// `from` is `Some` only when this look actually took a newer version into use,
/// which is what makes a later failure *that version's* failure rather than the
/// harness's (`docs/design/bridge-updates.md`, Flow).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swap {
    /// The version to start on
    pub version: Version,

    /// The version this swap leaves behind, `None` when nothing swapped
    pub from: Option<Version>,
}

/// The swap the next agent runs on
type Current = Box<dyn Fn() -> Swap + Send + Sync>;

/// The agent to start on one bridge version, and what its harness offers
type Start = Box<dyn Fn(&Version) -> Result<Started, String> + Send + Sync>;

/// What a version that failed a session gives back: the version to go to, `None` when there is none
type Fallback = Box<dyn Fn(&Version, &str) -> Option<Version> + Send + Sync>;

/// A bridge kelpie installs and can update, which a session moves to a newer
/// version of between turns
///
/// The app builds one from the bridge's [`crate::bridge::Versions`] and its own
/// way of building a harness for a version; a test builds one over scripted
/// agents (`fake::FakeBridge`, under the `testing` feature).
/// (`docs/design/bridge-updates.md`, Swapping)
pub struct Bridge {
    /// The bridge, as kelpie names it in a divider, such as `claude-agent-acp`
    pub id: String,

    /// The version to run on, and the one it swaps from
    ///
    /// Answers with the version in use when nothing newer is to be used. Taking
    /// a ready version into use is what moves it into use, and is what answers
    /// with the version it left
    /// ([`crate::bridge::Versions::current`]).
    pub current: Current,

    /// The agent to start on `version`, and what its harness offers
    ///
    /// # Errors
    ///
    /// The harness for that version could not be built. The text is for the maintainer.
    pub start: Start,

    /// Say `version` failed a session with `reason`, and get the version to go
    /// back to, `None` when there is none
    pub failed: Fallback,
}

impl fmt::Debug for Bridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bridge").field("id", &self.id).finish()
    }
}

/// The bridge as a divider names it
impl fmt::Display for Bridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)
    }
}

/// Where a session's agent comes from
#[derive(Clone)]
enum Source {
    /// One harness, started the same way every turn
    Fixed {
        /// What to run, and what its harness offers
        ///
        /// Boxed so this variant is no larger than the bridge one
        harness: Box<Harness>,
        /// What starts it
        launcher: Arc<Mutex<Box<dyn Launch>>>,
    },

    /// A bridge kelpie installs, which a session moves to a newer version of at
    /// its next idle point
    Bridge {
        /// Which versions to run on, and what starts each one
        bridge: Arc<Bridge>,
    },
}

/// What a worker needs to open its session
struct Setup {
    source: Source,
    project: PathBuf,
    /// The id to resume, which the worker replaces with the one the harness gives
    id: Arc<Mutex<Option<SessionId>>>,
    choice: Choice,
    /// What [`Session::choose`] last set, read before each turn
    wanted: Arc<Mutex<Choice>>,
    instructions: Option<String>,
    tools: Option<SessionTools>,
    mode: Option<String>,
    /// What judges each tool call the harness asks about, `None` to refuse them all
    judge: Option<Judge>,
    log: Option<log::Log>,
    seed: Option<Seed>,
    /// The bridge version the session's process runs on, shared with [`Session::bridge_version`]
    running: Arc<Mutex<Option<Version>>>,
}

/// Where a session that starts over takes its seed from, and when
#[derive(Debug, Clone)]
struct Seed {
    /// The standing-decisions file
    decisions: PathBuf,
    /// Whether a new session is seeded though no resume was refused
    at_start: bool,
}

/// How long a stopped harness is given to exit before the ladder goes on
const STOP_GRACE: Duration = Duration::from_secs(2);

/// The running worker: its inbox, and the thread that holds the harness
struct Worker {
    inbox: UnboundedSender<Command>,
    thread: thread::JoinHandle<()>,
}

/// One conversation with a harness
///
/// Dropping it closes the connection, which a harness takes as its end. The
/// process tree stops it for certain when the run ends.
pub struct Session {
    source: Source,
    project: PathBuf,
    /// The choice the next message runs on, which [`Session::choose`] changes
    choice: Arc<Mutex<Choice>>,
    instructions: Option<String>,
    /// The kelpie tools the session's harness is handed as an MCP server
    tools: Option<SessionTools>,
    /// The value of the harness's `mode` option to set, `None` to leave it
    mode: Option<String>,
    /// What judges each tool call the harness asks about, `None` to refuse them all
    judge: Option<Judge>,
    log: Option<log::Log>,
    seed: Option<Seed>,
    id: Arc<Mutex<Option<SessionId>>>,
    /// The bridge version the session's process runs on, `None` before the
    /// first start, and for a harness kelpie does not move
    running: Arc<Mutex<Option<Version>>>,
    /// The running worker, `None` before the first send
    worker: Mutex<Option<Worker>>,
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
        let launcher = launcher(Arc::clone(&tree), harness.clone(), project.clone());
        Self::with_launcher(launcher, harness, project, resume, choice)
    }

    /// A session like [`Session::new`] that runs its harness inside `sandbox`
    ///
    /// Every start of the harness is wrapped by the sandbox before the tree
    /// spawns it (ADR 0001, ADR 0005), and the sandbox lasts as long as the
    /// session and its worker do. The sandbox is what confines the harness, so
    /// the session turns the harness's own permission prompts off by setting
    /// its [`Harness::unprompted`] mode. A request that still arrives is
    /// refused, and a command the sandbox refuses shows as a failed step.
    ///
    /// A session on a [`Bridge`] has no one sandbox: each version's
    /// [`Started::launch`] comes from [`sandboxed_launcher`] with its own.
    #[must_use]
    pub fn sandboxed(
        tree: Arc<ProcessTree>,
        harness: Harness,
        sandbox: Sandbox,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
    ) -> Self {
        let launcher = sandboxed_launcher(tree, harness.clone(), sandbox, project.clone());
        Self::with_launcher(launcher, harness, project, resume, choice)
    }

    /// A session on `bridge`, which kelpie can move to a newer version of
    ///
    /// The first send starts the version [`Bridge::current`] names. Each later
    /// send at an idle point moves the session to a newer one and resumes it,
    /// and a version that fails the session sends it back to the one before
    /// (`docs/design/bridge-updates.md`, Swapping).
    #[must_use]
    pub fn on_bridge(
        bridge: Bridge,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
    ) -> Self {
        Self::from_source(
            Source::Bridge {
                bridge: Arc::new(bridge),
            },
            project,
            resume,
            choice,
        )
    }

    /// A session whose agent `launcher` starts, on what `harness` offers
    pub(crate) fn with_launcher(
        launcher: Box<dyn Launch>,
        harness: Harness,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
    ) -> Self {
        Self::from_source(
            Source::Fixed {
                harness: Box::new(harness),
                launcher: Arc::new(Mutex::new(launcher)),
            },
            project,
            resume,
            choice,
        )
    }

    fn from_source(
        source: Source,
        project: PathBuf,
        resume: Option<SessionId>,
        choice: Choice,
    ) -> Self {
        Self {
            source,
            project,
            choice: Arc::new(Mutex::new(choice)),
            instructions: None,
            tools: None,
            mode: None,
            judge: None,
            log: None,
            seed: None,
            id: Arc::new(Mutex::new(resume)),
            running: Arc::new(Mutex::new(None)),
            worker: Mutex::new(None),
        }
    }

    /// Set the harness's `mode` option to `mode` once the session opens
    #[cfg(test)]
    fn with_mode(mut self, mode: &str) -> Self {
        self.mode = Some(mode.to_owned());
        self
    }

    /// Judge each tool call the harness asks about, and answer for the person
    ///
    /// The harness is set to its [`Harness::prompting`] mode, so every tool call
    /// reaches the handler, which reads the call with the harness's own
    /// [`Harness::read_call`] and asks `judge`. A call it allows is answered with
    /// the harness's allow option. One it refuses is answered with the reject
    /// option, and the refused step keeps the reason. A harness that names no
    /// prompting mode is left in the mode it starts in. Without a judge every
    /// request is refused. Call this before the first send.
    #[must_use]
    pub fn with_judge(mut self, judge: Judge) -> Self {
        self.judge = Some(judge);
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

    /// Hand the harness kelpie's tools as an MCP server
    ///
    /// The server is `kelpie tools`, given as a stdio server in `mcpServers` on
    /// `session/new`, `session/load` and `session/resume`, since stdio is the one
    /// transport every ACP agent takes. Its environment carries the door and
    /// the session's token. The tools end with the session: the token goes when
    /// the session and its worker drop it. Call this before the first send.
    #[must_use]
    pub fn with_tools(mut self, tools: SessionTools) -> Self {
        self.tools = Some(tools);
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

    /// Seed a session that starts over from the file at `decisions` and from this session's log
    ///
    /// When the harness refuses to resume the saved id, the new session it
    /// starts has [`seed::seed`] ahead of the message, and the log gets a
    /// divider saying so. Needs [`Session::with_log`]: without a log there is
    /// nothing to seed from, and no divider is written. Call this before the
    /// first send.
    #[must_use]
    pub fn with_seed(mut self, decisions: impl Into<PathBuf>) -> Self {
        self.seed = Some(Seed {
            decisions: decisions.into(),
            at_start: false,
        });
        self
    }

    /// Seed this session's first message too, for a session that replaces one on another harness
    ///
    /// As [`Session::with_seed`], and a session that starts with no id to
    /// resume gets the seed as well. The caller writes the divider, since it
    /// knows what was switched.
    #[must_use]
    pub fn with_seed_at_start(mut self, decisions: impl Into<PathBuf>) -> Self {
        self.seed = Some(Seed {
            decisions: decisions.into(),
            at_start: true,
        });
        self
    }

    /// Stop the harness and wait until it has stopped
    ///
    /// A turn in flight is abandoned. Later sends start the harness again and
    /// resume the session, so a session that is to be replaced is dropped after
    /// this. Does nothing before the first send.
    pub fn stop(&self) {
        let worker = lock(&self.worker).take();
        if let Some(Worker { inbox, thread }) = worker {
            // The worker ends when its inbox does, and stops the harness
            drop(inbox);
            let _ = thread.join();
        }
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

    /// The bridge version the session's process runs on
    ///
    /// `None` before the first start, and for a session on a harness kelpie does
    /// not update. A session that moved and fell back reports the version it is
    /// on, not the one that failed it.
    #[must_use]
    pub fn bridge_version(&self) -> Option<Version> {
        lock(&self.running).clone()
    }

    /// Send `text` as the next message, and stream the turn into `sink`
    ///
    /// Returns at once. Every send ends with one `TurnEnded`, or with a
    /// `Failed` when the harness could not carry on. A `Failed` can also come
    /// first and the send go on: the session started over because the id could
    /// not be resumed, moved to a newer bridge version, or kept its default
    /// because a model or effort was not taken. Those come before the turn's own
    /// `TurnEnded`. `sink` runs on the session's own thread and must not block
    /// for long.
    pub fn send(&self, text: impl Into<String>, sink: impl FnMut(SessionEvent) + Send + 'static) {
        let text = text.into();
        let sink: Sink = match &self.log {
            Some(log) => log::logged(log, &text, sink),
            None => Box::new(sink),
        };
        let mut command = Command::Send { text, sink };
        let mut worker = lock(&self.worker);
        if let Some(Worker { inbox, .. }) = worker.as_ref() {
            match inbox.unbounded_send(command) {
                Ok(()) => return,
                // The worker ended, so the next one starts the harness again
                Err(unsent) => command = unsent.into_inner(),
            }
        }
        let (inbox, commands) = unbounded();
        let setup = Setup {
            source: self.source.clone(),
            project: self.project.clone(),
            id: Arc::clone(&self.id),
            choice: lock(&self.choice).clone(),
            wanted: Arc::clone(&self.choice),
            instructions: self.instructions.clone(),
            tools: self.tools.clone(),
            mode: self.mode.clone(),
            judge: self.judge.clone(),
            log: self.log.clone(),
            seed: self.seed.clone(),
            running: Arc::clone(&self.running),
        };
        let started = thread::Builder::new()
            .name("kelpie-session".to_owned())
            .spawn(move || futures::executor::block_on(acp::run(setup, commands)));
        match started {
            Ok(thread) => {
                // The thread holds the receiver, so this cannot fail
                let _ = inbox.unbounded_send(command);
                *worker = Some(Worker { inbox, thread });
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
        if let Some(Worker { inbox, .. }) = lock(&self.worker).as_ref() {
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
    fn sandboxed(&self) -> bool {
        self.sandbox.is_some()
    }

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
