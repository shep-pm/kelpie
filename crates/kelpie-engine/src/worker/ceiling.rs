//! A turn's ceiling: how long a worker's turn may hold its slot
//!
//! A stuck turn would hold its slot for good, so each turn has a ceiling, the
//! `worker-turn-timeout` setting, 60 minutes by default, as SK's
//! `worker.turn_timeout` has it (`docs/design/workers.md`, Slots, pacing and
//! leases). A turn past it is cancelled over ACP and the worker is Paused, so
//! the maintainer decides what comes next. The slot it held goes to the next
//! worker in line.
//!
//! The ceiling counts from when the slot was granted, not from the launch, as
//! SK's `Flight` does: the worktree and the harness take time to start, and a
//! worker holds its slot through all of it, so that time is spent.
//!
//! SK's `REVIEW_CEILING` bounds a wait for review, which comes with review in
//! Phase 3.

use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crate::session::Session;

use super::{Records, Status};

/// When a turn's time runs out
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ceiling {
    deadline: Instant,
}

impl Ceiling {
    /// The ceiling of a turn whose slot was granted at `granted`, `limit` long
    ///
    /// A limit too long for the clock to hold never runs out.
    #[must_use]
    pub fn from_grant(granted: Instant, limit: Duration) -> Self {
        Self {
            deadline: granted
                .checked_add(limit)
                .unwrap_or_else(|| granted + Duration::from_secs(u64::from(u32::MAX))),
        }
    }

    /// How long is left at `now`, nothing once the ceiling is past
    #[must_use]
    pub fn left(self, now: Instant) -> Duration {
        self.deadline.saturating_duration_since(now)
    }
}

/// Where a turn stands against its ceiling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// The turn runs and the clock counts
    Counting,

    /// The ceiling ran out and the turn was cancelled, or the maintainer stopped it
    Expired,

    /// The turn was cancelled so another could take its place
    Replaced,

    /// The turn ended
    Done,
}

/// What a turn's end tells the record
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    /// The turn ended by itself
    Ended,

    /// The turn ended because it was cancelled at its ceiling, or stopped
    Cancelled,

    /// The turn ended because Stop and tell cancelled it for the turn after
    Replaced,
}

/// One turn's clock, shared between the turn's events and the timer
#[derive(Debug, Clone)]
pub(super) struct Watch {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl Watch {
    /// A clock for a turn that has not begun
    pub(super) fn new() -> Self {
        Self {
            shared: Arc::new((Mutex::new(State::Counting), Condvar::new())),
        }
    }

    /// Say the turn ended, and whether the ceiling is why
    ///
    /// Stops the timer. The caller keeps the record a [`Verdict::Cancelled`]
    /// turn left as the timer set it: Paused.
    pub(super) fn turn_ended(&self) -> Verdict {
        let (state, changed) = &*self.shared;
        let mut state = lock(state);
        let verdict = match *state {
            State::Expired => Verdict::Cancelled,
            State::Replaced => Verdict::Replaced,
            State::Counting | State::Done => Verdict::Ended,
        };
        *state = State::Done;
        changed.notify_all();
        verdict
    }

    /// Retire the clock of a turn that is over or will never report its end
    ///
    /// Stops the timer, and leaves a turn already stopped or replaced as it is, so
    /// its end is still read as that.
    pub(super) fn retire(&self) {
        self.leave(State::Done);
    }

    /// Mark the turn stopped, `false` when it already ended
    ///
    /// Its end then leaves the record as the caller set it, as a ceiling's does.
    /// Call it with the records locked, so no end of the turn settles them between.
    pub(super) fn stop(&self) -> bool {
        self.leave(State::Expired)
    }

    /// Mark the turn replaced by the next one, `false` when it already ended
    ///
    /// Its end then settles nothing: the worker is still Running and holds its slot.
    pub(super) fn replace(&self) -> bool {
        self.leave(State::Replaced)
    }

    /// Whether the turn was replaced by the next one
    pub(super) fn replaced(&self) -> bool {
        *lock(&self.shared.0) == State::Replaced
    }

    fn leave(&self, to: State) -> bool {
        let (state, changed) = &*self.shared;
        let mut state = lock(state);
        if *state != State::Counting {
            return false;
        }
        *state = to;
        changed.notify_all();
        true
    }

    /// Hold the turn to `ceiling` from a thread of its own
    ///
    /// When the time runs out before the turn ends, the harness is asked to
    /// cancel, `ticket`'s worker is Paused in `records`, and the slots run again
    /// under `slots` so the slot it held goes to the next in line. Call it after
    /// the turn was sent, so the cancel lands on that turn.
    pub(super) fn hold_to(
        &self,
        ceiling: Ceiling,
        session: Arc<Session>,
        records: Arc<Mutex<Records>>,
        ticket: String,
        slots: usize,
    ) {
        let shared = Arc::clone(&self.shared);
        let spawned = thread::Builder::new()
            .name("kelpie-ceiling".to_owned())
            .spawn(move || {
                let (state, changed) = &*shared;
                let mut state = lock(state);
                loop {
                    if *state != State::Counting {
                        return;
                    }
                    let left = ceiling.left(Instant::now());
                    if left.is_zero() {
                        break;
                    }
                    state = changed
                        .wait_timeout(state, left)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                }
                drop(state);
                // The records first, as every turn event takes them, so a turn
                // that ended by itself just now is seen and left as it ended
                let mut records = lock(&records);
                {
                    let mut state = lock(&shared.0);
                    if *state != State::Counting {
                        return;
                    }
                    *state = State::Expired;
                }
                // Paused before the cancel, so the turn's end finds it so
                if let Err(error) = pause(&mut records, &ticket, slots) {
                    eprintln!(
                        "kelpie: the record of ticket {ticket} did not keep its pause: {error}"
                    );
                }
                drop(records);
                session.cancel();
            });
        if let Err(error) = spawned {
            eprintln!("kelpie: no ceiling holds this turn, its timer did not start: {error}");
        }
    }
}

/// Pause `ticket`'s worker, and give its slot to the next in line
fn pause(records: &mut Records, ticket: &str, slots: usize) -> io::Result<()> {
    records.set_status(ticket, Status::Paused)?;
    records.seat(slots)?;
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use kelpie_protocol::{SessionEvent, StopReason};

    use super::Ceiling;
    use crate::session::Session;
    use crate::session::fake::{FakeAgent, Turn};
    use crate::worker::tests::{Rig, binding, events, kept, on, ticket, until_turn_ends};
    use crate::worker::{Opening, Seat, Started, Status, start};

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn the_ceiling_counts_from_the_grant_not_the_launch() {
        let granted = Instant::now();
        let launched = granted + 10 * MINUTE;
        let ceiling = Ceiling::from_grant(granted, 60 * MINUTE);

        assert_eq!(
            ceiling.left(launched),
            50 * MINUTE,
            "the ten minutes before the launch were already spent"
        );
        assert_eq!(ceiling.left(granted + 60 * MINUTE), Duration::ZERO);
        assert_eq!(
            ceiling.left(granted + 90 * MINUTE),
            Duration::ZERO,
            "a turn past it has nothing left"
        );
    }

    #[test]
    fn a_turn_past_its_ceiling_is_cancelled_and_the_worker_paused() {
        let rig = Rig::new("ceiling");
        let binding = binding();
        let agent = FakeAgent::new().turn(Turn::new().text("stuck").until_cancelled());
        let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
        let (sink, heard) = events();
        let mut project = rig.project(&binding, &open, 1);
        project.turn_timeout = Duration::from_millis(300);

        let started = start(&project, &ticket("74", "Build the thing"), sink).unwrap();
        assert!(matches!(started, Started::Running { .. }), "{started:?}");
        let next = start(&project, &ticket("75", "Build another"), |_| {}).unwrap();
        assert!(
            matches!(next, Started::Waiting { .. }),
            "the one slot is held: {next:?}"
        );

        let heard = until_turn_ends(&heard);
        assert!(
            matches!(
                heard.last(),
                Some(SessionEvent::TurnEnded {
                    stop_reason: StopReason::Cancelled,
                    ..
                })
            ),
            "the harness was asked to cancel: {heard:?}"
        );
        assert!(agent.methods().contains(&"session/cancel".to_owned()));

        let records = kept(&rig);
        let worker = records.iter().find(|r| r.ticket == "74").unwrap();
        assert_eq!(
            (worker.status, worker.seat),
            (Status::Paused, Seat::Without),
            "Paused, so the maintainer decides what comes next, and holding no slot"
        );
        let waiting = records.iter().find(|r| r.ticket == "75").unwrap();
        assert_eq!(
            waiting.seat,
            Seat::Held,
            "the freed slot went on to the next in line"
        );
    }

    #[test]
    fn a_restarted_worker_is_not_paused_by_the_ceiling_of_the_turn_before() {
        let rig = Rig::new("restart");
        let binding = binding();
        // The harness goes away mid-turn: the turn ends with a `Failed` and no `TurnEnded`
        let crashing = FakeAgent::new().turn(Turn::new().exit());
        let open =
            |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&crashing, opening)) };
        let (sink, heard) = events();
        let mut short = rig.project(&binding, &open, 2);
        // Long enough that the worktree's preparation, which counts, is not
        // what ends the turn
        short.turn_timeout = Duration::from_secs(2);
        let first_start = Instant::now();
        start(&short, &ticket("74", "Build the thing"), sink).unwrap();
        loop {
            let event = heard
                .recv_timeout(Duration::from_secs(20))
                .expect("the first turn never failed");
            if matches!(event, SessionEvent::Failed { .. }) {
                break;
            }
        }

        let healthy = FakeAgent::new()
            .can_resume()
            .knows("fake-1")
            .turn(Turn::new().text("again").until_cancelled());
        let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&healthy, opening)) };
        let (sink, heard) = events();
        let long = rig.project(&binding, &open, 2);
        let started = start(&long, &ticket("74", "Build the thing"), sink).unwrap();
        assert!(matches!(started, Started::Running { .. }), "{started:?}");
        loop {
            let event = heard
                .recv_timeout(Duration::from_secs(20))
                .expect("the second turn never started");
            if matches!(event, SessionEvent::Started { .. }) {
                break;
            }
        }

        // Well past the first turn's ceiling
        std::thread::sleep(
            (first_start + Duration::from_millis(2500)).saturating_duration_since(Instant::now()),
        );

        let records = kept(&rig);
        assert_eq!(
            (records[0].status, records[0].seat),
            (Status::Running, Seat::Held),
            "the first turn's ceiling left the restarted worker alone"
        );
        assert!(
            !healthy.methods().contains(&"session/cancel".to_owned()),
            "and cancelled nothing of it"
        );
    }
}
