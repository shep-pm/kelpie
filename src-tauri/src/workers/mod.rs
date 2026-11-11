//! The worker commands: what Start, a drop on Building and the PM's `start_worker` call
//!
//! [`Workers`] is the app's host for [`kelpie_engine::worker::start()`]. It keeps
//! what outlives one start: each project's records, the leases, the GitHub App
//! tokens, the `gh` on the machine's `PATH`, the sessions of the workers that are
//! running, and one [`GithubSource`] per project, which the board's poller shares
//! so every GitHub call of a project goes through one [`Hold`].
//!
//! A ticket is read before a slot is held, so a closed issue, a number naming
//! nothing and a project whose origin is not on GitHub are each refused with a
//! line saying which. A start does not refuse a ticket that is blocked or has
//! sub-issues: dispatch does.
//!
//! A worker's events go to its log and to its [`Feed`], which the window watches
//! with [`worker_watch`] and stops watching with [`worker_unwatch`]. A Paused
//! worker resumes with [`worker_resume`], when the maintainer says so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use kelpie_engine::github::gh::{Gh, GithubSource, hold::Hold};
use kelpie_engine::github::{AppTokens, origin_of, system_tokens};
use kelpie_engine::lease::Leases;
use kelpie_engine::login::Store;
use kelpie_engine::session::{Session, log};
use kelpie_engine::settings::{ConfigDir, Settings};
use kelpie_engine::ticket::{ForgeError, TicketId, Tickets};
use kelpie_engine::tools::Tools;
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::worker::{self, Opening, Paths, Records, Seat, Started, Status, Ticket};
use kelpie_protocol::{LogEntry, SessionEvent};
use serde::Serialize;
use specta::Type;
use tauri::State;
use tauri::ipc::Channel;

mod feed;
mod open;
mod plan;

pub use feed::Feed;
use open::OpenOn;
pub use plan::{WorkerPlan, plan};

use crate::state::AppState;

#[cfg(test)]
mod tests;

/// The branch every GitHub project's worktrees are cut from
const BASE: &str = "main";

/// What Resume sends as the turn's message: a message the maintainer writes is a Note
const CARRY_ON: &str = "Carry on from where you stopped.";

/// What a start answers
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct WorkerStarted {
    /// The ticket, as it was given
    pub ticket: String,

    /// No slot was free: the ticket is in line for the next one, and nothing started
    pub waiting: bool,
}

/// Build the worker's session for an [`Opening`], from what the app adds to it
type Opener<'a> = dyn Fn(&OpenOn<'_>, &Opening<'_>) -> Result<Session, String> + 'a;

/// What the app keeps for its workers, across starts
pub struct Workers {
    data_dir: PathBuf,
    tree: Arc<ProcessTree>,
    config: ConfigDir,
    logins: Store,
    tokens: AppTokens,
    /// The `gh` on the machine's `PATH`, `None` when there is none
    gh: Option<PathBuf>,
    /// Built on the first start, from the settings of the project it is for
    leases: OnceLock<Leases>,
    records: Mutex<BTreeMap<String, Arc<Mutex<Records>>>>,
    /// The sessions of the workers that hold a slot, by project and ticket
    ///
    /// The engine holds a worker's session only weakly, so this map is what keeps
    /// one alive. [`Workers::session`] and every start give up the sessions of
    /// workers that no longer hold a slot.
    sessions: Mutex<BTreeMap<(String, String), Arc<Session>>>,
    /// The channels watching each worker, by project and ticket
    ///
    /// A feed outlives its worker's turns, so a tab stays attached across a Resume.
    feeds: Mutex<BTreeMap<(String, String), Arc<Feed>>>,
    sources: Mutex<BTreeMap<String, Arc<GithubSource>>>,
}

impl std::fmt::Debug for Workers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workers")
            .field("data_dir", &self.data_dir)
            .field("gh", &self.gh)
            .finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Workers {
    /// The host for workers started in `tree`, with kelpie's files in `data_dir`
    #[must_use]
    pub fn new(
        data_dir: &Path,
        tree: &Arc<ProcessTree>,
        config: &ConfigDir,
        logins: &Store,
    ) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            tree: Arc::clone(tree),
            config: config.clone(),
            logins: logins.clone(),
            tokens: system_tokens(tree, data_dir),
            gh: worker::login::helper_on_path(),
            leases: OnceLock::new(),
            records: Mutex::default(),
            sessions: Mutex::default(),
            feeds: Mutex::default(),
            sources: Mutex::default(),
        }
    }

    /// The records of the project `project_id`'s workers, read from disk on the first ask
    ///
    /// # Errors
    ///
    /// The records file cannot be read.
    pub fn records(&self, project_id: &str) -> Result<Arc<Mutex<Records>>, String> {
        let mut kept = lock(&self.records);
        if let Some(records) = kept.get(project_id) {
            return Ok(Arc::clone(records));
        }
        let file = Paths::new(&self.data_dir, project_id).records();
        let records = Records::load(file).map_err(|error| error.to_string())?;
        let records = Arc::new(Mutex::new(records));
        kept.insert(project_id.to_owned(), Arc::clone(&records));
        Ok(records)
    }

    /// The tickets of the project `project_id`, whose checkout is `folder`
    ///
    /// One source per project, so every call of its goes through one [`Hold`].
    ///
    /// # Errors
    ///
    /// The checkout's origin is not a GitHub repo, or there is no `gh` to ask.
    pub fn source(&self, project_id: &str, folder: &Path) -> Result<Arc<GithubSource>, String> {
        let mut kept = lock(&self.sources);
        if let Some(source) = kept.get(project_id) {
            return Ok(Arc::clone(source));
        }
        let repo = origin_of(&self.tree, folder).ok_or_else(|| {
            format!(
                "{} has no GitHub origin, and GitHub is the only place kelpie reads tickets from yet",
                folder.display()
            )
        })?;
        let gh = self
            .gh
            .clone()
            .ok_or("the gh command is not on this machine's PATH, so kelpie cannot read GitHub")?;
        let hold = Hold::new(Gh::new(gh, &self.tree));
        let source = Arc::new(GithubSource::new(hold, repo));
        kept.insert(project_id.to_owned(), Arc::clone(&source));
        Ok(source)
    }

    /// The session of the worker on `ticket` while it holds a slot
    ///
    /// `None` once its turn ended or it was Paused, and then nothing of the
    /// worker's is kept alive.
    #[must_use]
    pub fn session(&self, project_id: &str, ticket: &str) -> Option<Arc<Session>> {
        self.give_up_idle();
        lock(&self.sessions)
            .get(&(project_id.to_owned(), ticket.to_owned()))
            .cloned()
    }

    /// Keep `session` as the worker on `ticket`'s, in place of one it had
    fn keep(&self, project_id: &str, ticket: &str, session: Arc<Session>) {
        let before =
            lock(&self.sessions).insert((project_id.to_owned(), ticket.to_owned()), session);
        // Dropped here, outside the lock: the last of a session stops its harness
        drop(before);
    }

    /// Drop the session of each worker whose record says it holds no slot
    ///
    /// Run from the caller's thread and never from a session's own event sink:
    /// the last reference to a session stops its harness and joins its thread.
    fn give_up_idle(&self) {
        let keys: Vec<(String, String)> = lock(&self.sessions).keys().cloned().collect();
        let idle: Vec<_> = keys
            .into_iter()
            .filter(|(project, ticket)| !self.holds_slot(project, ticket))
            .collect();
        let given: Vec<_> = {
            let mut sessions = lock(&self.sessions);
            idle.iter().filter_map(|key| sessions.remove(key)).collect()
        };
        drop(given);
    }

    fn holds_slot(&self, project_id: &str, ticket: &str) -> bool {
        let records = lock(&self.records).get(project_id).cloned();
        records.is_some_and(|records| {
            lock(&records)
                .get(ticket)
                .is_some_and(|r| r.status.needs_slot() && r.seat == Seat::Held)
        })
    }

    /// Stop logging every worker, so a quit does not log the `Failed` of its turn
    pub fn close_logs(&self) {
        for session in lock(&self.sessions).values() {
            session.close_log();
        }
    }

    /// Start a worker on `ticket` in the project `project_id`, whose checkout is `folder`
    ///
    /// Keeps the session of a worker that began, by ticket.
    ///
    /// # Errors
    ///
    /// The settings, the records or the leases cannot be used, or the engine's
    /// start refuses: its own text.
    pub fn start(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &Ticket,
        open: &Opener<'_>,
    ) -> Result<WorkerStarted, String> {
        self.give_up_idle();
        let plan = plan(&self.config, project_id)?;
        let records = self.records(project_id)?;
        let paths = Paths::new(&self.data_dir, project_id);
        let leases = self.leases(project_id)?;
        let on = OpenOn {
            data_dir: &self.data_dir,
            logins: &self.logins,
            config: &self.config,
            project_id,
            folder,
            plan: &plan,
        };
        let opener = |opening: &Opening<'_>| open(&on, opening);
        let project = worker::Project {
            paths: &paths,
            folder,
            base: BASE,
            tree: &self.tree,
            slots: plan.slots,
            turn_timeout: plan.turn_timeout,
            records: &records,
            binding: &plan.binding,
            network_allowlist: &plan.network_allowlist,
            reads: plan.reads,
            tools,
            tokens: Some(&self.tokens),
            gh: self.gh.as_deref(),
            leases: Some(leases),
            open: &opener,
        };
        // The session logs its own turn, and the feed follows it live
        let feed = self.feed(project_id, &ticket.id);
        let started = worker::start(&project, ticket, move |event| feed.forward(event))
            .map_err(|error| error.to_string())?;
        let waiting = matches!(started, Started::Waiting { .. });
        if let Started::Running { session, .. } = started {
            self.keep(project_id, &ticket.id, session);
        }
        Ok(WorkerStarted {
            ticket: ticket.id.clone(),
            waiting,
        })
    }

    /// The feed of the worker on `ticket`, made on the first ask
    #[must_use]
    pub fn feed(&self, project_id: &str, ticket: &str) -> Arc<Feed> {
        let mut feeds = lock(&self.feeds);
        let feed = feeds
            .entry((project_id.to_owned(), ticket.to_owned()))
            .or_insert_with(|| {
                // Made before the worker's session, so no event of the log is in flight
                let file = Paths::new(&self.data_dir, project_id).log_file(ticket);
                let past = log::read(file).unwrap_or_default();
                Arc::new(Feed::new(Feed::logged(&past)))
            });
        Arc::clone(feed)
    }

    /// The past of the worker on `ticket`, and `send` attached to follow it live
    ///
    /// The past is the worker's log, read under the feed's lock (see [`Feed::watch`]).
    /// A worker with no log yet has no entries. `send` answers whether the event
    /// was sent, and a `false` drops it from the feed. `id` names the channel:
    /// watching again under it replaces the first watch, and [`Workers::unwatch`] ends it.
    ///
    /// # Errors
    ///
    /// The project has no worker on `ticket`, or its log cannot be read.
    pub fn watch(
        &self,
        project_id: &str,
        ticket: &str,
        id: u32,
        send: impl Fn(SessionEvent) -> bool + Send + 'static,
    ) -> Result<Vec<LogEntry>, String> {
        let ticket = ticket_id(ticket)?;
        self.status_of(project_id, &ticket)?;
        let file = Paths::new(&self.data_dir, project_id).log_file(&ticket);
        self.feed(project_id, &ticket)
            .watch(id, || log::read(&file), send)
            .map_err(|error| format!("the log of ticket {ticket} cannot be read: {error}"))
    }

    /// Stop sending the worker on `ticket`'s events to the channel `id`
    ///
    /// A ticket that is not a number, or has no feed, has nothing to stop.
    pub fn unwatch(&self, project_id: &str, ticket: &str, id: u32) {
        let Ok(ticket) = ticket_id(ticket) else {
            return;
        };
        let feed = lock(&self.feeds)
            .get(&(project_id.to_owned(), ticket))
            .cloned();
        if let Some(feed) = feed {
            feed.unwatch(id);
        }
    }

    /// What the record of the worker on `ticket` says it is doing
    fn status_of(&self, project_id: &str, ticket: &str) -> Result<Status, String> {
        let records = self.records(project_id)?;
        let records = lock(&records);
        records
            .get(ticket)
            .map(|record| record.status)
            .ok_or_else(|| format!("there is no worker on ticket {ticket}"))
    }

    /// Start the next turn of the Paused worker on `ticket`, with "Carry on from where you stopped." as its message
    ///
    /// # Errors
    ///
    /// The project has no worker on `ticket`, or it is not Paused: the line says what
    /// it is doing. Or the start fails (see [`Workers::start`]).
    pub fn resume(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
        open: &Opener<'_>,
    ) -> Result<WorkerStarted, String> {
        let ticket = ticket_id(ticket)?;
        match self.status_of(project_id, &ticket)? {
            Status::Paused => {}
            other => {
                return Err(format!(
                    "ticket {ticket} is {}, so it does not resume",
                    doing(other)
                ));
            }
        }
        let ticket = Ticket {
            id: ticket,
            text: CARRY_ON.to_owned(),
        };
        self.start(project_id, folder, tools, &ticket, open)
    }

    fn leases(&self, project_id: &str) -> Result<&Leases, String> {
        if let Some(leases) = self.leases.get() {
            return Ok(leases);
        }
        let settings =
            Settings::load(&self.config, project_id).map_err(|error| error.to_string())?;
        Ok(self.leases.get_or_init(|| Leases::new(&settings)))
    }
}

/// The ticket id the issue number `number` names, whichever way it is written
///
/// The id is the number formatted back, so `012`, `+12` and `#12` are ticket 12
/// and meet the worker already on it.
///
/// # Errors
///
/// `number` is not an issue number.
fn ticket_id(number: &str) -> Result<String, String> {
    number
        .trim()
        .trim_start_matches('#')
        .parse()
        .ok()
        .and_then(TicketId::from_issue)
        .map(|id| id.to_string())
        .ok_or_else(|| format!("{number} is not an issue number"))
}

/// What a worker in `status` is doing, in a line's words
fn doing(status: Status) -> &'static str {
    match status {
        Status::Running => "running",
        Status::Paused => "paused",
        Status::WaitingOnMaintainer => "waiting on the maintainer",
        Status::Ended => "ended",
        _ => "in a state kelpie has no word for",
    }
}

/// The ticket `number` of `source`, as a worker's first message reads it
///
/// The title, a blank line, then the body.
///
/// # Errors
///
/// `number` is not an issue number, names no issue, names a closed one, or the
/// source cannot be asked. The text says which.
pub fn ticket_of(source: &dyn Tickets, number: &str) -> Result<Ticket, String> {
    let number = ticket_id(number)?;
    let id = number
        .parse()
        .ok()
        .and_then(TicketId::from_issue)
        .ok_or_else(|| format!("{number} is not an issue number"))?;
    let issue = source.issue(&id).map_err(|error| match error {
        ForgeError::NoIssue { .. } => format!("there is no issue {number} in this project"),
        other => other.to_string(),
    })?;
    if !issue.open {
        return Err(format!(
            "issue {number} is closed, so no worker starts on it"
        ));
    }
    Ok(Ticket {
        id: number,
        text: format!("{}\n\n{}", issue.title, issue.body),
    })
}

/// Start a worker on `ticket` in the project `project_id`, reading it from `source`
///
/// `source` is what [`AppState::source`] answered, so its refusal is the answer.
/// `open` builds the session, which the app's opener does in `open.rs`.
///
/// # Errors
///
/// The ticket is refused (see [`ticket_of`]), the project has no such checkout,
/// or the start fails (see [`Workers::start`]).
pub fn start_on(
    state: &AppState,
    project_id: &str,
    source: Result<&dyn Tickets, String>,
    ticket: &str,
    open: &Opener<'_>,
) -> Result<WorkerStarted, String> {
    let ticket = ticket_of(source?, ticket)?;
    let folder = state
        .project_path_of(project_id)
        .ok_or_else(|| format!("kelpie has no project {project_id}"))?;
    state
        .workers()
        .start(project_id, &folder, state.tools(), &ticket, open)
}

/// Start a worker on the GitHub issue `ticket` of the project
///
/// Answers `waiting: true` when no slot is free, and the ticket is in line.
///
/// # Errors
///
/// The ticket is closed, names no issue, or the project is not on GitHub; or the
/// worker could not start. The text says why.
#[tauri::command(async)]
#[specta::specta]
pub fn worker_start(state: State<'_, AppState>, ticket: String) -> Result<WorkerStarted, String> {
    start_ticket(&state, &ticket)
}

/// Start a worker on the GitHub issue `ticket` of the project: what the Start
/// button and a drop on Building both call
///
/// # Errors
///
/// As [`worker_start`].
pub fn start_ticket(state: &AppState, ticket: &str) -> Result<WorkerStarted, String> {
    let project = state.project_id(None)?;
    let source = state.source(&project);
    let source = source.as_ref().map(|source| &**source as &dyn Tickets);
    start_on(
        state,
        &project,
        source.map_err(Clone::clone),
        ticket,
        &open::open,
    )
}

/// Resume the Paused worker on `ticket` of the project `project_id`
///
/// `open` builds the session, which the app's opener does in `open.rs`.
///
/// # Errors
///
/// The project has no such checkout, or the resume is refused or fails (see
/// [`Workers::resume`]).
pub fn resume_on(
    state: &AppState,
    project_id: &str,
    ticket: &str,
    open: &Opener<'_>,
) -> Result<WorkerStarted, String> {
    let folder = state
        .project_path_of(project_id)
        .ok_or_else(|| format!("kelpie has no project {project_id}"))?;
    state
        .workers()
        .resume(project_id, &folder, state.tools(), ticket, open)
}

/// Read the past of the worker on the ticket, and follow it live on `on_event`
///
/// Answers the worker's log, oldest first. Every event the worker sends after
/// that goes to `on_event`, until the window closes it.
///
/// # Errors
///
/// The project has no worker on `ticket`, or its log cannot be read.
#[tauri::command(async)]
#[specta::specta]
pub fn worker_watch(
    state: State<'_, AppState>,
    ticket: String,
    on_event: Channel<SessionEvent>,
) -> Result<Vec<LogEntry>, String> {
    let project = state.project_id(None)?;
    let id = on_event.id();
    state.workers().watch(&project, &ticket, id, move |event| {
        on_event.send(event).is_ok()
    })
}

/// Stop sending the worker on the ticket's events to `on_event`
///
/// A tab calls it when it closes: a channel's send succeeds while the window
/// lives, so nothing else tells the feed the tab is gone.
#[tauri::command]
#[specta::specta]
pub fn worker_unwatch(
    state: State<'_, AppState>,
    ticket: String,
    on_event: Channel<SessionEvent>,
) -> Result<(), String> {
    let project = state.project_id(None)?;
    state.workers().unwatch(&project, &ticket, on_event.id());
    Ok(())
}

/// Start the next turn of the Paused worker on the ticket
///
/// A worker that is not Paused is refused, with a line saying what it is doing.
///
/// # Errors
///
/// The ticket has no worker, it is not Paused, or the worker could not start.
#[tauri::command(async)]
#[specta::specta]
pub fn worker_resume(state: State<'_, AppState>, ticket: String) -> Result<WorkerStarted, String> {
    let project = state.project_id(None)?;
    resume_on(&state, &project, &ticket, &open::open)
}
