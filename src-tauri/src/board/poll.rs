//! The poller: one read of GitHub a minute, one placing every two seconds
//!
//! Each read also hands over the drafts the project's finished turns still owe the
//! forge ([`Retry`]), which is the retry a worker parked on a failed open is told to
//! expect (`docs/design/tickets.md`, Use case).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use kelpie_engine::board::{Board, Read, Rules, place};
use kelpie_engine::github::{Clock, ForgeSlug, Timestamp};
use kelpie_engine::settings::PickUp;
use kelpie_engine::ticket::{ForgeError, GithubTickets, OpenPull};
use kelpie_engine::worker::{Record, Records};

use super::{BoardEvent, BoardView};

/// What a poller reads a project's board from: its tickets, and the three things only its source knows
pub trait Feed: GithubTickets {
    /// The repo the tickets are in
    fn repo(&self) -> &ForgeSlug;

    /// The login `gh` holds
    ///
    /// # Errors
    ///
    /// [`ForgeError`] when `gh` is missing, signed out or cannot be asked.
    fn viewer(&self) -> Result<String, ForgeError>;

    /// When the hold on a used-up rate limit lifts, `None` while none stands
    fn lifts_at(&self) -> Option<Timestamp>;
}

/// The project's choices a read depends on, read from its settings each time
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// Whose issues are picked up
    pub pick_up: PickUp,
    /// The label a Ready ticket carries
    pub ready_label: String,
    /// How many days a closed ticket stays in Done
    pub done_days: u32,
}

/// Where a read finds the project's choices
pub type Choose = Box<dyn Fn() -> Result<Choices, String> + Send>;

/// Where a placing finds the hosts each ticket's worker waits to reach
///
/// A sandbox request lives in its worker's feed and not in its record, so the
/// poller asks for the feeds' side each time it places.
pub type Requests = Box<dyn Fn() -> BTreeMap<String, Vec<String>> + Send>;

/// Where a read finds the workers that owe the forge a draft, and hands them over
///
/// A turn that ends hands its own draft over, on the thread its sink runs on
/// ([`crate::workers::hand_over`]). A worker whose draft could not be opened, or whose
/// file has since been rewritten, is handed over on the next read, which is what its
/// reason promises: "kelpie tries again each minute".
///
/// It is given the pull requests the read listed too, which is what the gate's pass reads
/// ([`crate::gate::Gate::after_read`]).
pub type Retry = Box<dyn Fn(&[Record], &[OpenPull]) + Send>;

/// Where a placing finds the tickets the gate found ready to merge
pub type Ready = Box<dyn Fn() -> BTreeSet<String> + Send>;

const DAY: u64 = 24 * 60 * 60;

/// What the window says when `gh` is missing or holds no login
const SIGN_IN: &str =
    "Kelpie reads GitHub through gh: install gh, then run `gh auth login` to sign in.";

/// How often the poller reads GitHub, and how often it places the last read
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cadence {
    /// Between two placings of the last read, which make no call
    pub place: Duration,
    /// Between two reads of GitHub
    pub read: Duration,
}

impl Cadence {
    /// A read a minute, and a placing every two seconds
    pub const LIVE: Self = Self {
        place: Duration::from_secs(2),
        read: Duration::from_secs(60),
    };
}

/// What the window asks a running poller to do
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Read GitHub now
    Refresh,
}

/// What the last read found, and the rules it was read under
struct Last {
    read: Read,
    rules: Rules,
}

/// Reads a project's board and places its tickets
pub struct Poller<F> {
    feed: Arc<F>,
    choose: Choose,
    records: Arc<Mutex<Records>>,
    requests: Requests,
    retry: Retry,
    ready: Ready,
    clock: Arc<dyn Clock>,
    last: Option<Last>,
    held: Option<Timestamp>,
    shown: Option<BoardEvent>,
}

impl<F> std::fmt::Debug for Poller<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Poller").finish_non_exhaustive()
    }
}

impl<F: Feed> Poller<F> {
    /// A poller of `feed`'s board, with the workers' `records`, the sandbox requests
    /// their feeds hold (`requests`) and the hand-overs owed to the forge (`retry`)
    pub fn new(
        feed: Arc<F>,
        choose: Choose,
        records: Arc<Mutex<Records>>,
        requests: Requests,
        retry: Retry,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            feed,
            choose,
            records,
            requests,
            retry,
            ready: Box::new(BTreeSet::new),
            clock,
            last: None,
            held: None,
            shown: None,
        }
    }

    /// Say which tickets are ready to merge, from `ready`, on each placing
    #[must_use]
    pub fn with_ready(mut self, ready: Ready) -> Self {
        self.ready = ready;
        self
    }

    /// Read GitHub, hand over for each worker that owes a draft, then place what it said
    ///
    /// `Some` only when what the window shows changes. A read that fails keeps
    /// the last board, and says so only when there is none.
    pub fn read(&mut self) -> Option<BoardEvent> {
        match self.fetch() {
            Ok(last) => {
                self.held = None;
                self.last = Some(last);
            }
            Err(error) => return self.failed(&error),
        }
        self.hand_over();
        self.place()
    }

    /// Hand over again for each worker that owes the forge a draft
    ///
    /// Only on a read, which is what just asked GitHub, and never on a placing, which
    /// asks nothing. The records are read under their lock and dropped before the first
    /// hand-over starts, so a start on another thread is not held behind a call to
    /// GitHub. A draft opened here shows on the board on the next read, which is the one
    /// that will have seen it.
    fn hand_over(&self) {
        let records: Vec<Record> = {
            let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
            records.list().to_vec()
        };
        let pulls = self
            .last
            .as_ref()
            .map_or(&[][..], |last| &last.read.pulls[..]);
        (self.retry)(&records, pulls);
    }

    fn fetch(&mut self) -> Result<Last, ForgeError> {
        // Asked on every read, so `gh auth switch` shows on the next minute
        let login = self.feed.viewer()?;
        let choices = (self.choose)().map_err(ForgeError::Failed)?;
        let rules = Rules {
            pick_up: choices.pick_up,
            ready_label: choices.ready_label,
            login,
        };
        let since = Timestamp::from_secs(
            self.clock
                .now()
                .secs()
                .saturating_sub(u64::from(choices.done_days) * DAY),
        );
        let read = Read {
            ready: self.feed.ready_issues(&rules.ready_label)?,
            open: self.feed.open_issues()?,
            closed: self.feed.closed_issues(since)?,
            pulls: self.feed.open_pulls()?,
        };
        Ok(Last { read, rules })
    }

    fn failed(&mut self, error: &ForgeError) -> Option<BoardEvent> {
        match error {
            ForgeError::Held(until) => self.held = Some(*until),
            ForgeError::Limited { .. } => self.held = self.feed.lifts_at(),
            _ if needs_gh(error) => {
                self.last = None;
                return self.show(BoardEvent::Unavailable {
                    message: SIGN_IN.to_owned(),
                });
            }
            _ if self.last.is_none() => {
                return self.show(BoardEvent::Unavailable {
                    message: error.to_string(),
                });
            }
            _ => return None,
        }
        self.place()
    }

    /// Place the last read on the workers' records now, with no call to GitHub
    ///
    /// `Some` only when the board differs from the one the window shows.
    pub fn place(&mut self) -> Option<BoardEvent> {
        let now = self.clock.now();
        let held = self.held.filter(|until| *until > now);
        // Asked before the records lock is held: the feeds' locks are never taken under it
        let requests = (self.requests)();
        let ready = (self.ready)();
        let records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        let board = match &self.last {
            Some(last) => place(&last.read, records.list(), &last.rules),
            // Held before any board was read: an empty one, with the time
            None if held.is_some() => Board::default(),
            None => return None,
        };
        let view = BoardView::of(
            &board,
            self.feed.repo(),
            held,
            records.list(),
            &requests,
            &ready,
        );
        drop(records);
        self.show(BoardEvent::Board(view))
    }

    fn show(&mut self, event: BoardEvent) -> Option<BoardEvent> {
        if self.shown.as_ref() == Some(&event) {
            return None;
        }
        self.shown = Some(event.clone());
        Some(event)
    }
}

// Whether `gh` is missing, holds no login, or holds one GitHub refuses
fn needs_gh(error: &ForgeError) -> bool {
    match error {
        ForgeError::Spawn(_) => true,
        ForgeError::Failed(line) => crate::github::says_no_login(line),
        _ => false,
    }
}

/// Poll until `send` fails or `commands` is dropped
///
/// Reads at once, then every [`Cadence::read`], and places the last read every
/// [`Cadence::place`]. Each event goes to `send`, which answers whether the
/// window still takes them: a `false` ends the poll, since the window is gone.
pub fn run<F: Feed>(
    poller: &mut Poller<F>,
    commands: &Receiver<Command>,
    cadence: Cadence,
    send: &mut dyn FnMut(BoardEvent) -> bool,
) {
    let mut next_read = Instant::now();
    loop {
        let event = if Instant::now() >= next_read {
            next_read = Instant::now() + cadence.read;
            poller.read()
        } else {
            poller.place()
        };
        if let Some(event) = event
            && !send(event)
        {
            return;
        }
        match commands.recv_timeout(cadence.place) {
            Ok(Command::Refresh) => next_read = Instant::now(),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
