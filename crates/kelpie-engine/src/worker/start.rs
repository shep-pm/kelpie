//! Starting a worker: the one function every caller of one goes through
//!
//! The Start button on a ticket, the PM's `start_worker` and Phase 3's board
//! dispatch all call [`start`] (`docs/design/workers.md`, Starting one), so
//! dispatch is a caller of it and never a second path. In order: a slot, or a
//! place in line for one; the ticket's worktree, idempotent, so starting a
//! ticket again finds the worktree its last turn left; the worker's sandbox; and
//! a session on the `kelpie:worker` binding whose every tool call is judged
//! against the worktree. The ticket's text is the turn's prompt.
//!
//! The turn's events go to the caller's sink, and `start` reads them for the
//! record on the way: the session id is kept when the harness gives it, and when
//! the turn ends its branch passes the fence, which settles the record as done
//! or as waiting on the maintainer, whatever the caller does with the events.
//!
//! The session is built by the caller's `open`, as the PM's is
//! ([`crate::session`]). Kelpie ships no harness (ADR 0006), and only the app
//! holds the logins and the installed bridges; `start` hands it the folders the
//! worker works in and a way to start its sandbox, so every harness on every
//! caller gets the same policy ([`crate::sandbox::worker`]).
//!
//! Each worker gets a lease socket of its own in its state folder, which its
//! policy names and its environment names, so its `kelpie lease run` waits its
//! turn behind the other workers' tests ([`crate::lease`]).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::agents::Binding;
use crate::lease::{Leases, SessionLease};
use crate::permission::Judge;
use crate::sandbox::{self, Reads, Sandbox};
use crate::session::log::Log;
use crate::session::{Choice, Harness, Session, SessionEvent, SessionId};
use crate::tools::{SessionTools, Tools};
use crate::tree::ProcessTree;
use crate::worktree::{self, Remote, clone};

use super::ceiling::{Ceiling, Verdict, Watch};
use super::{
    Record, Records, Seat, Status,
    turn_end::{self, Ending},
};

/// The folder under the data folder that holds each project's state
const PROJECTS_FOLDER: &str = "projects";

/// The folder inside a project's folder that holds each ticket's worktree
const WORKTREES_FOLDER: &str = "worktrees";

/// The folder inside a project's folder that holds each worker's state
const WORKERS_FOLDER: &str = "workers";

/// The file that holds a project's worker records, beside the state folders
const RECORDS_FILE: &str = "workers.json";

/// The folder inside a worker's state folder that it builds into
const BUILD_FOLDER: &str = "build";

/// A worker's log, by name, in its state folder
const LOG_FILE: &str = "log.jsonl";

/// The hand-over file a worker writes a pull request's title and body in, by
/// name, in its state folder (`docs/design/workers.md`, Pushing and the pull request)
const PULL_REQUEST_FILE: &str = "pull-request.md";

/// The prefix every ticket's branch carries, as SK's `home/paths.rs` has it
const BRANCH_PREFIX: &str = "kelpie/";

/// Where one project's workers live under kelpie's data folder
///
/// The worktree, its clone's git dir and the state folder are the only folders a
/// worker writes, and the record is kept beside them, out of reach of the
/// worker's sandbox ([`crate::sandbox::worker`]).
#[derive(Debug, Clone)]
pub struct Paths {
    /// kelpie's data folder
    pub data_dir: PathBuf,

    /// The project's id, which its state folder carries
    pub project_id: String,
}

impl Paths {
    /// The paths of the project `project_id` inside kelpie's data folder `data_dir`
    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>, project_id: impl Into<String>) -> Self {
        Self {
            data_dir: data_dir.into(),
            project_id: project_id.into(),
        }
    }

    /// `<data_dir>/projects`, which no worker's sandbox reads but its own state folder
    #[must_use]
    pub fn sessions_root(&self) -> PathBuf {
        self.data_dir.join(PROJECTS_FOLDER)
    }

    /// `<data_dir>/projects/<id>`, the project's own state folder
    #[must_use]
    pub fn project(&self) -> PathBuf {
        self.sessions_root().join(&self.project_id)
    }

    /// `<data_dir>/projects/<id>/workers.json`, the records of its workers
    #[must_use]
    pub fn records(&self) -> PathBuf {
        self.project().join(RECORDS_FILE)
    }

    /// `<data_dir>/projects/<id>/worktrees/<ticket>`, the ticket's worktree
    #[must_use]
    pub fn worktree(&self, ticket: &str) -> PathBuf {
        self.project().join(WORKTREES_FOLDER).join(ticket)
    }

    /// `<data_dir>/projects/<id>/worktrees/<ticket>.git`, the clone's own git dir
    ///
    /// [`clone::git_dir_of`] is the one spelling of it, so a ticket whose id
    /// holds a dot, `1.2`, gets `1.2.git` here and not `1.git`.
    #[must_use]
    pub fn git_dir(&self, ticket: &str) -> PathBuf {
        clone::git_dir_of(&self.worktree(ticket))
    }

    /// `<data_dir>/projects/<id>/workers/<ticket>`, which holds the worker's
    /// harness's config folder, its build folder and its log
    #[must_use]
    pub fn state_dir(&self, ticket: &str) -> PathBuf {
        self.project().join(WORKERS_FOLDER).join(ticket)
    }

    /// The folder in the worker's state folder that its build writes into
    #[must_use]
    pub fn build_dir(&self, ticket: &str) -> PathBuf {
        self.state_dir(ticket).join(BUILD_FOLDER)
    }

    /// The worker's log, in its state folder
    #[must_use]
    pub fn log_file(&self, ticket: &str) -> PathBuf {
        self.state_dir(ticket).join(LOG_FILE)
    }

    /// The hand-over file the worker writes a pull request's title and body in,
    /// in its state folder, which its sandbox writes and kelpie reads
    #[must_use]
    pub fn pull_request_file(&self, ticket: &str) -> PathBuf {
        self.state_dir(ticket).join(PULL_REQUEST_FILE)
    }
}

/// The branch a ticket's worker works on
#[must_use]
pub fn branch_of(ticket: &str) -> String {
    format!("{BRANCH_PREFIX}{ticket}")
}

/// The ticket a worker starts on
///
/// The ticket model lands later; until then a start takes the two facts a
/// worker's first turn reads: which ticket, and what it asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    /// The ticket's id, which names its branch, its worktree and its state folder
    pub id: String,

    /// What the ticket asks for, sent as the worker's first message
    pub text: String,
}

/// The project a worker starts in, with everything a start needs from it
pub struct Project<'a> {
    /// Where its workers live under kelpie's data folder
    pub paths: &'a Paths,

    /// The project's checkout, which each worktree is cut from
    pub folder: &'a Path,

    /// The branch each worktree is cut from, `main` for a GitHub project
    pub base: &'a str,

    /// The tree every worker's harness starts in (ADR 0001)
    pub tree: &'a Arc<ProcessTree>,

    /// How many of the project's workers run at once (`worker-slots`)
    pub slots: usize,

    /// How long a worker's turn may run, counted from its slot's grant
    /// (`worker-turn-timeout`)
    pub turn_timeout: Duration,

    /// The project's workers, oldest first, as their records keep them
    pub records: &'a Arc<Mutex<Records>>,

    /// What a worker runs on: `kelpie:worker`, then the global `worker.md`, then
    /// the project's own setting
    pub binding: &'a Binding,

    /// Domains the worker's sandbox lets through, besides its harness's model
    /// hosts and GitHub (`network-allowlist`)
    pub network_allowlist: &'a [String],

    /// How far the worker reads (`read-anywhere`)
    pub reads: Reads,

    /// kelpie's tools server, whose door the worker's sandbox names.
    /// `None` where the door could not open.
    pub tools: Option<&'a Tools>,

    /// kelpie's leases, whose socket each worker's session gets. `None` where the
    /// platform has no lease socket, which leaves every worker unleased.
    pub leases: Option<&'a Leases>,

    /// Build the worker's session on the harness the binding names, inside the
    /// sandbox [`Opening::sandbox`] starts. The text of an error is for the
    /// maintainer to read.
    pub open: &'a dyn Fn(&Opening<'_>) -> Result<Session, String>,
}

// The opener is a closure, which no derive can print
impl std::fmt::Debug for Project<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Project")
            .field("paths", &self.paths)
            .field("folder", &self.folder)
            .field("base", &self.base)
            .field("slots", &self.slots)
            .field("turn_timeout", &self.turn_timeout)
            .field("reads", &self.reads)
            .finish_non_exhaustive()
    }
}

/// What a worker's session is built from
pub struct Opening<'a> {
    /// The tree the session's harness starts in
    pub tree: &'a Arc<ProcessTree>,

    /// What the worker runs on, each value with the layer that set it
    pub binding: &'a Binding,

    /// The model and effort to ask the harness for, from the binding
    pub choice: Choice,

    /// The worktree, which is the harness's working folder
    pub worktree: &'a Path,

    /// The worker's state folder, which holds its harness's config folder
    pub state_dir: &'a Path,

    /// The session id to resume, `None` for a ticket's first turn
    pub resume: Option<SessionId>,

    /// kelpie's tools for this session, with a token of its own
    pub tools: Option<SessionTools>,

    /// This session's lease socket, which its sandbox policy names and its
    /// harness's environment names as [`SOCKET_VAR`](crate::lease::SOCKET_VAR).
    /// An opener puts [`SessionLease::env`] into the harness's environment, so a
    /// `kelpie lease run` it starts can reach kelpie. `None` where the platform
    /// has no lease socket, or the state folder was too deep for one.
    pub lease: Option<SessionLease>,

    /// Start the worker's sandbox for `harness`, as its policy has it
    ///
    /// A bridge starts one per version it runs a session on, as the PM's does.
    pub sandbox: &'a dyn Fn(&Harness) -> Result<Sandbox, String>,
}

// The sandbox is a closure, which no derive can print
impl std::fmt::Debug for Opening<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opening")
            .field("binding", &self.binding)
            .field("choice", &self.choice)
            .field("worktree", &self.worktree)
            .field("state_dir", &self.state_dir)
            .field("resume", &self.resume)
            .finish_non_exhaustive()
    }
}

/// What a start did with a ticket
#[derive(Debug)]
pub enum Started {
    /// It has a slot, and its turn is running on the ticket's text
    Running {
        /// Its record, as kelpie keeps it
        record: Record,

        /// Its session, which kelpie holds while its turn runs
        session: Arc<Session>,

        /// The worktree it works in
        worktree: PathBuf,

        /// Its state folder, which holds its harness's config folder and its log
        state_dir: PathBuf,
    },

    /// No slot was free: the ticket is in line for the next one, and nothing started
    Waiting {
        /// Its record, which says where it stands
        record: Record,
    },
}

/// Why a worker could not start
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum StartError {
    /// `id` cannot name a folder, a branch and a file of its own
    Name {
        /// The ticket id as it was given
        id: String,
    },

    /// The ticket already has a worker holding a slot, so a second session would
    /// write the same worktree on the same branch
    Running {
        /// The ticket as it was given
        ticket: String,
    },

    /// The ticket's worktree could not be prepared
    Worktree(worktree::WorktreeError),

    /// The worker's record could not be written
    Record(io::Error),

    /// The worker's log could not be opened, so its turn would be kept nowhere
    Log(io::Error),

    /// The worker's session could not be built, with the reason for the maintainer
    Session(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Name { id } => write!(
                f,
                "{id} is not a ticket id kelpie can name a branch and a folder with"
            ),
            Self::Running { ticket } => {
                write!(f, "ticket {ticket} already has a worker on its worktree")
            }
            Self::Worktree(error) => {
                write!(f, "the ticket's worktree could not be prepared: {error}")
            }
            Self::Record(error) => write!(f, "the worker's record could not be kept: {error}"),
            Self::Log(error) => write!(f, "the worker's log could not be opened: {error}"),
            Self::Session(message) => {
                write!(f, "the worker's session could not be built: {message}")
            }
        }
    }
}

impl core::error::Error for StartError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Name { .. } | Self::Running { .. } | Self::Session(_) => None,
            Self::Worktree(error) => Some(error),
            Self::Record(error) | Self::Log(error) => Some(error),
        }
    }
}

/// Start a worker on `ticket`, and stream its turn into `events`
///
/// A ticket whose worker has ended, is Paused, or is still in line for a slot is
/// started again: its worktree keeps what its last turn left, and its session
/// resumes under the id its harness gave it. A ticket with no free slot is kept
/// in line for one and nothing starts for it, which is what a caller that wants
/// it running tries again for. A ticket whose worker already holds a slot is
/// refused, so no two sessions write one worktree.
///
/// `events` hears every [`SessionEvent`] of the turn, after `start` has read it
/// for the record: the session id is kept as soon as the harness gives it, and
/// once the turn ends its branch passes the fence ([`turn_end`]), which settles
/// the record and holds no slot.
///
/// A turn that runs past `project.turn_timeout` is cancelled and its worker
/// Paused ([`super::ceiling`]). The time counts from the grant of the slot, which
/// is before the worktree is prepared and the harness started.
///
/// A slot held here is given back when the worker never began. The design's
/// order puts the slot first (`docs/design/workers.md`, Starting one), and the
/// worktree, the log and the opener can each still fail after it.
///
/// # Errors
///
/// [`StartError`]: the ticket id names nothing, the ticket already has a worker
/// running, the worktree could not be prepared, the record or the log could not
/// be written, or the session could not be built.
pub fn start(
    project: &Project<'_>,
    ticket: &Ticket,
    events: impl FnMut(SessionEvent) + Send + 'static,
) -> Result<Started, StartError> {
    if !is_name(&ticket.id) {
        return Err(StartError::Name {
            id: ticket.id.clone(),
        });
    }
    let held = hold_slot(project, &ticket.id, &branch_of(&ticket.id))?;
    let ceiling = Ceiling::from_grant(Instant::now(), project.turn_timeout);
    if held.record.seat == Seat::Waiting {
        return Ok(Started::Waiting {
            record: held.record,
        });
    }
    match begin(project, ticket, held.record, ceiling, events) {
        Ok(started) => Ok(started),
        Err(error) => {
            // The slot was held for a worker that never began
            if let Err(lost) = give_slot_back(project, &ticket.id, held.before.as_ref()) {
                eprintln!(
                    "kelpie: the slot of ticket {} was not given back: {lost}",
                    ticket.id
                );
            }
            Err(error)
        }
    }
}

/// The worker a held slot begins: its worktree, its sandbox and its session
///
/// # Errors
///
/// [`StartError::Worktree`], [`StartError::Log`] or [`StartError::Session`]: the
/// worktree could not be prepared, the log could not be opened, or the session
/// could not be built.
fn begin(
    project: &Project<'_>,
    ticket: &Ticket,
    record: Record,
    ceiling: Ceiling,
    events: impl FnMut(SessionEvent) + Send + 'static,
) -> Result<Started, StartError> {
    let branch = branch_of(&ticket.id);
    let worktree = project.paths.worktree(&ticket.id);
    let state_dir = project.paths.state_dir(&ticket.id);
    let prepared = worktree::prepare(
        project.tree,
        project.folder,
        &worktree,
        &branch,
        worktree::Start::Main,
        project.base,
        &Remote::Origin,
        &project.paths.build_dir(&ticket.id),
    )
    .map_err(StartError::Worktree)?;

    let log = Log::open(project.paths.log_file(&ticket.id)).map_err(StartError::Log)?;
    let tools = project.tools.map(Tools::session);
    // One lease socket per session, in its state folder, which its policy names.
    // A worker whose socket could not open runs unleased: its `kelpie lease run`
    // exits [`NO_LEASE`](crate::lease::run::NO_LEASE) and says so.
    let lease = project.leases.and_then(|leases| {
        leases
            .session(&state_dir)
            .map(Some)
            .unwrap_or_else(|error| {
                eprintln!("kelpie: ticket {} starts unleased: {error}", ticket.id);
                None
            })
    });
    let sessions_root = project.paths.sessions_root();
    let folders = sandbox::Worker {
        worktree: &worktree,
        git_dir: &prepared.git_dir,
        git_common_dir: &prepared.git_common_dir,
        state_dir: &state_dir,
        kelpie_home: &project.paths.data_dir,
        door: tools.as_ref().map(SessionTools::door),
        lease: lease.as_ref().map(SessionLease::door),
    };
    let reach = sandbox::Project {
        folder: project.folder,
        network_allowlist: project.network_allowlist,
        reads: project.reads,
        sessions_root: &sessions_root,
    };
    let sandbox = |harness: &Harness| {
        Sandbox::start_worker(harness, &reach, &folders).map_err(|error| error.to_string())
    };
    let session = (project.open)(&Opening {
        tree: project.tree,
        binding: project.binding,
        choice: choice(project.binding),
        worktree: &worktree,
        state_dir: &state_dir,
        resume: record.session.clone(),
        tools: tools.clone(),
        lease: lease.clone(),
        sandbox: &sandbox,
    })
    .map_err(StartError::Session)?;

    let session = match tools {
        Some(tools) => session.with_tools(tools),
        None => session,
    };
    // The socket stays open while the session runs, and closes with it
    let session = match lease {
        Some(lease) => session.with_lease(lease),
        None => session,
    };
    let session = Arc::new(
        session
            .with_judge(Judge::new(
                &worktree,
                vec![worktree.clone(), state_dir.clone()],
            ))
            .with_instructions(instructions(project, &ticket.id))
            .with_log(log),
    );
    let watch = Watch::new();
    lock(project.records).keep_watch(&ticket.id, watch.clone());
    session.send(
        ticket.text.clone(),
        kept(project, &ticket.id, watch.clone(), events),
    );
    watch.hold_to(
        ceiling,
        Arc::clone(&session),
        Arc::clone(project.records),
        ticket.id.clone(),
        project.slots,
    );
    Ok(Started::Running {
        record,
        session,
        worktree,
        state_dir,
    })
}

/// Put `ticket`'s worker in the project's slots, and keep its record
///
/// A ticket already started keeps its place in the list, which is the order
/// freed slots go out in. A ticket whose worker holds a slot is refused, since a
/// second session would write the same worktree on the same branch.
///
/// # Errors
///
/// [`StartError::Running`]: the ticket's worker is already running.
/// [`StartError::Record`]: the records file could not be written.
fn hold_slot(project: &Project<'_>, ticket: &str, branch: &str) -> Result<Held, StartError> {
    let mut records = lock(project.records);
    let before = records.get(ticket).cloned();
    if before
        .as_ref()
        .is_some_and(|held| held.status == Status::Running && held.seat == Seat::Held)
    {
        return Err(StartError::Running {
            ticket: ticket.to_owned(),
        });
    }
    // A turn that ended with its harness never reported an end, and its clock
    // must not reach the worker this start begins
    records.retire_watch(ticket);
    let mut record = before
        .clone()
        .unwrap_or_else(|| Record::new(ticket, branch));
    record.status = Status::Running;
    records.keep(record.clone()).map_err(StartError::Record)?;
    records.seat(project.slots).map_err(StartError::Record)?;
    if let Some(seated) = records.get(ticket) {
        record.seat = seated.seat;
    }
    Ok(Held { record, before })
}

/// The record a start holds, and what the ticket's record was before it
struct Held {
    /// The record as the slots now see it: [`Seat::Held`], or [`Seat::Waiting`]
    /// when the project has no free slot
    record: Record,

    /// The record as it was before this start, `None` when the ticket had none.
    /// A start that never began puts this back.
    before: Option<Record>,
}

/// Give back the slot of a worker that never began
///
/// A ticket that had no record before is dropped, since nothing runs for it and
/// nothing is queued for it. One that had a record is put back as it was. Either
/// way the slot count runs again, so the next ticket in line takes the freed slot.
///
/// # Errors
///
/// The records file could not be written.
fn give_slot_back(project: &Project<'_>, ticket: &str, before: Option<&Record>) -> io::Result<()> {
    let mut records = lock(project.records);
    match before {
        Some(record) => records.keep(record.clone())?,
        None => {
            records.remove(ticket)?;
        }
    }
    records.seat(project.slots)?;
    Ok(())
}

/// The caller's sink, with the record read from every event that changes it
///
/// Each change is kept before the event goes on, so a caller that hears a turn
/// end finds the worker settled and holding no slot. A `Failed` that the session
/// goes on after is followed by that turn's `TurnEnded`, which settles the record,
/// unless the turn was cancelled at its ceiling: the worker is Paused then, and
/// stays so.
///
/// A turn's end goes through [`turn_end`] first: the branch is checked with the
/// fence, and what it found settles the record, done for a clean branch and
/// waiting on the maintainer for a held one.
fn kept(
    project: &Project<'_>,
    ticket: &str,
    watch: Watch,
    mut events: impl FnMut(SessionEvent) + Send + 'static,
) -> impl FnMut(SessionEvent) + Send + 'static {
    let kept = Arc::clone(project.records);
    let ticket = ticket.to_owned();
    let tree = Arc::clone(project.tree);
    let repo = project.folder.to_path_buf();
    let worktree = project.paths.worktree(&ticket);
    let base = project.base.to_owned();
    move |event: SessionEvent| {
        // The fence runs git, so it answers before the records lock is held
        let ending = match &event {
            SessionEvent::TurnEnded { .. } => Some(turn_end::check(&tree, &repo, &worktree, &base)),
            _ => None,
        };
        let mut records = lock(&kept);
        match &event {
            SessionEvent::Started { session_id } => {
                lost(
                    &ticket,
                    records.set_session(&ticket, session_id),
                    "its session id",
                );
            }
            SessionEvent::Failed { .. } => {
                lost(
                    &ticket,
                    records.set_status(&ticket, Status::Paused),
                    "its pause",
                );
            }
            _ => {}
        }
        // A turn the ceiling cancelled stays as the ceiling left it: Paused
        let settled = ending
            .as_ref()
            .filter(|_| watch.turn_ended() == Verdict::Ended);
        if let Some(ending) = settled {
            lost(
                &ticket,
                records.set_status(&ticket, ending.status()),
                "its end",
            );
        }
        drop(records);
        if let Some(ending) = settled {
            waits(&ticket, ending);
        }
        events(event);
    }
}

/// Name what a held branch holds its ticket for
///
/// The names reach kelpie's log until the app shows a held ticket on its card and
/// in the attention count (`docs/design/ux.md`, Attention).
fn waits(ticket: &str, ending: &Ending) {
    if let Ending::Held(held) = ending {
        eprintln!(
            "kelpie: ticket {ticket} waits on the maintainer: {}",
            held.line()
        );
    }
}

/// Say what a record that would not be written cost the ticket
///
/// The turn goes on either way, so the loss is named rather than swallowed: the
/// next start opens a new session instead of resuming, or the slot stays held
/// until the next launch pauses the worker.
fn lost(ticket: &str, kept: io::Result<()>, what: &str) {
    if let Err(error) = kept {
        eprintln!("kelpie: the record of ticket {ticket} did not keep {what}: {error}");
    }
}

/// The worker's instructions, with the built-in agent file's placeholders filled in
///
/// `{kelpie}` is kelpie's own binary, which the worker runs `lease run` through,
/// `{base}` the branch its worktree is cut from, and `{pull_request}` the
/// hand-over file it writes a pull request's title and body in
/// (`docs/design/workers.md`, Pushing and the pull request). A project's own
/// agent file that names none of them is sent as it is.
fn instructions(project: &Project<'_>, ticket: &str) -> String {
    let kelpie = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("kelpie"))
        .display()
        .to_string();
    let hand_over = project
        .paths
        .pull_request_file(ticket)
        .display()
        .to_string();
    project
        .binding
        .body
        .value
        .clone()
        .unwrap_or_default()
        .replace("{kelpie}", &kelpie)
        .replace("{base}", project.base)
        .replace("{pull_request}", &hand_over)
}

/// The model and effort the session asks the harness for
fn choice(binding: &Binding) -> Choice {
    Choice {
        model: binding.model.value.clone(),
        effort: binding.effort.value.clone(),
    }
}

/// Whether `ticket` can name a folder, a branch and a file of its own
///
/// The id names a worktree, its clone's git dir, a state folder and the branch
/// `kelpie/<id>`, so it is refused wherever those collide or where git refuses a
/// branch name: an id ending in `.git` is another ticket's clone git dir
/// ([`clone::git_dir_of`]), one ending in `.lock` is a ref git locks, `..` and a
/// trailing dot are refused in a ref name, and a leading dot would hide the
/// folder.
fn is_name(ticket: &str) -> bool {
    !ticket.is_empty()
        && !ticket.starts_with('.')
        && !ticket.ends_with('.')
        && !ticket.ends_with(".git")
        && !ticket.ends_with(".lock")
        && !ticket.contains("..")
        && ticket
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
