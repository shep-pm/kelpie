//! The review pass the app runs on a ticket whose head CI let through
//!
//! The order is the engine's ([`kelpie_engine::review::step`]): it says the one thing a
//! pass does next from what the pass kept and what it just read. This module holds what a
//! pass runs on, and `act` carries the answer out (`docs/design/review.md`, The pass).
//!
//! [`Review`] is one per project, built beside its gate. It holds the reviewers the
//! project lists in the Reviewers setting, resolved through [`Lineup::of`] each time a
//! step runs so a change on the settings page applies to the next one, and every ticket's
//! facts in `review.json`, beside `workers.json` and `gate.json`
//! ([`kelpie_engine::review::Book`]). Each read is a fresh session of the reviewer named
//! by the pass ([`kelpie_engine::review::SessionReviewer`], `open`).
//!
//! The pass and the gate step each other. The gate gives
//! [`kelpie_engine::gate::Step::Review`] for a head
//! the pass has not read ([`Review::looked`]), [`Review::step`] runs one step, and a step
//! that leaves nothing to wait on hands the ticket back to the gate, which runs its order
//! again. A step that sends the worker a turn, rules, or finds no slot free holds the
//! ticket until the next read of GitHub, and one that never ran at all, a fix turn the
//! worker was never sent, leaves the pass exactly where it was, so the next read runs the
//! same step instead of ruling on a turn that never happened. A read never holds the book
//! of the project's passes, which every other ticket's pass reads while it is at work.
//!
//! A read takes a slot like any turn (`docs/design/workers.md`, Slots): the record is set
//! [`kelpie_engine::worker::Status::Reviewing`] before the read and back to
//! [`kelpie_engine::worker::Status::Ended`] after it, so the
//! read holds a slot and the ticket stays in Review. With none free the read waits for the
//! next GitHub read, which steps a Reviewing ticket too (`gate`).
//!
//! A reviewer's failing test is run by `act` through [`Testing`]: [`SandboxedTests`] runs each
//! as `kelpie lease run cargo-test` in the worker's own sandbox policy, so it waits its turn
//! behind the project's other test runs (`tests_run`).
//!
//! A head a catch-up pushed is vouched for when the head it replaced was read or vouched
//! for ([`Review::vouched`]), as SK's `send_unread` does, so a caught-up head needs no new
//! read.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use kelpie_engine::agents::{Agents, Binding};
use kelpie_engine::gate::Reviewed;
use kelpie_engine::github::Clock;
use kelpie_engine::review::{
    self, Book, Lineup, RULE_READS, Review as Pass, ReviewStage, Reviewer,
};
use kelpie_engine::settings::{ConfigDir, Resolved, Settings, TestCommand};
use kelpie_engine::ticket::{TicketId, Tickets};
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::worker::{Paths, Records};
use kelpie_engine::worktree;

use crate::gate::Fix;
use crate::workers::{WorkerPlan, plan};

mod act;
mod open;
#[cfg(test)]
mod tests;
mod tests_run;

pub use open::{ReviewOpen, app_make, reviewer_open};
pub use tests_run::{Exit, Gear, Kit, Launch, SandboxedTests, Testing, app_testing};

/// The folder of the project's, beside its worker and review folders, that holds the kept
/// copies of the reviewers' tests, the record of a test in a worktree and nothing else
///
/// The worker's policy writes its own state folder and no other, and a reviewer writes its
/// build folder's tests folder, so neither can change what a later run reads.
pub(crate) const KEPT: &str = "kept-tests";

/// Builds one ticket's reviewer of one listed reviewer's own
///
/// The app's builds a [`kelpie_engine::review::SessionReviewer`] on the [`Binding`] the
/// reviewer's agent file and the project's Reviewer harness, model and effort rows give
/// it, which is the one the read's prompt was written for. A test's hands in a
/// `fake::FakeReviewer` (the engine's, under its `testing` feature) scripted for the
/// ticket.
pub type Make =
    dyn Fn(&str, &Binding, &WorkerPlan) -> Result<Arc<dyn Reviewer>, String> + Send + Sync;

/// What a project's review pass is made from
pub struct Parts {
    /// The project's id
    pub project_id: String,

    /// The project's checkout, which each ticket's worktree is a worktree of
    pub folder: PathBuf,

    /// Where the project's workers and reviewers live
    pub paths: Paths,

    /// The tree kelpie's own git reads and each reviewer's session go through (ADR 0001)
    pub tree: Arc<ProcessTree>,

    /// The project's worker records, held weakly, as the gate's are
    pub records: Weak<Mutex<Records>>,

    /// The project's tickets, whose issue gives a reviewer the acceptance criteria
    pub tickets: Arc<dyn Tickets>,

    /// The folder settings and agent files are read from
    pub config: ConfigDir,

    /// Where a step reads the present moment from
    pub clock: Arc<dyn Clock>,

    /// What sends the worker on a ticket back with a prompt
    pub fix: Fix,

    /// What builds one ticket's reviewer
    pub make: Arc<Make>,

    /// What runs a reviewer's test, and what it reaches
    pub testing: Testing,
}

// The ports are closures and trait objects, which no derive can print
impl std::fmt::Debug for Parts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Parts")
            .field("project_id", &self.project_id)
            .finish_non_exhaustive()
    }
}

/// A project's review pass
pub struct Review {
    project_id: String,
    folder: PathBuf,
    paths: Paths,
    tree: Arc<ProcessTree>,
    records: Weak<Mutex<Records>>,
    tickets: Arc<dyn Tickets>,
    config: ConfigDir,
    clock: Arc<dyn Clock>,
    fix: Fix,
    make: Arc<Make>,
    testing: Testing,

    /// What each ticket's pass keeps, and `review.json`, which holds it
    book: Mutex<Book>,
}

// The ports are closures and trait objects, which no derive can print
impl std::fmt::Debug for Review {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Review")
            .field("project_id", &self.project_id)
            .finish_non_exhaustive()
    }
}

/// Where a ticket's pull request stands for one step of its pass
#[derive(Debug, Clone)]
pub struct At<'a> {
    /// The ticket
    pub ticket: &'a str,

    /// The pull request's number
    pub number: u64,

    /// The head on `origin`, which a reviewer reads
    pub head: &'a str,
}

/// What a step left for the gate
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// The step did its part, and the gate's order runs again for the ticket
    Again,

    /// The pass waits: the worker has a turn, no slot was free, or a read could not run
    Held,

    /// The pass ruled, so the ticket's line is the ruling's reason and the gate holds it
    /// off its passes until its head moves
    Ruled(review::Ruling),

    /// The step never ran, so the pass is kept where it was and the next read of GitHub
    /// runs the same step again
    ///
    /// A turn the worker was never sent is the case this is for: the pass is kept at the
    /// findings it could not send, rather than at the fix turn it is waiting on, which
    /// would be ruled a fix that pushed nothing.
    Undone,
}

/// What one step reads of the settings and the agent files
pub(super) struct Standing {
    lineup: Lineup,
    settings: Resolved,
    agents: Agents,
    test_command: Option<TestCommand>,
    plan: WorkerPlan,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How many reads `pass` has had, which is what its line counts the next one from
///
/// The same rule as [`Lineup::reads`]: a reviewer the pass passed over counts for nothing.
/// It is read here as well as through the lineup because a pass whose settings cannot be
/// read still has to say where it stands.
fn reads_had(pass: &Pass, passed_over: &[String]) -> u32 {
    u32::try_from(
        pass.ran
            .iter()
            .filter(|name| !passed_over.contains(name))
            .count(),
    )
    .unwrap_or(u32::MAX)
}

/// What a read came to, saying so when it failed
fn read_of<T, E: std::fmt::Display>(ticket: &str, what: &str, result: Result<T, E>) -> Option<T> {
    match result {
        Ok(read) => Some(read),
        Err(why) => {
            eprintln!("kelpie: ticket {ticket} was not read, {what} failed: {why}");
            None
        }
    }
}

impl Review {
    /// The review pass of a project, with what `review.json` keeps of each ticket
    ///
    /// # Errors
    ///
    /// `review.json` cannot be read, or is not the review's facts.
    pub fn new(parts: Parts) -> Result<Self, String> {
        let file = parts.paths.project().join(review::FILE);
        let book = Book::load(file).map_err(|error| error.to_string())?;
        Ok(Self {
            project_id: parts.project_id,
            folder: parts.folder,
            paths: parts.paths,
            tree: parts.tree,
            records: parts.records,
            tickets: parts.tickets,
            config: parts.config,
            clock: parts.clock,
            fix: parts.fix,
            make: parts.make,
            testing: parts.testing,
            book: Mutex::new(book),
        })
    }

    /// What the pass says of `head`, which the gate steps on
    ///
    /// `changed` is the files the pull request's diff changes, which each reviewer's
    /// `paths` are matched against. A head the pass still owes a read to, and which no fix
    /// turn of the pass pushed, goes to the pass before the merge yes.
    #[must_use]
    pub fn looked(&self, ticket: &str, head: &str, changed: &[String]) -> Reviewed {
        let book = lock(&self.book);
        let kept = ticket
            .parse::<TicketId>()
            .ok()
            .and_then(|id| book.get(id).cloned());
        drop(book);
        let kept = kept.as_ref();
        let pass = kept
            .and_then(|kept| kept.review.clone())
            .unwrap_or_else(Pass::first);
        let passed_over: &[String] = kept.map_or(&[], |kept| &kept.passed_over);
        let heads = kept.map_or_else(review::Heads::default, |kept| kept.heads.clone());
        let had = reads_had(&pass, passed_over);
        let Some(standing) = self.standing(ticket) else {
            // The reviewers cannot be read at all, which is no proof the pass has had its
            // reads. The gate holds the ticket off the merge yes rather than saying
            // "Ready to merge" of a head no reviewer read
            return Reviewed {
                vouches: heads.vouches_for(head),
                can_run: true,
                read: had + 1,
                of: RULE_READS,
            };
        };
        // A round that has started, or a turn the pass is waiting on, has a step to run
        // whatever the lineup still owes: settling the worker's fix turn is a step, and a
        // pass that skipped it would give the yes on a round that never finished.
        let started = pass.reviewer.is_some() || !matches!(pass.stage, ReviewStage::Round);
        let owed = !review::pass_ended(&pass, passed_over, &standing.lineup, changed);
        let can_run = started || owed;
        Reviewed {
            // A head the pass pushed for is one it has read for, so it needs no new pass.
            // A round of the pass that is still at work on that head is stepped all the
            // same: its fix turn and its tests are the pass's own business, and a pass
            // left off mid-round would end at the merge yes with its findings unsent.
            vouches: heads.vouches_for(head) && !started,
            can_run,
            read: had + u32::from(can_run),
            of: RULE_READS,
        }
    }

    /// One step of `at`'s pass: what the engine's order says to do, carried out
    ///
    /// A step that leaves the pass waiting for the worker, a slot or the maintainer holds
    /// the ticket; one that finishes its part hands it back to the gate.
    pub fn step(&self, at: &At<'_>) -> Ran {
        let Some(standing) = self.standing(at.ticket) else {
            return Ran::Held;
        };
        let Some(look) = self.look(at) else {
            return Ran::Held;
        };
        let Ok(id) = at.ticket.parse::<TicketId>() else {
            return Ran::Held;
        };
        // The book is held only to read the ticket's facts and to write them back, never
        // across the step: a read is a model session that can run for minutes, and the book
        // is one per project, which every other ticket's pass reads to look at its own head.
        // One pass per ticket at a time is the gate's own rule (`gate::Gate::take`), so
        // nothing else is writing this ticket's facts while the step runs.
        let before = lock(&self.book).get(id).cloned().unwrap_or_default();
        let mut kept = before.clone();
        let step = review::step(&look, &mut kept, &standing.lineup, self.clock.now());
        let ran = self.carry_out(at, &look, &mut kept, &standing, step);
        let kept = match ran {
            // The step never ran, so the pass is kept where it was and the next read of
            // GitHub runs the same step, rather than a ruling on a turn that was never sent
            Ran::Undone => before,
            _ => kept,
        };
        let mut book = lock(&self.book);
        *book.entry(id) = kept;
        if let Err(why) = book.save() {
            eprintln!(
                "kelpie: the review's facts of ticket {} did not keep: {why}",
                at.ticket
            );
        }
        ran
    }

    /// Records that a catch-up pushed `pushed` for a ticket whose head `head` the pass had
    /// read or vouched for, so the new head needs no new read
    pub fn vouched(&self, ticket: &str, head: &str, pushed: &str) {
        let Ok(id) = ticket.parse() else { return };
        let mut book = lock(&self.book);
        let Some(kept) = book.get(id) else { return };
        if !kept.heads.reviewed(head) && !kept.heads.vouches_for(head) {
            return;
        }
        book.entry(id).heads.vouch(pushed, false);
        if let Err(why) = book.save() {
            eprintln!("kelpie: the review's vouched head did not keep: {why}");
        }
    }

    /// What the pass keeps of `ticket`, which a test reads back
    #[cfg(test)]
    pub(crate) fn kept(&self, ticket: &str) -> Option<review::Kept> {
        let id = ticket.parse().ok()?;
        lock(&self.book).get(id).cloned()
    }

    /// The reviewers, the settings and the plan one step reads
    fn standing(&self, ticket: &str) -> Option<Standing> {
        let settings = read_of(
            ticket,
            "the project's settings",
            Settings::load(&self.config, &self.project_id),
        )?;
        let agents = read_of(
            ticket,
            "the agent files",
            Agents::load(&self.config).map_err(|error| error.to_string()),
        )?;
        let plan = read_of(
            ticket,
            "the worker plan",
            plan(&self.config, &self.project_id),
        )?;
        let lineup = Lineup::of(&settings.reviewers.value, &agents);
        let test_command = settings.test_command.value.clone();
        Some(Standing {
            lineup,
            settings,
            agents,
            test_command,
            plan,
        })
    }

    /// What the pass reads of git for one step, `None` when it cannot be read
    ///
    /// A step that cannot read waits for the next read, as a gate pass that cannot does.
    fn look(&self, at: &At<'_>) -> Option<review::Look> {
        let (tree, repo) = (&*self.tree, self.folder.as_path());
        let worktree = self.paths.worktree(at.ticket);
        let head = read_of(
            at.ticket,
            "its worktree's head",
            worktree::head(tree, repo, &worktree),
        )?;
        let left = read_of(
            at.ticket,
            "its worktree's status",
            worktree::uncommitted(tree, repo, &worktree),
        )?;
        let changed = read_of(
            at.ticket,
            "its changed files",
            worktree::changed::changed_files(tree, repo, at.head, crate::workers::BASE),
        )?;
        Some(review::Look {
            number: at.number,
            head: at.head.to_owned(),
            worktree: review::Checkout {
                head,
                clean: left.is_empty(),
            },
            changed,
            build: self.paths.build_dir(at.ticket),
        })
    }

    /// The project's records, while kelpie still holds them
    fn records(&self) -> Option<Arc<Mutex<Records>>> {
        self.records.upgrade()
    }
}
