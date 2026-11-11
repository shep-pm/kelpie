//! The sink that reads a worker's turn into its record, and the turns that follow it

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::session::{Session, SessionEvent};
use crate::tree::ProcessTree;

use super::ceiling::{Ceiling, Verdict, Watch};
use super::start::{Project, lock};
use super::{
    Records, Seat, Status,
    turn_end::{self, Ending},
};

/// The caller's sink, shared by every turn of a worker
///
/// A worker's later turns, the one a Note starts and the one Stop and tell
/// starts, reach the caller that started it through this.
#[derive(Clone)]
pub(super) struct Events(Arc<Mutex<Sink>>);

type Sink = Box<dyn FnMut(SessionEvent) + Send>;

impl Events {
    pub(super) fn new(sink: impl FnMut(SessionEvent) + Send + 'static) -> Self {
        Self(Arc::new(Mutex::new(Box::new(sink))))
    }

    fn hear(&self, event: SessionEvent) {
        (lock(&self.0))(event);
    }
}

// The sink is a closure, which no derive can print
impl fmt::Debug for Events {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Events").finish_non_exhaustive()
    }
}

/// What a worker's turns need to read themselves into its record and start the next
///
/// Cloned into each turn's sink, which holds the session by a weak reference: the
/// session's thread owns the sink, and must not keep the session alive.
#[derive(Clone)]
pub(super) struct Turns {
    records: Arc<Mutex<Records>>,
    ticket: String,
    tree: Arc<ProcessTree>,
    repo: PathBuf,
    worktree: PathBuf,
    base: String,
    slots: usize,
    timeout: Duration,
    session: Weak<Session>,
    events: Events,
}

impl Turns {
    /// The turns of `ticket`'s worker on `session`, whose events go to `events`
    ///
    /// `events` is kept in the records, so a later [`Turns::resume`] reaches the same
    /// caller.
    pub(super) fn begin(
        project: &Project<'_>,
        ticket: &str,
        session: &Arc<Session>,
        events: impl FnMut(SessionEvent) + Send + 'static,
    ) -> Self {
        let events = Events::new(events);
        lock(project.records).keep_events(ticket, events.clone());
        Self::on(project, ticket, session, events)
    }

    /// The turns of a worker that has begun, `None` when none of its turns ran this launch
    pub(super) fn resume(
        project: &Project<'_>,
        ticket: &str,
        session: &Arc<Session>,
        records: &Records,
    ) -> Option<Self> {
        let events = records.events_of(ticket)?;
        Some(Self::on(project, ticket, session, events))
    }

    fn on(project: &Project<'_>, ticket: &str, session: &Arc<Session>, events: Events) -> Self {
        Self {
            records: Arc::clone(project.records),
            ticket: ticket.to_owned(),
            tree: Arc::clone(project.tree),
            repo: project.folder.to_path_buf(),
            worktree: project.paths.worktree(ticket),
            base: project.base.to_owned(),
            slots: project.slots,
            timeout: project.turn_timeout,
            session: Arc::downgrade(session),
            events,
        }
    }

    /// Send `text` as the turn `watch` clocks, held to `ceiling`
    ///
    /// The caller kept `watch` in the records already, so nothing sees a worker
    /// that is running with no clock.
    pub(super) fn dispatch(
        &self,
        session: &Arc<Session>,
        text: String,
        watch: Watch,
        ceiling: Ceiling,
    ) {
        session.send(text, self.sink(watch.clone()));
        watch.hold_to(
            ceiling,
            Arc::clone(session),
            Arc::clone(&self.records),
            self.ticket.clone(),
            self.slots,
        );
    }

    /// The sink of one turn, which reads the record from every event that changes it
    ///
    /// Each change is kept before the event goes on, so a caller that hears a turn
    /// end finds the worker settled and holding no slot. A `Failed` that the session
    /// goes on after is followed by that turn's `TurnEnded`, which settles the
    /// record, unless the turn was cancelled at its ceiling or stopped: the worker
    /// is Paused then, and stays so. A turn Stop and tell cancelled settles
    /// nothing either, since the turn after it takes its place with the slot held.
    ///
    /// A turn's end goes through [`turn_end`] first: the branch is checked with the
    /// fence, and what it found settles the record, done for a clean branch and
    /// waiting on the maintainer for a held one. A clean branch whose record holds
    /// notes is not done: the notes go out together as the next prompt, one to a
    /// paragraph, and the worker stays Running with its slot. A held branch or a
    /// Paused worker keeps its notes for its next turn.
    fn sink(&self, watch: Watch) -> impl FnMut(SessionEvent) + Send + 'static {
        let turns = self.clone();
        move |event| turns.hear(&watch, event)
    }

    fn hear(&self, watch: &Watch, event: SessionEvent) {
        let ticket = self.ticket.as_str();
        // The fence runs git, so it answers before the records lock is held
        let ending = match &event {
            SessionEvent::TurnEnded { .. } => Some(turn_end::check(
                &self.tree,
                &self.repo,
                &self.worktree,
                &self.base,
            )),
            _ => None,
        };
        let mut records = lock(&self.records);
        match &event {
            SessionEvent::Started { session_id } => {
                lost(
                    ticket,
                    records.set_session(ticket, session_id),
                    "its session id",
                );
            }
            // The next turn restarts a harness that went while its turn was replaced
            SessionEvent::Failed { .. } if !watch.replaced() => {
                lost(
                    ticket,
                    records.set_status(ticket, Status::Paused),
                    "its pause",
                );
            }
            _ => {}
        }
        // A turn the ceiling cancelled stays as the ceiling left it: Paused
        let verdict = ending.as_ref().map(|_| watch.turn_ended());
        let settled = ending.as_ref().filter(|_| verdict == Some(Verdict::Ended));
        // A turn Stop and tell cancelled hands over to the redirect, whether it ended
        // or the harness failed under it
        let redirected = verdict == Some(Verdict::Replaced)
            || (matches!(event, SessionEvent::Failed { .. }) && watch.replaced());
        let next = if redirected {
            self.redirected(&mut records)
        } else {
            settled.and_then(|ending| self.continued(&mut records, ending))
        };
        if let Some(ending) = settled
            && next.is_none()
        {
            // The reason goes with the status under this lock hold, so a reader never
            // finds one without the other
            lost(
                ticket,
                records
                    .set_status(ticket, ending.status())
                    .and_then(|()| records.set_reason(ticket, ending.reason())),
                "its end",
            );
        }
        drop(records);
        self.events.hear(event);
        if let Some((session, notes, watch)) = next {
            self.dispatch(
                &session,
                notes,
                watch,
                Ceiling::from_grant(Instant::now(), self.timeout),
            );
        }
    }

    /// The turn that sends what Stop and tell said, when the cancelled turn's end finds any
    ///
    /// Its clock is kept under the lock the caller holds, so a Stop or another Stop and
    /// tell from here on finds the turn that runs. A worker whose session is gone keeps
    /// the text as a note for its next start.
    fn redirected(&self, records: &mut Records) -> Option<(Arc<Session>, String, Watch)> {
        let ticket = self.ticket.as_str();
        let text = records.take_redirect(ticket)?;
        let Some(session) = self.session.upgrade() else {
            lost(
                ticket,
                records.add_note(ticket, &text).map(drop),
                "what it was told",
            );
            return None;
        };
        let watch = Watch::new();
        records.keep_watch(ticket, watch.clone());
        Some((session, text, watch))
    }

    /// The turn that sends the record's notes, when the one that ended leaves any to send
    ///
    /// Its clock is kept with the notes cleared, under the lock the caller holds.
    fn continued(
        &self,
        records: &mut Records,
        ending: &Ending,
    ) -> Option<(Arc<Session>, String, Watch)> {
        let ticket = self.ticket.as_str();
        let running = records
            .get(ticket)
            .is_some_and(|record| record.status == Status::Running && record.seat == Seat::Held);
        if *ending != Ending::Done || !running {
            return None;
        }
        let session = self.session.upgrade()?;
        let notes = match records.take_notes(ticket) {
            Ok(notes) => notes,
            Err(error) => {
                lost(ticket, Err(error), "which notes it sent");
                return None;
            }
        };
        if notes.is_empty() {
            return None;
        }
        let watch = Watch::new();
        records.keep_watch(ticket, watch.clone());
        Some((session, notes.join("\n\n"), watch))
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
