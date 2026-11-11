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
//! - A forge that could not be asked parks the worker as waiting on the maintainer with the reason, which the board marks as needing the maintainer and which holds no slot.
//!
//! A parked worker is handed over again on every read of GitHub ([`crate::board::poll`]),
//! which is the retry its reason promises, and a hand-over that then goes through sets
//! the worker Ended and clears the reason.
//!
//! The draft is opened as the maintainer's login, never as the App, and the login never
//! reaches the worker (`docs/design/tickets.md`, GitHub).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::thread;

use kelpie_engine::github::AppTokens;
use kelpie_engine::lease::Leases;
use kelpie_engine::login::Store;
use kelpie_engine::session::Session;
use kelpie_engine::settings::ConfigDir;
use kelpie_engine::ticket::{ForgeError, GithubTickets, Pulls};
use kelpie_engine::tree::ProcessTree;
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

/// Why a hand-over could not open the draft, as the worker's record keeps it
fn reason(why: &ForgeError) -> String {
    format!("{OWES}: {why}, and {EVERY_MINUTE}")
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
}

impl Forge {
    /// What a project's hand-overs ask of `tickets` and `pulls`
    #[must_use]
    pub fn new(tickets: Arc<dyn GithubTickets>, pulls: Arc<dyn Pulls>) -> Self {
        Self { tickets, pulls }
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
        thread::spawn(move || hands.hand_over(&ticket));
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
            self.hand_over(&record.ticket);
        }
    }

    /// Hand `ticket`'s finished turn over to the forge, and settle its record by what
    /// the forge answered
    fn hand_over(&self, ticket: &str) {
        let Some(forge) = &self.forge else {
            eprintln!(
                "kelpie: project {} has no GitHub forge, so ticket {ticket} hands over nothing",
                self.project_id
            );
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
        self.settle(ticket, worker::after_turn(&ctx));
    }

    /// Settle `ticket`'s record by what the forge made of its hand-over
    fn settle(&self, ticket: &str, outcome: worker::Outcome) {
        match outcome {
            worker::Outcome::Told(text) => self.tell(ticket, &text),
            worker::Outcome::Failed(why) => self.park(ticket, &why),
            worker::Outcome::Opened(..)
            | worker::Outcome::Edited(..)
            | worker::Outcome::Unchanged => self.cleared(ticket),
            // A turn that left no file, or one whose branch is not on the forge, asks
            // nothing of the record: the worker is where it was
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
            open: &opener,
        };
        if let Err(why) = worker::note(&project, &session, ticket, text) {
            eprintln!("kelpie: ticket {ticket} was not told about its draft: {why}");
        }
    }

    /// Park `ticket`'s worker on the maintainer with the reason, which holds no slot
    ///
    /// The reason goes with the status under one hold of the records' lock, as the
    /// engine's own settling does.
    fn park(&self, ticket: &str, why: &ForgeError) {
        let Some(records) = self.kept() else {
            return;
        };
        let mut records = lock(&records);
        let written = records
            .set_status(ticket, Status::WaitingOnMaintainer)
            .and_then(|()| records.set_reason(ticket, Some(reason(why))));
        lost(ticket, written, "its wait for the draft");
    }

    /// Set `ticket`'s worker Ended and clear the reason, when a hand-over that failed
    /// before has now gone through
    ///
    /// A worker held for something else keeps its reason: only the one this module wrote
    /// is cleared.
    fn cleared(&self, ticket: &str) {
        let Some(records) = self.kept() else {
            return;
        };
        let parked = lock(&records).get(ticket).is_some_and(|record| {
            record.status == Status::WaitingOnMaintainer
                && record
                    .reason
                    .as_deref()
                    .is_some_and(|why| why.starts_with(OWES))
        });
        if !parked {
            return;
        }
        let mut records = lock(&records);
        let written = records
            .set_status(ticket, Status::Ended)
            .and_then(|()| records.set_reason(ticket, None));
        lost(ticket, written, "its draft going through");
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
    fn forge(&self, project_id: &str, folder: &Path) -> Result<Arc<Forge>, String> {
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
        Ok(Hands {
            project_id: project_id.to_owned(),
            data_dir: self.data_dir.clone(),
            config: self.config.clone(),
            logins: self.logins.clone(),
            paths: Paths::new(&self.data_dir, project_id),
            folder: folder.to_owned(),
            tree: Arc::clone(&self.tree),
            records: Arc::downgrade(&records),
            forge,
            plan: plan.clone(),
            tokens: Arc::clone(&self.tokens),
            gh: self.gh.clone(),
            leases: Arc::clone(self.leases(project_id)?),
            sessions: Arc::downgrade(&self.sessions),
        })
    }
}
