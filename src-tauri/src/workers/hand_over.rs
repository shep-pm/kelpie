//! The hand-over the app runs: a finished turn's draft, and what its worker is told
//!
//! A worker pushes with a token that opens no pull request, and leaves the draft's
//! title and body in a file of its own. When its turn ends with the branch pushed,
//! kelpie opens the draft as the maintainer's own `gh` login, and the ticket is in
//! Review on the board's next read (`docs/design/tickets.md`, Use case, item 4).
//!
//! The sink [`Workers::start`] gives [`kelpie_engine::worker::start()`] is this
//! module's `Sink`: the worker's feed, then the hand-over. The hand-over runs on a
//! thread of its own, since the session's sink must not block: opening a draft waits
//! on git and on `gh`, and a sink that waits holds up the turn streaming into it.
//!
//! What the forge answers goes back into the worker's record
//! ([`kelpie_engine::worker::after_turn`], `docs/design/workers.md`, Pushing and the
//! pull request):
//!
//! - A draft opened, or an open one edited to the file, changes no record: the worker already ended.
//! - A refusal the worker can fix goes to it as a Note ([`kelpie_engine::worker::note`]), so it rewrites its file.
//! - The same refusal at the next hand-over is the worker leaving its file as it was, and is ruled on instead: it waits on the maintainer with the refusal as its reason, holds no slot, and is told no more. Without this, a refusal the worker never fixes costs a turn and a slot at every hand-over.
//! - A forge that could not be asked parks the worker as waiting on the maintainer with the reason, which the board marks as needing the maintainer and which holds no slot.
//!
//! A parked worker is handed over again on every read of GitHub ([`crate::board::poll`]),
//! which is the retry its reason promises, and a hand-over that then goes through sets
//! the worker Ended and clears the reason.
//!
//! A ruling is an answer to a turn, and the board's read is not one. A refusal the read
//! finds is told to the worker once, and a refusal its record already holds, as the text
//! it was told or as a Note waiting in its record, is told no twice and ruled on not at
//! all: the worker is ruled on at its next turn's end, which is the first hand-over that
//! knows what it made of the telling.
//!
//! Two hand-overs never work on one ticket at once (`Hands::take`). A turn's end and a
//! board read can both want the same draft, and a hand-over waits on git and `gh`: two
//! of them would read the open pull requests, find none, and each open a draft. And a
//! settle only writes the record it began from (`Standing`): a hand-over that finds the
//! maintainer has started its worker or sent it a Note while it waited writes nothing,
//! since that record is a live turn's.
//!
//! The draft is opened as the maintainer's login, never as the App, and the login never
//! reaches the worker (`docs/design/tickets.md`, GitHub).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::thread;

use kelpie_engine::ci::Ci;
use kelpie_engine::github::AppTokens;
use kelpie_engine::lease::Leases;
use kelpie_engine::login::Store;
use kelpie_engine::session::Session;
use kelpie_engine::settings::ConfigDir;
use kelpie_engine::ticket::{ForgeError, GithubTickets, Pulls};
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::usage::InFlight;
use kelpie_engine::worker::{self, Opening, Paths, Record, Records, Status};
use kelpie_protocol::SessionEvent;

use super::{BASE, Feed, OpenOn, SessionMap, WorkerPlan, Workers, lock, open};

/// The start of the reason a worker waits on the maintainer for its draft
///
/// The board's read hands over again for every worker waiting with this reason
/// ([`crate::board::poll`]), so a parked worker is its retry's own subject.
const OWES: &str = "Could not open the draft pull request";

/// What kelpie then does for a worker parked on a draft it could not open
const EVERY_MINUTE: &str = "kelpie tries again each minute";

/// The start of the reason a worker waits on the maintainer for a refusal it was told
///
/// A worker waiting on its draft is handed over again on every read of GitHub, and a
/// worker waiting on this is not ([`OWES`]): what is owed here is a file the worker
/// will not fix, which is the maintainer's to rule on.
const TOLD_ONCE: &str =
    "Kelpie told the worker to fix its hand-over file, and the same refusal stands";

/// Why a hand-over could not open the draft, as the worker's record keeps it
fn reason(why: &ForgeError) -> String {
    format!("{OWES}: {why}, and {EVERY_MINUTE}")
}

/// Why a worker waits on the maintainer for the refusal it was told and left as it was
fn ruling(text: &str) -> String {
    format!("{TOLD_ONCE}: {text}. kelpie tells it no more until the file changes")
}

/// Say what a record that would not be written cost the ticket
///
/// The turn goes on either way, so the loss is named rather than swallowed: the worker
/// is not parked on the maintainer, or is parked with no reason, and the board shows it
/// as it was.
fn lost(ticket: &str, kept: std::io::Result<()>, what: &str) {
    if let Err(why) = kept {
        eprintln!("kelpie: the record of ticket {ticket} did not keep {what}: {why}");
    }
}

/// What a hand-over asks of a project's forge: which pull requests are open, and
/// opening or editing one
///
/// kelpie builds it from the project's ticket source and its draft source, both called
/// through one hold ([`Workers::source`]), so a project's reads and its drafts are
/// paced as one set. A test stands one in with `FakeTickets` and `FakePulls`, which
/// kelpie-engine ships under its `testing` feature, through [`Workers::stand_in_forge`].
pub struct Forge {
    /// The project's tickets, which know which pull requests are open
    tickets: Arc<dyn GithubTickets>,

    /// The drafts of the project's repo, opened as the maintainer
    pulls: Arc<dyn Pulls>,

    /// The project's CI, which the gate reruns a stuck run through
    ci: Arc<dyn Ci>,
}

impl Forge {
    /// What a project's hand-overs and its gate ask of `tickets`, `pulls` and `ci`
    #[must_use]
    pub fn new(tickets: Arc<dyn GithubTickets>, pulls: Arc<dyn Pulls>, ci: Arc<dyn Ci>) -> Self {
        Self { tickets, pulls, ci }
    }

    /// The project's tickets
    pub(crate) fn tickets(&self) -> &Arc<dyn GithubTickets> {
        &self.tickets
    }

    /// The project's pull requests
    pub(crate) fn pulls(&self) -> &Arc<dyn Pulls> {
        &self.pulls
    }

    /// The project's CI
    pub(crate) fn ci(&self) -> &Arc<dyn Ci> {
        &self.ci
    }
}

// The sources are trait objects, which no derive can print
impl std::fmt::Debug for Forge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Forge").finish_non_exhaustive()
    }
}

/// The sink [`Workers::start`] gives [`kelpie_engine::worker::start()`]: the worker's
/// feed, then its hand-over
#[derive(Debug)]
pub(super) struct Sink {
    /// The channels watching the worker
    pub(super) feed: Arc<Feed>,

    /// The ticket whose turns this sink hears
    pub(super) ticket: String,

    /// What the project's finished turns hand over
    pub(super) hands: Hands,
}

impl Sink {
    /// Forward `event` to the worker's feed, and hand over the turn when it ends
    ///
    /// The engine settles the record before it sends the turn's end here, so a record
    /// that says Ended is one whose branch left it clean.
    pub(super) fn hear(&mut self, event: SessionEvent) {
        let ended = matches!(event, SessionEvent::TurnEnded { .. });
        self.feed.forward(event);
        if ended {
            self.hands.turn_ended(&self.ticket);
        }
    }
}

/// The tickets of a project whose hand-over is running, by ticket
///
/// A turn's end and a board read can both want one ticket's draft, and a hand-over
/// waits on git and `gh`. Two of them read the open pull requests, find none, and each
/// open a draft, which GitHub answers with a second draft or a 422 that parks the
/// worker on a draft that is already open. One hand-over at a time per ticket rules
/// that out.
///
/// One per project, held by [`Workers`], so every [`Hands`] of it shares the same set.
pub(super) type Busy = Arc<Mutex<BTreeSet<String>>>;

/// The ticket a hand-over is at work on, given back when it ends
#[derive(Debug)]
struct Working {
    busy: Busy,
    ticket: String,
}

impl Drop for Working {
    fn drop(&mut self) {
        lock(&self.busy).remove(&self.ticket);
    }
}

/// What ran a hand-over, which is what its answer may be
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trigger {
    /// The worker's turn ended, which is the only hand-over that knows what the worker
    /// made of what it was last told
    TurnEnded,

    /// The board's read of GitHub, which retries a draft the forge could not take
    Read,
}

/// What `ticket`'s record said when its hand-over began
///
/// A hand-over waits on git and `gh`, and while it waits the maintainer can start the
/// worker or send it a Note. A settle writes the record it began from and no other: a
/// live turn's record is settled by that turn's end, and writing over it would show a
/// running worker as waiting on the maintainer, holding no slot, for a turn that has
/// already ended.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Standing {
    status: Status,
    reason: Option<String>,
}

impl Standing {
    /// The record it was read from
    fn of(record: &Record) -> Self {
        Self {
            status: record.status,
            reason: record.reason.clone(),
        }
    }

    /// Whether `record` is still what the hand-over began from
    fn holds(&self, record: &Record) -> bool {
        record.status == self.status && record.reason == self.reason
    }
}

/// What a project's finished turns hand over, and the retry its board read runs
///
/// It holds what a start holds, so a hand-over can tell its worker from a thread of its
/// own, long after the start that began the turn has returned.
#[derive(Debug, Clone)]
pub struct Hands {
    /// The project these hand-overs belong to
    project_id: String,

    /// kelpie's data folder, which holds the installed bridges an opener runs on
    data_dir: PathBuf,

    /// The folder settings are read from
    config: ConfigDir,

    /// Where the person's harness logins are kept
    logins: Store,

    /// Where the project's workers live
    paths: Paths,

    /// The project's ledger, which a note's turn writes its line to
    ///
    /// The same handle the project's starts meter into, so a turn a hand-over tells
    /// the worker to run is metered, and written, like any other.
    usage: InFlight,

    /// The project's checkout, which holds the remote each branch was cut from
    folder: PathBuf,

    /// The tree kelpie's own git reads go through (ADR 0001)
    tree: Arc<ProcessTree>,

    /// The project's worker records, held weakly
    ///
    /// `Records` keeps where each ticket's turns send their events, which is this
    /// module's [`Sink`], which holds these `Hands`. A strong record here is a ring
    /// nothing could ever drop, which would keep each worker's session, its harness
    /// and its log alive after kelpie closes.
    records: Weak<Mutex<Records>>,

    /// What its hand-overs ask of GitHub, `None` where the project has no forge
    forge: Option<Arc<Forge>>,

    /// What its workers run on, which a note is sent on
    plan: WorkerPlan,

    /// kelpie's GitHub App tokens, which mint what a worker pushes with
    tokens: Arc<AppTokens>,

    /// The `gh` on the machine's `PATH`
    gh: Option<PathBuf>,

    /// kelpie's leases, whose socket a worker's session holds
    leases: Arc<Leases>,

    /// The sessions of the workers kelpie keeps, by project and ticket
    ///
    /// Held weakly for the same reason as [`Hands::records`]: a turn's sink reaches
    /// the session it is running on, and this module's sink reaches these sessions.
    sessions: Weak<SessionMap>,

    /// The tickets of this project with a hand-over running, shared by every `Hands` of it
    busy: Busy,
}

impl Hands {
    /// Hand over `ticket`'s turn, when the engine settled its record as done
    ///
    /// A turn that ended held, cancelled, or with its worker still running asks nothing
    /// of the forge. The hand-over runs git and `gh`, so it goes onto a thread of its
    /// own: this one is the session's, and a sink that waits holds up the turn.
    fn turn_ended(&self, ticket: &str) {
        if !self.settled(ticket) {
            return;
        }
        let hands = self.clone();
        let ticket = ticket.to_owned();
        thread::spawn(move || hands.hand_over(&ticket, Trigger::TurnEnded));
    }

    /// Hand over again for every worker of the project that owes the forge a draft
    ///
    /// The board's read runs this on each read of GitHub, which is the retry a parked
    /// worker's reason promises. A worker owes one when it is Ended, or is waiting on
    /// the maintainer for its draft, and left a file the forge has not been given, so
    /// once the draft is open and holds the file nothing here asks the forge again.
    pub fn retry(&self, records: &[Record]) {
        for record in records {
            if !self.owes(record) {
                continue;
            }
            self.hand_over(&record.ticket, Trigger::Read);
        }
    }

    /// Hand `ticket`'s finished turn over to the forge, and settle its record by what
    /// the forge answered
    ///
    /// One hand-over works on a ticket at a time ([`Hands::take`]), and what it may
    /// settle is the record it began from ([`Standing`]), read before the git and `gh`
    /// this waits on.
    fn hand_over(&self, ticket: &str, trigger: Trigger) {
        let Some(forge) = &self.forge else {
            eprintln!(
                "kelpie: project {} has no GitHub forge, so ticket {ticket} hands over nothing",
                self.project_id
            );
            return;
        };
        let Some(_at_work) = self.take(ticket) else {
            // A hand-over is already at work on this ticket, and it will settle the
            // record. A second one would ask the forge for the same draft twice.
            return;
        };
        let Some(standing) = self.standing(ticket) else {
            return;
        };
        let ctx = worker::HandOver {
            paths: &self.paths,
            ticket,
            tree: &self.tree,
            folder: &self.folder,
            base: BASE,
            pulls: forge.pulls.as_ref(),
            tickets: forge.tickets.as_ref(),
        };
        self.settle(ticket, trigger, &standing, worker::after_turn(&ctx));
    }

    /// Take `ticket` for a hand-over, `None` when one is already at work on it
    ///
    /// The guard hands the ticket back when the hand-over ends, however it ends.
    fn take(&self, ticket: &str) -> Option<Working> {
        let busy = Arc::clone(&self.busy);
        if !lock(&busy).insert(ticket.to_owned()) {
            return None;
        }
        Some(Working {
            busy,
            ticket: ticket.to_owned(),
        })
    }

    /// What `ticket`'s record says now, which is the only one this hand-over may settle
    ///
    /// Read before the git and `gh` it waits on, so a settle can tell whether the
    /// maintainer moved the worker out from under it ([`Standing`]).
    fn standing(&self, ticket: &str) -> Option<Standing> {
        let records = self.kept()?;
        let record = lock(&records).get(ticket).cloned()?;
        Some(Standing::of(&record))
    }

    /// Settle `ticket`'s record by what the forge made of its hand-over
    ///
    /// `trigger` is what ran it, which is what the answer may be ([`Trigger`]), and
    /// `standing` is the record it began from, which is the only one it may write
    /// ([`Standing`]).
    fn settle(
        &self,
        ticket: &str,
        trigger: Trigger,
        standing: &Standing,
        outcome: worker::Outcome,
    ) {
        match outcome {
            worker::Outcome::Told(text) => self.answer(ticket, &text, trigger, standing),
            worker::Outcome::Failed(why) => self.park(ticket, &why, standing),
            worker::Outcome::Opened(..)
            | worker::Outcome::Edited(..)
            | worker::Outcome::Unchanged => self.cleared(ticket, standing),
            // A file the worker took away leaves nothing owed of it, and the refusal
            // kelpie remembers for it goes with it
            worker::Outcome::NoFile => self.forget(ticket),
            // A branch the forge does not have asks nothing of the record: the worker
            // is where it was
            _ => {}
        }
    }

    /// Whether `record`'s worker owes the forge a draft
    fn owes(&self, record: &Record) -> bool {
        let waiting = record.status == Status::WaitingOnMaintainer
            && record
                .reason
                .as_deref()
                .is_some_and(|why| why.starts_with(OWES));
        (record.status == Status::Ended || waiting)
            && worker::owes_a_draft(&self.paths, &record.ticket)
    }

    /// The project's records, while kelpie still holds them
    ///
    /// `None` once the app is closing and nothing else keeps them, which is when a
    /// hand-over in flight has nothing left to settle.
    fn kept(&self) -> Option<Arc<Mutex<Records>>> {
        self.records.upgrade()
    }

    /// The project's records, when `ticket`'s still says what it did at `standing`
    ///
    /// A hand-over waits on git and `gh`, and while it waits the maintainer can start
    /// its worker or send it a Note. A settle that reaches such a record writes nothing
    /// and says so: a live turn's record is settled by that turn's end, and a hand-over
    /// that parked a running worker would show it waiting on the maintainer, holding no
    /// slot, for a turn that ended before the maintainer moved it.
    fn kept_as(&self, ticket: &str, standing: &Standing) -> Option<Arc<Mutex<Records>>> {
        let records = self.kept()?;
        let moved = {
            let held = lock(&records);
            held.get(ticket)
                .is_none_or(|record| !standing.holds(record))
        };
        if moved {
            eprintln!(
                "kelpie: ticket {ticket} is not where its hand-over left it, so its hand-over settles nothing",
            );
            return None;
        }
        Some(records)
    }

    /// Whether the engine settled `ticket`'s record as done, which is what a turn that
    /// ended with its branch clean leaves behind
    fn settled(&self, ticket: &str) -> bool {
        let Some(records) = self.kept() else {
            return false;
        };
        lock(&records)
            .get(ticket)
            .is_some_and(|record| record.status == Status::Ended)
    }

    /// Answer the refusal `ticket`'s file drew: tell the worker, or rule on it when it was told already
    ///
    /// The first refusal goes to the worker as a Note, which is its next turn, and its
    /// text is kept in the record. A hand-over that answers with the same text is the
    /// worker leaving its file as it was, and telling it again would start another turn,
    /// and another, each one taking a slot for the same file. What kelpie will not fix
    /// goes to the maintainer instead ([`Hands::rule`]).
    ///
    /// The text is kept before the worker is told, so a turn that ends fast is answered
    /// by the ruling and not by a second telling.
    ///
    /// Only a turn's end is asked what the worker made of the telling ([`Trigger`]). A
    /// board read that finds the refusal the record already holds rules on nothing: the
    /// worker has had no turn since it was told, and its note may still be waiting in
    /// its record, so "the same refusal stands" would be said of a worker that never
    /// heard it.
    fn answer(&self, ticket: &str, text: &str, trigger: Trigger, standing: &Standing) {
        let Some(records) = self.kept() else {
            return;
        };
        let told = lock(&records)
            .get(ticket)
            .and_then(|record| record.refused.as_deref())
            .is_some_and(|before| before == text);
        if told {
            if trigger == Trigger::TurnEnded {
                self.rule(ticket, text, standing);
            }
            return;
        }
        let written = lock(&records).set_refused(ticket, Some(text.to_owned()));
        lost(ticket, written, "the refusal it was told");
        self.tell(ticket, text);
    }

    /// Tell `ticket`'s worker `text` as a Note, so it can fix the file it left
    ///
    /// A worker whose turn ended takes the note as its next turn. One whose session is
    /// gone, or which has no free slot, keeps it in its record for the turn its next
    /// start begins ([`kelpie_engine::worker::note`]).
    fn tell(&self, ticket: &str, text: &str) {
        let Some(records) = self.kept() else {
            return;
        };
        let Some(session) = self.held(ticket) else {
            // The worker's session went with the launch that started it, so the note
            // waits in its record, which is what a note does for a worker away from its
            // turns.
            if let Err(why) = lock(&records).add_note(ticket, text) {
                eprintln!("kelpie: ticket {ticket} did not keep the note: {why}");
            }
            return;
        };
        let on = OpenOn {
            data_dir: &self.data_dir,
            logins: &self.logins,
            config: &self.config,
            project_id: &self.project_id,
            folder: &self.folder,
            plan: &self.plan,
        };
        // `note` starts no session of its own: it sends to the worker's, which already
        // has its tools, its credential and its lease socket. The opener is here because
        // a project names one, and `note` never calls it.
        let opener = |opening: &Opening<'_>| open::open(&on, opening);
        let project = worker::Project {
            paths: &self.paths,
            folder: &self.folder,
            base: BASE,
            tree: &self.tree,
            slots: self.plan.slots,
            turn_timeout: self.plan.turn_timeout,
            records: &records,
            binding: &self.plan.binding,
            network_allowlist: &self.plan.network_allowlist,
            reads: self.plan.reads,
            tools: None,
            tokens: Some(&*self.tokens),
            gh: self.gh.as_deref(),
            leases: Some(&*self.leases),
            usage: Some(&self.usage),
            open: &opener,
        };
        if let Err(why) = worker::note(&project, &session, ticket, text) {
            eprintln!("kelpie: ticket {ticket} was not told about its draft: {why}");
        }
    }

    /// Park `ticket`'s worker on the maintainer with the reason, which holds no slot
    ///
    /// The reason goes with the status under one hold of the records' lock, as the
    /// engine's own settling does. A record the maintainer moved while this hand-over
    /// was at work is left as they left it ([`Hands::kept_as`]).
    fn park(&self, ticket: &str, why: &ForgeError, standing: &Standing) {
        let Some(records) = self.kept_as(ticket, standing) else {
            return;
        };
        let mut records = lock(&records);
        let written = records
            .set_status(ticket, Status::WaitingOnMaintainer)
            .and_then(|()| records.set_reason(ticket, Some(reason(why))));
        lost(ticket, written, "its wait for the draft");
    }

    /// Park `ticket`'s worker on the refusal it was told and left as it was
    ///
    /// The reason names the refusal, which the board shows on the ticket's needs-you
    /// line, and no slot is held. Nothing hands it over again by itself: the board's
    /// read retries a worker waiting on its draft ([`OWES`]), not one waiting on a
    /// ruling, so the next hand-over is the next turn's end, which the maintainer
    /// starts. A file kelpie can then open clears the ruling ([`Hands::cleared`]).
    fn rule(&self, ticket: &str, text: &str, standing: &Standing) {
        let Some(records) = self.kept_as(ticket, standing) else {
            return;
        };
        let mut records = lock(&records);
        let written = records
            .set_status(ticket, Status::WaitingOnMaintainer)
            .and_then(|()| records.set_reason(ticket, Some(ruling(text))));
        lost(ticket, written, "its ruling on the refusal");
    }

    /// Set `ticket`'s worker Ended and clear the reason, when a hand-over that failed
    /// before has now gone through, and forget the refusal its file drew
    ///
    /// A worker held for something else keeps its reason: only the one this module wrote
    /// is cleared. The refusal is forgotten whatever the worker was waiting on, since a
    /// file the forge has taken is one its worker was told to fix for nothing.
    fn cleared(&self, ticket: &str, standing: &Standing) {
        let Some(records) = self.kept() else {
            return;
        };
        let refusal = lock(&records).set_refused(ticket, None);
        lost(ticket, refusal, "its refusal going away");
        let owed_a_draft = standing.status == Status::WaitingOnMaintainer
            && standing
                .reason
                .as_deref()
                .is_some_and(|why| why.starts_with(OWES));
        if !owed_a_draft {
            return;
        }
        let Some(records) = self.kept_as(ticket, standing) else {
            return;
        };
        let mut records = lock(&records);
        let written = records
            .set_status(ticket, Status::Ended)
            .and_then(|()| records.set_reason(ticket, None));
        lost(ticket, written, "its draft going through");
    }

    /// Forget the refusal `ticket`'s worker was told, when its hand-over file is gone
    ///
    /// The worker took away the file it was refused for. A refusal kept for a file that
    /// is no longer there would rule on the next one the worker writes, before it has
    /// been told anything about it.
    fn forget(&self, ticket: &str) {
        let Some(records) = self.kept() else {
            return;
        };
        let written = lock(&records).set_refused(ticket, None);
        lost(ticket, written, "its refusal going away");
    }

    /// The session of the worker on `ticket`, while kelpie keeps one
    ///
    /// A worker whose turn ended holds no slot, so this never goes through
    /// [`Workers::session`], which gives up the sessions of workers that hold none.
    fn held(&self, ticket: &str) -> Option<Arc<Session>> {
        let sessions = self.sessions.upgrade()?;
        lock(&sessions)
            .get(&(self.project_id.clone(), ticket.to_owned()))
            .cloned()
    }
}

impl Workers {
    /// The forge of the project `project_id`, whose checkout is `folder`, made on the first ask
    ///
    /// Its drafts are called through the same hold as the project's reads
    /// ([`Workers::source`]).
    ///
    /// # Errors
    ///
    /// The checkout's origin is not a GitHub repo, or there is no `gh` to ask.
    pub(super) fn forge(&self, project_id: &str, folder: &Path) -> Result<Arc<Forge>, String> {
        {
            let kept = lock(&self.forges);
            if let Some(forge) = kept.get(project_id) {
                return Ok(Arc::clone(forge));
            }
        }
        let calls = self.calls(project_id, folder)?;
        let forge = Arc::new(Forge::new(
            Arc::clone(&calls.source) as Arc<dyn GithubTickets>,
            Arc::clone(&calls.pulls) as Arc<dyn Pulls>,
            Arc::clone(&calls.ci) as Arc<dyn Ci>,
        ));
        lock(&self.forges).insert(project_id.to_owned(), Arc::clone(&forge));
        Ok(forge)
    }

    /// Ask the hand-overs of the project `project_id` of `forge`, in place of the `gh` on this machine
    ///
    /// kelpie builds a project's forge from its origin and its `gh`, which a test has
    /// neither of. A test stands the forge in here, and every hand-over of the project
    /// then goes through it.
    pub fn stand_in_forge(&self, project_id: &str, forge: Arc<Forge>) {
        lock(&self.forges).insert(project_id.to_owned(), forge);
    }

    /// The tickets of `project_id` with a hand-over at work
    ///
    /// One set per project, so the hand-over a turn's end started and the one its board
    /// read starts are the same hand-over as far as a ticket is concerned
    /// ([`Hands::take`]).
    fn busy(&self, project_id: &str) -> Busy {
        let mut kept = lock(&self.handovers);
        kept.entry(project_id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(BTreeSet::new())))
            .clone()
    }

    /// What the finished turns of the project `project_id`, whose checkout is `folder`, hand over
    ///
    /// `plan` is what its workers run on, which a note is sent on. A project with no
    /// forge to open a draft through hands over nothing, and says so when a turn ends.
    ///
    /// # Errors
    ///
    /// The project's records cannot be read (see [`Workers::records`]).
    pub fn hands(
        &self,
        project_id: &str,
        folder: &Path,
        plan: &WorkerPlan,
    ) -> Result<Hands, String> {
        // A project with no forge to open a draft through hands over nothing, and
        // [`Hands::hand_over`] says so when a turn ends.
        let forge = self.forge(project_id, folder).ok();
        let records = self.records(project_id)?;
        let paths = Paths::new(&self.data_dir, project_id);
        Ok(Hands {
            project_id: project_id.to_owned(),
            data_dir: self.data_dir.clone(),
            config: self.config.clone(),
            logins: self.logins.clone(),
            usage: self.ledger(&paths, project_id),
            paths,
            folder: folder.to_owned(),
            tree: Arc::clone(&self.tree),
            records: Arc::downgrade(&records),
            forge,
            plan: plan.clone(),
            tokens: Arc::clone(&self.tokens),
            gh: self.gh.clone(),
            leases: Arc::clone(self.leases(project_id)?),
            sessions: Arc::downgrade(&self.sessions),
            busy: self.busy(project_id),
        })
    }
}
