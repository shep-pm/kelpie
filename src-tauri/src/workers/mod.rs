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
//! A worker's events go to its log and nowhere else yet.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use kelpie_engine::github::gh::{Gh, GithubSource, hold::Hold};
use kelpie_engine::github::{AppTokens, origin_of, system_tokens};
use kelpie_engine::lease::Leases;
use kelpie_engine::login::Store;
use kelpie_engine::session::Session;
use kelpie_engine::settings::{ConfigDir, Settings};
use kelpie_engine::ticket::{ForgeError, TicketId, Tickets};
use kelpie_engine::tools::Tools;
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::worker::{self, Opening, Paths, Records, Started, Ticket};
use serde::Serialize;
use specta::Type;
use tauri::State;

mod open;
mod plan;

use open::OpenOn;
pub use plan::{WorkerPlan, plan};

use crate::state::AppState;

#[cfg(test)]
mod tests;

/// The branch every GitHub project's worktrees are cut from
const BASE: &str = "main";

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
    sessions: Mutex<BTreeMap<(String, String), Arc<Session>>>,
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
    #[must_use]
    pub fn session(&self, project_id: &str, ticket: &str) -> Option<Arc<Session>> {
        lock(&self.sessions)
            .get(&(project_id.to_owned(), ticket.to_owned()))
            .cloned()
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
        // The session logs its own turn, which is all its events go to for now
        let started = worker::start(&project, ticket, |_| {}).map_err(|error| error.to_string())?;
        let waiting = matches!(started, Started::Waiting { .. });
        if let Started::Running { session, .. } = started {
            lock(&self.sessions).insert((project_id.to_owned(), ticket.id.clone()), session);
        }
        Ok(WorkerStarted {
            ticket: ticket.id.clone(),
            waiting,
        })
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

/// The ticket `number` of `source`, as a worker's first message reads it
///
/// The title, a blank line, then the body.
///
/// # Errors
///
/// `number` is not an issue number, names no issue, names a closed one, or the
/// source cannot be asked. The text says which.
pub fn ticket_of(source: &dyn Tickets, number: &str) -> Result<Ticket, String> {
    let id = number
        .trim()
        .trim_start_matches('#')
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
        id: number.trim().trim_start_matches('#').to_owned(),
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
    let project = state.project_id(None)?;
    let source = state.source(&project);
    let source = source.as_ref().map(|source| &**source as &dyn Tickets);
    start_on(
        &state,
        &project,
        source.map_err(Clone::clone),
        &ticket,
        &open::open,
    )
}
