//! Telling a running worker something: a Note, Stop and tell, and Stop
//!
//! The maintainer steers a worker without waiting for its pull request
//! (`docs/design/workers.md`, Talking to a worker). Each move is the engine's own,
//! because [`Session::send`] on its own queues a second prompt with a sink that runs
//! no record logic, and [`Session::cancel`] followed by a send would let the
//! cancelled turn settle the record and free the slot.
//!
//! A [`note`] waits for the turn's end on every harness, so the behaviour is the
//! same on each. Steering a turn in flight is per harness (Claude Code's queued
//! second prompt, goose's `_goose/unstable/session/steer`) and comes later. The
//! turn's end sends the notes ([`super::turn_events`]).
//!
//! A worker that is Paused or waiting on the maintainer takes a note or a Stop and
//! tell into its record, and the next start carries it: its session ended, and
//! resuming it is the maintainer's to say.

use std::io;
use std::sync::Arc;
use std::time::Instant;

use crate::session::Session;

use super::ceiling::{Ceiling, Watch};
use super::start::{Project, lock};
use super::turn_events::Turns;
use super::{Records, Seat, Status};

/// What became of a Note, or of a Stop and tell
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Noted {
    /// The text is in the ticket's record, and goes out with the worker's next prompt
    Queued,

    /// The text went to the worker as its next turn
    Sent,
}

/// Tell `ticket`'s worker `text` without interrupting it
///
/// While a turn runs, the text is added to the ticket's record and [`Noted::Queued`]
/// returns: the turn's end sends every note together. A worker that is idle, with
/// its turn ended, takes a slot and is sent the text as its next turn
/// ([`Noted::Sent`]), the way [`super::start()`] sends one. A worker that is Paused or
/// waiting on the maintainer, one with no free slot, and one that no turn ran for
/// since the launch keep the text for the turn its next start begins.
///
/// # Errors
///
/// [`io::ErrorKind::NotFound`] when the project has no worker on `ticket`, or the
/// records file could not be written.
pub fn note(
    project: &Project<'_>,
    session: &Arc<Session>,
    ticket: &str,
    text: &str,
) -> io::Result<Noted> {
    let mut records = lock(project.records);
    match standing(&records, ticket)? {
        Standing::Idle => send_next(project, session, ticket, text, records),
        Standing::Turn | Standing::Away => queue(&mut records, ticket, text),
    }
}

/// Cancel `ticket`'s turn over ACP and send `text` as the next prompt
///
/// The cancelled turn's end settles nothing, so the worker stays Running and keeps
/// its slot, and the new turn's end settles it as any other. Notes already queued
/// stay in the record and go out when the new turn ends. A worker with no turn to
/// interrupt is told as [`note`] tells it.
///
/// # Errors
///
/// As [`note`].
pub fn stop_and_tell(
    project: &Project<'_>,
    session: &Arc<Session>,
    ticket: &str,
    text: &str,
) -> io::Result<Noted> {
    let mut records = lock(project.records);
    match standing(&records, ticket)? {
        Standing::Idle => send_next(project, session, ticket, text, records),
        Standing::Away => queue(&mut records, ticket, text),
        Standing::Turn => {
            let Some(turns) = Turns::resume(project, ticket, session, &records) else {
                return queue(&mut records, ticket, text);
            };
            if let Some(turn) = records.watch_of(ticket) {
                turn.replace();
            }
            let watch = Watch::new();
            records.keep_watch(ticket, watch.clone());
            drop(records);
            // The cancel goes first, so it lands on the turn and not on the next one
            session.cancel();
            turns.dispatch(session, text.to_owned(), watch, ceiling_of(project));
            Ok(Noted::Sent)
        }
    }
}

/// Cancel `ticket`'s turn and leave its worker Paused
///
/// As the turn ceiling does: the worker's slot goes to the next in line, and the
/// maintainer resumes the worker by hand. Notes in its record stay for that turn.
/// Does nothing when no turn is running.
///
/// # Errors
///
/// The records file could not be written. The turn is cancelled all the same.
pub fn stop(project: &Project<'_>, session: &Arc<Session>, ticket: &str) -> io::Result<()> {
    let mut records = lock(project.records);
    let Some(turn) = records.watch_of(ticket) else {
        return Ok(());
    };
    // With the records locked, so the turn's end finds the worker Paused
    if !turn.stop() {
        return Ok(());
    }
    let paused = records
        .set_status(ticket, Status::Paused)
        .and_then(|()| records.seat(project.slots).map(drop));
    drop(records);
    session.cancel();
    paused
}

/// Where a worker stands for what the maintainer tells it
enum Standing {
    /// A turn runs, or holds its slot
    Turn,

    /// Its turn ended with the work committed
    Idle,

    /// Its turn ended and it will not go on by itself, or it waits for a slot
    Away,
}

fn standing(records: &Records, ticket: &str) -> io::Result<Standing> {
    let record = records.get(ticket).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no worker is on ticket {ticket}"),
        )
    })?;
    Ok(match (record.status, record.seat) {
        (Status::Running, Seat::Held) => Standing::Turn,
        (Status::Ended, _) => Standing::Idle,
        _ => Standing::Away,
    })
}

/// Add `text` to `ticket`'s notes, for the turn it starts next
fn queue(records: &mut Records, ticket: &str, text: &str) -> io::Result<Noted> {
    records.add_note(ticket, text)?;
    Ok(Noted::Queued)
}

/// Send `text` as the next turn of a worker whose turn ended
///
/// The worker takes a slot as a start does. With none free it waits in line, and
/// the text is kept in its record for the turn that start begins.
fn send_next(
    project: &Project<'_>,
    session: &Arc<Session>,
    ticket: &str,
    text: &str,
    mut records: std::sync::MutexGuard<'_, Records>,
) -> io::Result<Noted> {
    let Some(turns) = Turns::resume(project, ticket, session, &records) else {
        return queue(&mut records, ticket, text);
    };
    records.set_status(ticket, Status::Running)?;
    records.seat(project.slots)?;
    if records.get(ticket).map(|record| record.seat) != Some(Seat::Held) {
        return queue(&mut records, ticket, text);
    }
    let watch = Watch::new();
    records.keep_watch(ticket, watch.clone());
    drop(records);
    turns.dispatch(session, text.to_owned(), watch, ceiling_of(project));
    Ok(Noted::Sent)
}

/// The ceiling of a turn sent now
fn ceiling_of(project: &Project<'_>) -> Ceiling {
    Ceiling::from_grant(Instant::now(), project.turn_timeout)
}
