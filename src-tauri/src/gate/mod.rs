//! The gate the app runs: one pass over each Review ticket's pull request per read of GitHub
//!
//! The order is the engine's ([`kelpie_engine::gate::pass`]): it takes what a pass read as
//! plain values and answers what to do. This module reads the world into those values and
//! `act` carries out the answer (`docs/design/gate-and-merge.md`, The order).
//!
//! [`Gate::after_read`] is what the board's read runs, with the pull requests that read
//! listed. A pass runs for each ticket whose worker is Ended and whose `kelpie/<id>` pull
//! request is open, or just left the listing, on a thread of its own: a pass waits on git
//! and on `gh`, and the poll thread must not. One pass works on a ticket at a time
//! (`Gate::take`), as one hand-over does, so a read that lands while a pass is at work on
//! a ticket leaves it be.
//!
//! - A pull request that left the listing is read once, by the number the gate kept, with
//!   [`Pulls::state`]: merged by hand leaves the gate, and closed is the Closed ruling.
//! - A ruling parks the worker waiting on the maintainer with its reason, as a hand-over's
//!   ruling does, and holds no slot. That worker gets no pass until its head moves or its
//!   pull request closes or merges: the next pass would only rule again.
//! - What a pass leaves for the next is kept in `gate.json`, beside `workers.json`
//!   ([`kelpie_engine::gate::Book`]), which also says which tickets are ready to merge:
//!   the board reads [`Gate::ready`] for each ticket's needs-you line.
//!
//! A red run is the worker's to fix (`red`): the failed logs and the files the branch changed
//! decide between a rerun, a ruling and a fix turn that carries the log.
//!
//! A ready head merges on the maintainer's yes and on nothing else. [`Gate::yes`] keeps the
//! yes with the head it was given on, and the next pass of that ticket carries it out
//! (`merge`): it marks a draft ready, waits out the checks, and merges pinned to the head,
//! or withdraws the yes where the head, CI or the base moved.
//!
//! A catch-up push and a fix turn reach the worker's own branch only: the rebase pushes
//! from the worker's clone with the credential git finds for its `origin`, and the fix
//! turn is a prompt to the worker's own session. The maintainer's `gh` login reads
//! GitHub and never reaches the worktree (`docs/design/tickets.md`, GitHub).
//!
//! The reading follows SK's `runner/gate/` (`docs/lifted.md`). Under Only on conflict, a
//! branch behind its base that git finds merges cleanly reads as current, and a head git
//! cannot judge is caught up, as under Always.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;

use kelpie_engine::ci::{Ci, HeadStatus};
use kelpie_engine::gate::{self, Book, Limits, Line, Look, Pull};
use kelpie_engine::github::Clock;
use kelpie_engine::sandbox::fence;
use kelpie_engine::settings::CatchUp;
use kelpie_engine::ticket::{MergeQueue, OpenPull, PullState, Pulls, TicketId};
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::worker::{Paths, Record, Records, Status};
use kelpie_engine::worktree::{self, Base, Remote};

use crate::workers::BASE;

mod act;
mod merge;
mod red;
#[cfg(test)]
mod tests;

/// Where a pass finds the project's limits, read from its settings each time
pub type Choose = Box<dyn Fn() -> Result<Limits, String> + Send + Sync>;

/// Sends the worker on a ticket back with a prompt, as a turn of its own session
///
/// The app's is [`crate::workers::Workers::fix_turn`], which a test replaces with the
/// worker's own agent.
pub type Fix = Box<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;

pub use merge::NotReady;

/// What a project's gate is made from
pub struct Parts {
    /// The project's id
    pub project_id: String,

    /// The project's checkout, which holds the remote each branch is fetched from
    pub folder: PathBuf,

    /// Where the project's workers live, whose worktrees a pass reads
    pub paths: Paths,

    /// The tree kelpie's own git reads go through (ADR 0001)
    pub tree: Arc<ProcessTree>,

    /// The project's worker records, held weakly
    ///
    /// The records keep where each worker's turns send their events, and a strong hold
    /// here could make a ring nothing drops.
    pub records: Weak<Mutex<Records>>,

    /// The project's pull requests, read once for one that left the listing
    pub pulls: Arc<dyn Pulls>,

    /// The project's CI, which a stuck run is rerun through
    pub ci: Arc<dyn Ci>,

    /// The base branch's merge queue, which a merge is watched through
    pub queue: Arc<dyn MergeQueue>,

    /// Where a pass reads the present moment from
    pub clock: Arc<dyn Clock>,

    /// Where a pass finds the project's limits
    pub limits: Choose,

    /// What sends a worker back with a prompt
    pub fix: Fix,
}

// The ports are trait objects, which no derive can print
impl std::fmt::Debug for Parts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Parts")
            .field("project_id", &self.project_id)
            .finish_non_exhaustive()
    }
}

/// A project's gate
pub struct Gate {
    project_id: String,
    folder: PathBuf,
    paths: Paths,
    tree: Arc<ProcessTree>,
    records: Weak<Mutex<Records>>,
    pulls: Arc<dyn Pulls>,
    ci: Arc<dyn Ci>,
    queue: Arc<dyn MergeQueue>,
    clock: Arc<dyn Clock>,
    limits: Choose,
    fix: Fix,

    /// The line each ticket showed after its last pass, which a yes that is refused says
    lines: Mutex<BTreeMap<TicketId, Line>>,

    /// What the gate keeps of each ticket, and `gate.json`, which holds it
    book: Mutex<Book>,

    /// The tickets with a pass at work, so one works on a ticket at a time
    busy: Busy,

    /// Held while a pass reads or changes the project's checkout
    ///
    /// Every ticket's fetch writes the same remote-tracking refs there, and two at once
    /// fail to lock them.
    checkout: Mutex<()>,
}

// The ports are trait objects, which no derive can print
impl std::fmt::Debug for Gate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gate")
            .field("project_id", &self.project_id)
            .finish_non_exhaustive()
    }
}

/// What a read came to, saying so when it failed
fn read_of<T, E: std::fmt::Display>(ticket: &str, what: &str, result: Result<T, E>) -> Option<T> {
    match result {
        Ok(read) => Some(read),
        Err(why) => {
            eprintln!("kelpie: ticket {ticket} was not passed, {what} failed: {why}");
            None
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a pass was started on
#[derive(Debug, Clone)]
enum Subject {
    /// The pull request is in the listing
    Open(Box<OpenPull>),
    /// It left the listing, and is read once by its number
    Left(NonZeroU64),
}

/// What `ticket`'s record said when its pass began, which is the only one the pass writes
///
/// A pass waits on git and on `gh`, and while it waits the maintainer can start the
/// worker or send it a Note. A write over such a record would show a running worker as
/// waiting on the maintainer, so a pass writes the record it began from and no other.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Standing {
    status: Status,
    reason: Option<String>,
}

impl Standing {
    fn of(record: &Record) -> Self {
        Self {
            status: record.status,
            reason: record.reason.clone(),
        }
    }

    fn holds(&self, record: &Record) -> bool {
        record.status == self.status && record.reason == self.reason
    }
}

/// One ticket's pass: who it is for, and what began it
#[derive(Debug)]
struct Work {
    ticket: String,
    id: TicketId,
    branch: String,
    subject: Subject,
    standing: Standing,
}

/// The tickets with a pass at work
type Busy = Arc<Mutex<BTreeSet<String>>>;

/// The ticket a pass is at work on, given back when it ends, however it ends
struct Working {
    busy: Busy,
    ticket: String,
}

impl Drop for Working {
    fn drop(&mut self) {
        lock(&self.busy).remove(&self.ticket);
    }
}

impl Gate {
    /// The gate of a project, with what the facts in its `gate.json` say
    ///
    /// # Errors
    ///
    /// `gate.json` cannot be read, or is not the gate's facts.
    pub fn new(parts: Parts) -> Result<Self, String> {
        let file = parts.paths.project().join(gate::FILE);
        let book = Book::load(file).map_err(|error| error.to_string())?;
        Ok(Self {
            project_id: parts.project_id,
            folder: parts.folder,
            paths: parts.paths,
            tree: parts.tree,
            records: parts.records,
            pulls: parts.pulls,
            ci: parts.ci,
            queue: parts.queue,
            clock: parts.clock,
            limits: parts.limits,
            fix: parts.fix,
            lines: Mutex::default(),
            book: Mutex::new(book),
            busy: Busy::default(),
            checkout: Mutex::new(()),
        })
    }

    /// Run a pass for each ticket whose worker is Ended and whose pull request is open or just left the listing
    ///
    /// `pulls` is what the read just listed. Each pass runs on a thread of its own, and
    /// a ticket another pass is at work on is left to it. A worker waiting on a ruling
    /// of the gate's gets a pass once its head moves, or its pull request is no longer
    /// open.
    pub fn after_read(self: &Arc<Self>, records: &[Record], pulls: &[OpenPull]) {
        self.withdraw_stale_ready(records);
        for record in records {
            let Some(work) = self.work_for(record, pulls) else {
                continue;
            };
            let Some(at_work) = self.take(&work.ticket) else {
                continue;
            };
            let gate = Arc::clone(self);
            thread::spawn(move || {
                let _at_work = at_work;
                gate.pass(&work);
            });
        }
    }

    /// Take the ready mark off each ticket whose worker is no longer Ended
    ///
    /// A worker the maintainer sent a Note or resumed is running and pushing again, so the
    /// head the gate found ready is not the one to merge. The gate gives such a worker no
    /// pass, so the mark would stay until the worker ended.
    fn withdraw_stale_ready(&self, records: &[Record]) {
        let mut book = lock(&self.book);
        let mut changed = false;
        for record in records.iter().filter(|r| r.status != Status::Ended) {
            let Ok(id) = record.ticket.parse::<TicketId>() else {
                continue;
            };
            if let Some(kept) = book.get(id).filter(|kept| kept.ready.is_some()) {
                let mut kept = kept.clone();
                kept.ready = None;
                *book.entry(id) = kept;
                changed = true;
            }
        }
        if changed && let Err(why) = book.save() {
            eprintln!("kelpie: the gate's facts did not keep: {why}");
        }
    }

    /// The tickets the gate found ready to merge, whose needs-you line says so
    #[must_use]
    pub fn ready(&self) -> BTreeSet<String> {
        lock(&self.book)
            .iter()
            .filter(|(_, kept)| kept.ready.is_some())
            .map(|(ticket, _)| ticket.to_string())
            .collect()
    }

    /// What the gate keeps of `ticket`, which a test reads back
    #[cfg(test)]
    pub(crate) fn kept(&self, ticket: &str) -> Option<gate::Kept> {
        let id = ticket.parse().ok()?;
        lock(&self.book).get(id).cloned()
    }

    /// Start `ticket` from what the gate kept, which a test sets up before a read
    #[cfg(test)]
    pub(crate) fn keep_for_test(&self, ticket: &str, change: impl FnOnce(&mut gate::Kept)) {
        let id = ticket.parse().expect("a ticket id");
        let mut book = lock(&self.book);
        change(book.entry(id));
        book.save().expect("gate.json is written");
    }

    /// Wait until no pass is at work, which a test needs to see what a read came to
    #[cfg(test)]
    pub(crate) fn idle(&self) {
        let since = std::time::Instant::now();
        while !lock(&self.busy).is_empty() {
            assert!(
                since.elapsed() < std::time::Duration::from_secs(30),
                "a pass never ended"
            );
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Whether a pass is at work on `ticket` now
    #[cfg(test)]
    pub(crate) fn at_work_on(&self, ticket: &str) -> bool {
        lock(&self.busy).contains(ticket)
    }

    /// The pass `record` calls for, `None` when it calls for none
    fn work_for(&self, record: &Record, pulls: &[OpenPull]) -> Option<Work> {
        let id: TicketId = record.ticket.parse().ok()?;
        let book = lock(&self.book);
        let kept = book.get(id);
        // Waiting on the gate's own ruling, which is the reason it put on the record. A
        // worker that waits for another reason (a question, a hand-over's ruling) waits on
        // the maintainer for that, and is no business of the gate's.
        let ruled = record.status == Status::WaitingOnMaintainer
            && kept.is_some_and(|kept| kept.rules_with(record.reason.as_deref()));
        if record.status != Status::Ended && !ruled {
            return None;
        }
        let subject = match pulls.iter().find(|pull| pull.branch == record.branch) {
            // A ruling at this head is the last word until the head moves
            Some(pull) if ruled && kept.is_some_and(|kept| kept.rules_at(&pull.head)) => {
                return None;
            }
            Some(pull) => Subject::Open(Box::new(pull.clone())),
            None => Subject::Left(NonZeroU64::new(kept?.pull?)?),
        };
        Some(Work {
            ticket: record.ticket.clone(),
            id,
            branch: record.branch.clone(),
            subject,
            standing: Standing::of(record),
        })
    }

    /// Take `ticket` for a pass, `None` when one is already at work on it
    fn take(&self, ticket: &str) -> Option<Working> {
        if !lock(&self.busy).insert(ticket.to_owned()) {
            return None;
        }
        Some(Working {
            busy: Arc::clone(&self.busy),
            ticket: ticket.to_owned(),
        })
    }

    /// One pass over `work`'s pull request, and what it comes to
    fn pass(&self, work: &Work) {
        let limits = match (self.limits)() {
            Ok(limits) => limits,
            Err(why) => {
                eprintln!(
                    "kelpie: ticket {} was not passed, its settings cannot be read: {why}",
                    work.ticket
                );
                return;
            }
        };
        let Some(look) = self.look(work, &limits) else {
            return;
        };
        let now = self.clock.now();
        let mut kept = lock(&self.book).get(work.id).cloned().unwrap_or_default();
        let started_with_yes = kept.yes.is_some();
        if let Pull::Open(pull) = &look.pull {
            kept.pull = Some(pull.number.get());
        }
        kept.ready = None;
        // A yes, or a merge already sent, is carried out before the order runs again
        let after = match &look.pull {
            Pull::Open(pull) if kept.yes.is_some() || kept.sent.is_some() => {
                self.merge_pass(work, &look, pull, &mut kept, &limits, now)
            }
            _ => merge::After::Order,
        };
        let (carried, line) = match after {
            merge::After::Carried(carried) => (carried, Some(merge::line_of(&kept))),
            merge::After::Order => {
                let step = gate::pass(&look, &mut kept, &limits, now);
                let line = step.line();
                (
                    self.carry_out(work, &look, &mut kept, step, &limits, now),
                    line,
                )
            }
        };
        let mut book = lock(&self.book);
        match carried {
            act::Carried::Keep => {
                // The maintainer's yes can land while the pass is at work, on the head it found ready
                if kept.yes.is_none()
                    && !started_with_yes
                    && let Some(said) = book.get(work.id).and_then(|then| then.yes.clone())
                    && kept.ready.as_deref() == Some(said.as_str())
                {
                    kept.yes = Some(said);
                }
                *book.entry(work.id) = kept;
            }
            act::Carried::Forget => book.forget(work.id),
        }
        if let Err(why) = book.save() {
            eprintln!(
                "kelpie: the gate's facts of ticket {} did not keep: {why}",
                work.ticket
            );
        }
        drop(book);
        let mut lines = lock(&self.lines);
        match (carried, line) {
            (act::Carried::Keep, Some(line)) => {
                lines.insert(work.id, line);
            }
            _ => {
                lines.remove(&work.id);
            }
        }
    }

    /// What the pass reads of GitHub and of git, `None` when it cannot be read
    ///
    /// A pass that cannot read waits for the next read: a branch that git or the
    /// forge will not show is not one the gate may rule on.
    fn look(&self, work: &Work, limits: &Limits) -> Option<Look> {
        let pull = match &work.subject {
            Subject::Open(pull) => return self.look_open(work, pull, limits),
            Subject::Left(number) => match self.pulls.state(*number) {
                Ok(PullState::Merged) => Pull::Merged,
                Ok(PullState::Closed) => Pull::Closed,
                // The listing missed it, which the next read will not
                Ok(PullState::Open) => return None,
                Err(why) => {
                    eprintln!(
                        "kelpie: pull request {number} of ticket {} was not read: {why}",
                        work.ticket
                    );
                    return None;
                }
            },
        };
        Some(Look {
            pull,
            base: Base::Current,
            head: String::new(),
            agents_files: Vec::new(),
            merges_clean: false,
            base_runs_pull_workflow: false,
        })
    }

    fn look_open(&self, work: &Work, pull: &OpenPull, limits: &Limits) -> Option<Look> {
        let (tree, repo) = (&*self.tree, self.folder.as_path());
        let _checkout = lock(&self.checkout);
        let base = read_of(
            &work.ticket,
            "its base",
            worktree::base_of(tree, repo, &work.branch, &pull.head, &Remote::Origin, BASE),
        )?;
        let head = read_of(
            &work.ticket,
            "its worktree's head",
            worktree::head(tree, repo, &self.paths.worktree(&work.ticket)),
        )?;
        let (_, agents_files) = read_of(
            &work.ticket,
            "its diff",
            fence::changed(tree, repo, &work.branch, &Remote::Origin, None, BASE),
        )?;
        // Asked only where the answer decides: under Only on conflict, of a branch behind
        // its base. A head git cannot judge is not clean, so it is caught up.
        let merges_clean = base == Base::Behind
            && limits.catch_up == CatchUp::Conflicts
            && matches!(
                worktree::merge_conflicts(tree, repo, &pull.head, BASE),
                Ok(false)
            );
        let base_runs_pull_workflow = pull.checks == HeadStatus::None
            && worktree::runs_on_pull_requests(tree, repo, BASE).unwrap_or(false);
        Some(Look {
            pull: Pull::Open(pull.clone()),
            base,
            head,
            agents_files,
            merges_clean,
            base_runs_pull_workflow,
        })
    }

    /// The project's records, while kelpie still holds them
    fn records(&self) -> Option<Arc<Mutex<Records>>> {
        self.records.upgrade()
    }

    /// The error a failed write of a record is said with, so the loss is named
    fn lost(&self, ticket: &str, written: io::Result<()>, what: &str) {
        if let Err(why) = written {
            eprintln!(
                "kelpie: project {}: the record of ticket {ticket} did not keep {what}: {why}",
                self.project_id
            );
        }
    }
}
