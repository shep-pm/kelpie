//! The worker commands: what Start, a drop on Building and the PM's `start_worker` call
//!
//! [`Workers`] is the app's host for [`kelpie_engine::worker::start()`]. It keeps
//! what outlives one start: each project's records, the leases, the GitHub App
//! tokens, the `gh` on the machine's `PATH`, the sessions of the workers that are
//! running, and one [`GithubSource`] per project, which the board's poller shares
//! so every GitHub call of a project goes through one [`Hold`].
//!
//! A turn that ends with its branch clean hands its draft over
//! ([`hand_over`], `docs/design/tickets.md`, Use case): the sink `start` gives the
//! engine forwards the worker's events to its feed and then opens the draft on a
//! thread of its own, and [`Hands`] is what the board's read runs again for every
//! worker that still owes one.
//!
//! A ticket is read before a slot is held, so a closed issue, a number naming
//! nothing and a project whose origin is not on GitHub are each refused with a
//! line saying which. A start does not refuse a ticket that is blocked or has
//! sub-issues: dispatch does.
//!
//! A worker's events go to its log and to its [`Feed`], which the window watches
//! with [`worker_watch`] and stops watching with [`worker_unwatch`]. A Paused
//! worker resumes with [`worker_resume`], when the maintainer says so.
//!
//! A worker that runs is steered from its tab: [`worker_note`], [`worker_stop_and_tell`]
//! and [`worker_cancel`] are the engine's own moves, and its sandbox request is answered
//! there with [`worker_answer`], as the PM's is (`docs/design/workers.md`, Talking to a
//! worker). Each refuses a ticket with no live session by name: kelpie keeps a worker's
//! session while its turn runs and after it ends with the work committed, so a Note
//! between turns is that worker's next turn, and a Paused worker is resumed, not told. A
//! Note to a worker waiting on the maintainer answers its ruling and sends it back as a fix
//! turn (`fix_turn.rs`). What each one calls is in `tell.rs`, which is where
//! the engine's own telling lives too (`crates/kelpie-engine/src/worker/tell.rs`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};

use kelpie_engine::github::gh::{
    Gh, GithubActions, GithubPulls, GithubQueue, GithubSource, hold::Hold,
};
use kelpie_engine::github::{AppTokens, origin_of, system_tokens};
use kelpie_engine::lease::Leases;
use kelpie_engine::local_paths::LocalPaths;
use kelpie_engine::login::Store;
use kelpie_engine::session::{SandboxAnswer, Session, log};
use kelpie_engine::settings::{ConfigDir, Settings};
use kelpie_engine::ticket::{ForgeError, TicketId, Tickets};
use kelpie_engine::tools::{Tools, WorkerStarter};
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::usage::{InFlight, Timestamp};
use kelpie_engine::worker::{self, Opening, Paths, Records, Seat, Started, Status, Ticket};
use kelpie_protocol::{LogEntry, SessionEvent};
use serde::Serialize;
use specta::Type;
use tauri::State;
use tauri::ipc::Channel;

mod feed;
mod fix_turn;
mod gates;
pub mod hand_over;
mod open;
mod plan;
mod tell;

pub use feed::Feed;
pub use gates::{Open, app_open};
pub use hand_over::{Forge, Hands};
pub(crate) use open::OpenOn;
pub use plan::{WorkerPlan, plan};
pub use tell::{NoteOutcome, answer_on, cancel_on, note_on, note_through, stop_and_tell_on};

use crate::board::poll::Requests;
use crate::gate::Gate;
use crate::state::AppState;

#[cfg(test)]
mod tests;

/// The branch every GitHub project's worktrees are cut from
pub(crate) const BASE: &str = "main";

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

/// The feeds of every worker, by project and ticket
type Feeds = Arc<Mutex<BTreeMap<(String, String), Arc<Feed>>>>;

/// The sessions of the workers kelpie keeps, by project and ticket
type SessionMap = Mutex<BTreeMap<(String, String), Arc<Session>>>;

/// The same, held behind an `Arc`
///
/// So a finished turn's hand-over can reach its worker's session on a thread of its
/// own ([`hand_over::Hands::retry`], [`hand_over::Sink`]).
type Sessions = Arc<SessionMap>;

/// A project's GitHub calls, made through one hold
///
/// The board's poller reads the tickets through `source`, a hand-over opens its
/// drafts through `pulls`, the gate reruns a stuck run through `ci` and watches a merge through `queue`, so a project's
/// calls are paced as one set
/// (`docs/design/tickets.md`, GitHub).
#[derive(Debug)]
struct Calls {
    source: Arc<GithubSource>,
    pulls: Arc<GithubPulls>,
    ci: Arc<GithubActions>,
    queue: Arc<GithubQueue>,
}

/// What one project's workers run on, read for one command
///
/// The plan, the folders and the records the engine's [`worker::Project`] borrows, held
/// together so one call can hand all three to it.
#[derive(Debug)]
struct Standing {
    plan: WorkerPlan,
    paths: Paths,
    records: Arc<Mutex<Records>>,
    usage: InFlight,
}

/// What the app keeps for its workers, across starts
pub struct Workers {
    data_dir: PathBuf,
    tree: Arc<ProcessTree>,
    config: ConfigDir,
    logins: Store,
    tokens: Arc<AppTokens>,
    /// The `gh` on the machine's `PATH`, `None` when there is none
    gh: Option<PathBuf>,
    /// Built on the first start, from the settings of the project it is for
    leases: OnceLock<Arc<Leases>>,
    records: Mutex<BTreeMap<String, Arc<Mutex<Records>>>>,
    /// Each project's ledger of worker turns, by project, built on the first ask
    ///
    /// One per project, so the drafts a start opened are still reachable when kelpie
    /// quits and writes the turns it cut off.
    usage: Mutex<BTreeMap<String, InFlight>>,
    /// The sessions of the workers that hold a slot, by project and ticket
    ///
    /// The engine holds a worker's session only weakly, so this map is what keeps
    /// one alive. [`Workers::session`] and every start give up the sessions of
    /// workers that no longer hold a slot.
    sessions: Sessions,
    /// The channels watching each worker, by project and ticket
    ///
    /// A feed outlives its worker's turns, so a tab stays attached across a Resume.
    feeds: Feeds,
    sources: Mutex<BTreeMap<String, Arc<Calls>>>,
    /// What each project's hand-overs ask of GitHub, by project
    ///
    /// Built from [`Workers::calls`], or stood in by [`Workers::stand_in_forge`].
    forges: Mutex<BTreeMap<String, Arc<Forge>>>,
    /// Each project's tickets with a hand-over at work, by project
    ///
    /// Held here, and not in a [`Hands`], so that every [`Hands`] of a project, the one
    /// a start made and the one a board read made, shares one set
    /// ([`hand_over::Busy`]).
    handovers: Mutex<BTreeMap<String, hand_over::Busy>>,
    /// Each project's gate, by project, built on the first ask ([`Workers::gate`])
    gates: Mutex<BTreeMap<String, Arc<Gate>>>,
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
            tokens: Arc::new(system_tokens(tree, data_dir)),
            gh: worker::login::helper_on_path(),
            leases: OnceLock::new(),
            records: Mutex::default(),
            usage: Mutex::default(),
            sessions: Arc::default(),
            feeds: Arc::default(),
            sources: Mutex::default(),
            forges: Mutex::default(),
            handovers: Mutex::default(),
            gates: Mutex::default(),
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
        Ok(Arc::clone(&self.calls(project_id, folder)?.source))
    }

    /// The GitHub calls of the project `project_id`, made on the first ask
    ///
    /// One [`Hold`] per project, so its ticket reads and its drafts are paced as one
    /// set. A project's drafts are refused before any call when a title or a body names
    /// this machine, which is what `LocalPaths` holds its own folders for.
    ///
    /// # Errors
    ///
    /// As [`Workers::source`].
    fn calls(&self, project_id: &str, folder: &Path) -> Result<Arc<Calls>, String> {
        let mut kept = lock(&self.sources);
        if let Some(calls) = kept.get(project_id) {
            return Ok(Arc::clone(calls));
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
        let calls = Arc::new(Calls {
            source: Arc::new(GithubSource::new(hold.clone(), repo.clone())),
            pulls: Arc::new(GithubPulls::new(
                hold.clone(),
                repo.clone(),
                LocalPaths::new([folder, self.data_dir.as_path()]),
            )),
            ci: Arc::new(GithubActions::new(hold.clone(), repo.clone())),
            queue: Arc::new(GithubQueue::new(hold, repo)),
        });
        kept.insert(project_id.to_owned(), Arc::clone(&calls));
        Ok(calls)
    }

    /// The session of the worker on `ticket`, while kelpie can still tell it
    ///
    /// `None` for a worker that is Paused or waiting on the maintainer, whose next turn
    /// is the start its Resume makes, and for one with no record.
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

    /// Drop the session of the worker on `ticket`, which is what a relaunch leaves
    ///
    /// While kelpie runs, an ended worker keeps its session ([`Workers::keeps_session`]).
    /// A test that wants a worker whose session went with the launch that started it, and
    /// whose record still says what it did, drops it here.
    #[cfg(test)]
    pub(crate) fn forget_session(&self, project_id: &str, ticket: &str) {
        let gone = lock(&self.sessions)
            .remove(&(project_id.to_owned(), ticket.to_owned()))
            .is_some();
        assert!(gone, "ticket {ticket} kept no session to forget");
    }

    /// Drop the session of each worker its record says kelpie cannot tell
    ///
    /// Run from the caller's thread and never from a session's own event sink:
    /// the last reference to a session stops its harness and joins its thread.
    fn give_up_idle(&self) {
        let keys: Vec<(String, String)> = lock(&self.sessions).keys().cloned().collect();
        let idle: Vec<_> = keys
            .into_iter()
            .filter(|(project, ticket)| !self.keeps_session(project, ticket))
            .collect();
        let given: Vec<_> = {
            let mut sessions = lock(&self.sessions);
            idle.iter().filter_map(|key| sessions.remove(key)).collect()
        };
        drop(given);
    }

    /// Whether the worker on `ticket` keeps a session kelpie can tell
    ///
    /// A worker holds its session while its turn runs, after that turn ends with the
    /// work committed, and while a reviewer reads its pull request: [`worker::note`] sends
    /// a Note to a worker whose turn ended as its next turn, and a review round, or the fix
    /// turn a read sends back, resumes the same conversation
    /// (`docs/design/workers.md`, Ending, quitting and resuming). A Paused worker, or one
    /// waiting on the maintainer, gives its session up: its next turn is the start its
    /// Resume makes, which builds a session of its own.
    fn keeps_session(&self, project_id: &str, ticket: &str) -> bool {
        let records = lock(&self.records).get(project_id).cloned();
        records.is_some_and(|records| {
            lock(&records).get(ticket).is_some_and(|record| {
                matches!(record.status, Status::Ended | Status::Reviewing)
                    || (record.status == Status::Running && record.seat == Seat::Held)
            })
        })
    }

    /// Stop logging every worker, so a quit does not log the `Failed` of its turn
    pub fn close_logs(&self) {
        for session in lock(&self.sessions).values() {
            session.close_log();
        }
    }

    /// Write a stopped line for every worker turn kelpie is cutting off
    ///
    /// Run before the process tree stops, so the quit writes these turns and no
    /// `Failed` from a stopped harness writes one after it.
    pub fn stop_usage(&self) {
        let ledgers: Vec<InFlight> = lock(&self.usage).values().cloned().collect();
        for usage in ledgers {
            usage.stop_all(Timestamp::now());
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
        let standing = self.standing(project_id)?;
        let on = OpenOn {
            data_dir: &self.data_dir,
            logins: &self.logins,
            config: &self.config,
            project_id,
            folder,
            plan: &standing.plan,
        };
        let opener = |opening: &Opening<'_>| open(&on, opening);
        let project = self.project_of(&standing, project_id, folder, tools, &opener)?;
        // The session logs its own turn, the feed follows it live, and a turn that ends
        // with its branch clean hands its draft over, on a thread of its own
        let hands = self.hands(project_id, folder, &standing.plan)?;
        let mut sink = hand_over::Sink {
            feed: self.feed(project_id, &ticket.id),
            ticket: ticket.id.clone(),
            hands,
        };
        let started = worker::start(&project, ticket, move |event| sink.hear(event))
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

    /// What reads the hosts each worker of the project `project_id` waits to reach, by ticket
    ///
    /// The host of each sandbox request still waiting on the maintainer, oldest
    /// first. A worker with none is left out. The reader holds the feeds and not
    /// the app's state, so the board's poller keeps it on its own thread.
    pub fn requests_of(&self, project_id: &str) -> Requests {
        let feeds = Arc::clone(&self.feeds);
        let project = project_id.to_owned();
        Box::new(move || {
            let mine: Vec<(String, Arc<Feed>)> = lock(&feeds)
                .iter()
                .filter(|((owner, _), _)| *owner == project)
                .map(|((_, ticket), feed)| (ticket.clone(), Arc::clone(feed)))
                .collect();
            mine.into_iter()
                .map(|(ticket, feed)| (ticket, feed.waiting()))
                .filter(|(_, hosts)| !hosts.is_empty())
                .collect()
        })
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

    /// Whether the worker on `ticket` has had a turn, which gave it a session to resume
    fn has_session(&self, project_id: &str, ticket: &str) -> Result<bool, String> {
        let records = self.records(project_id)?;
        let records = lock(&records);
        records
            .get(ticket)
            .map(|record| record.session.is_some())
            .ok_or_else(|| format!("there is no worker on ticket {ticket}"))
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
    /// A worker that never had a turn, as one whose sandbox was refused on its
    /// first start, has no conversation to carry on: its message is the ticket's own
    /// text, read from `source`, as a start sends it.
    ///
    /// # Errors
    ///
    /// The project has no worker on `ticket`, or it is not Paused: the line says what
    /// it is doing. Or a worker with no session has no ticket to read (see
    /// [`ticket_of`]). Or the start fails (see [`Workers::start`]).
    pub fn resume(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
        source: Result<&dyn Tickets, String>,
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
        let ticket = if self.has_session(project_id, &ticket)? {
            Ticket {
                id: ticket,
                text: CARRY_ON.to_owned(),
            }
        } else {
            ticket_of(source?, &ticket)?
        };
        self.start(project_id, folder, tools, &ticket, open)
    }

    /// What the project `project_id` runs its workers on, its records read from disk on the first ask
    ///
    /// # Errors
    ///
    /// The settings cannot be used, or the records file cannot be read.
    fn standing(&self, project_id: &str) -> Result<Standing, String> {
        let paths = Paths::new(&self.data_dir, project_id);
        Ok(Standing {
            // Read before `paths` goes into the list, which it names the ledger in
            usage: self.ledger(&paths, project_id),
            plan: plan(&self.config, project_id)?,
            paths,
            records: self.records(project_id)?,
        })
    }

    /// The ledger of the project `project_id`'s worker turns, made on the first ask
    fn ledger(&self, paths: &Paths, project_id: &str) -> InFlight {
        let mut kept = lock(&self.usage);
        if let Some(usage) = kept.get(project_id) {
            return usage.clone();
        }
        let usage = InFlight::new(paths.usage());
        kept.insert(project_id.to_owned(), usage.clone());
        usage
    }

    /// The engine's [`worker::Project`] for `standing`, whose workers it opens with `open`
    fn project_of<'a>(
        &'a self,
        standing: &'a Standing,
        project_id: &str,
        folder: &'a Path,
        tools: Option<&'a Tools>,
        open: &'a dyn Fn(&Opening<'_>) -> Result<Session, String>,
    ) -> Result<worker::Project<'a>, String> {
        let leases = self.leases(project_id)?;
        Ok(worker::Project {
            paths: &standing.paths,
            folder,
            base: BASE,
            tree: &self.tree,
            slots: standing.plan.slots,
            turn_timeout: standing.plan.turn_timeout,
            records: &standing.records,
            binding: &standing.plan.binding,
            network_allowlist: &standing.plan.network_allowlist,
            reads: standing.plan.reads,
            tools,
            tokens: Some(&self.tokens),
            gh: self.gh.as_deref(),
            leases: Some(leases.as_ref()),
            usage: Some(&standing.usage),
            open,
        })
    }

    fn leases(&self, project_id: &str) -> Result<&Arc<Leases>, String> {
        if let Some(leases) = self.leases.get() {
            return Ok(leases);
        }
        let settings =
            Settings::load(&self.config, project_id).map_err(|error| error.to_string())?;
        Ok(self.leases.get_or_init(|| Arc::new(Leases::new(&settings))))
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
        Status::Reviewing => "reviewing",
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

/// The PM's `start_worker`, for the project `project_id`, whose checkout is `folder`
///
/// `tools` is kelpie's tools server, which each worker this starts is given. It is
/// held weakly, since the server holds the PM's token and the token holds this
/// starter: holding the server back would close the loop. `OnProject` says what
/// that would cost.
#[must_use]
pub fn starter_for(
    workers: &Arc<Workers>,
    project_id: &str,
    folder: &Path,
    tools: Option<&Arc<Tools>>,
) -> Arc<dyn WorkerStarter> {
    Arc::new(OnProject {
        workers: Arc::clone(workers),
        project_id: project_id.to_owned(),
        folder: folder.to_owned(),
        tools: tools.map(Arc::downgrade),
    })
}

/// The PM's `start_worker` reaching the one start function, for one project
///
/// The ticket is read from the project's source exactly as the Start button and a
/// drop on Building read it, so the PM is refused the same lines the maintainer
/// would be. What a start answers becomes the tool's one line.
///
/// The tools are held weakly, which is what keeps `Tools` droppable. The strong
/// ring is `Tools` → its live token → the PM's role → this starter, so a strong
/// `Tools` here is a cycle: the server could only drop once its own session and
/// every connection of it were gone, which leaves the door's socket in the data
/// folder and its thread unjoined, and hands the last drop to a connection task
/// that would then join the thread it runs on. Held weakly, the app is the only
/// strong holder, and dropping it closes the door from a thread the door is not
/// on. A start that finds the server gone gives the worker no kelpie tools, as a
/// run whose door never opened does.
#[derive(Debug)]
struct OnProject {
    workers: Arc<Workers>,
    project_id: String,
    folder: PathBuf,
    tools: Option<Weak<Tools>>,
}

impl WorkerStarter for OnProject {
    fn start(&self, ticket: &str) -> Result<String, String> {
        let source = self.workers.source(&self.project_id, &self.folder)?;
        let ticket = ticket_of(source.as_ref(), ticket)?;
        let tools = self.tools.as_ref().and_then(Weak::upgrade);
        let started = self.workers.start(
            &self.project_id,
            &self.folder,
            tools.as_deref(),
            &ticket,
            &open::open,
        )?;
        Ok(said(&started))
    }
}

/// What a start answers the PM, in one line
fn said(started: &WorkerStarted) -> String {
    if started.waiting {
        return format!("#{} waits for a free slot", started.ticket);
    }
    format!("Started a worker on #{}", started.ticket)
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
/// `source` is read only for a worker that never had a turn (see
/// [`Workers::resume`]). `open` builds the session, which the app's opener does in
/// `open.rs`.
///
/// # Errors
///
/// The project has no such checkout, or the resume is refused or fails (see
/// [`Workers::resume`]).
pub fn resume_on(
    state: &AppState,
    project_id: &str,
    ticket: &str,
    source: Result<&dyn Tickets, String>,
    open: &Opener<'_>,
) -> Result<WorkerStarted, String> {
    let folder = state
        .project_path_of(project_id)
        .ok_or_else(|| format!("kelpie has no project {project_id}"))?;
    state
        .workers()
        .resume(project_id, &folder, state.tools(), ticket, source, open)
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
    let source = state.source(&project);
    let source = source.as_ref().map(|source| &**source as &dyn Tickets);
    resume_on(
        &state,
        &project,
        &ticket,
        source.map_err(Clone::clone),
        &open::open,
    )
}

/// Tell the worker on the ticket `text`, without interrupting its turn
///
/// Answers `queued` when the text waits in the ticket's record for the worker's next prompt,
/// and `sent` when it went out as a turn of its own.
///
/// # Errors
///
/// The ticket has no live session, or the record could not be written. The text says which.
#[tauri::command(async)]
#[specta::specta]
pub fn worker_note(
    state: State<'_, AppState>,
    ticket: String,
    text: String,
) -> Result<NoteOutcome, String> {
    note_on(&state, &ticket, &text)
}

/// Cancel the worker on the ticket's turn and send it `text` as its next prompt
///
/// Answers what [`worker_note`] does: `sent` for the text that goes out as the turn that
/// follows the cancelled one, and `queued` for the text kept for the worker's next start.
///
/// # Errors
///
/// As [`worker_note`].
#[tauri::command(async)]
#[specta::specta]
pub fn worker_stop_and_tell(
    state: State<'_, AppState>,
    ticket: String,
    text: String,
) -> Result<NoteOutcome, String> {
    stop_and_tell_on(&state, &ticket, &text)
}

/// Cancel the worker on the ticket's turn and leave it Paused
///
/// The worker gives its slot back, and Resume starts its next turn.
///
/// # Errors
///
/// As [`worker_note`].
#[tauri::command(async)]
#[specta::specta]
pub fn worker_cancel(state: State<'_, AppState>, ticket: String) -> Result<(), String> {
    cancel_on(&state, &ticket)
}

/// Answer the sandbox request `id` of the worker on the ticket
///
/// The worker's session takes the answer, which lets the host through the sandbox its turn
/// is still running in, and the board stops marking the ticket as needing the maintainer.
///
/// # Errors
///
/// The ticket has no live session, the request is over, the sandbox could not take the host,
/// or the host could not be kept to the project. The text is for the maintainer to read.
#[tauri::command(async)]
#[specta::specta]
pub fn worker_answer(
    state: State<'_, AppState>,
    ticket: String,
    id: String,
    answer: SandboxAnswer,
) -> Result<(), String> {
    answer_on(&state, &ticket, &id, answer)
}
