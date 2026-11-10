//! The sink that reads a worker's turn into its record

use std::io;
use std::sync::Arc;

use crate::session::SessionEvent;

use super::ceiling::{Verdict, Watch};
use super::start::{Project, lock};
use super::{
    Status,
    turn_end::{self, Ending},
};

/// The caller's sink, with the record read from every event that changes it
///
/// Each change is kept before the event goes on, so a caller that hears a turn
/// end finds the worker settled and holding no slot. A `Failed` that the session
/// goes on after is followed by that turn's `TurnEnded`, which settles the record,
/// unless the turn was cancelled at its ceiling: the worker is Paused then, and
/// stays so.
///
/// A turn's end goes through [`turn_end`] first: the branch is checked with the
/// fence, and what it found settles the record, done for a clean branch and
/// waiting on the maintainer for a held one.
pub(super) fn kept(
    project: &Project<'_>,
    ticket: &str,
    watch: Watch,
    mut events: impl FnMut(SessionEvent) + Send + 'static,
) -> impl FnMut(SessionEvent) + Send + 'static {
    let kept = Arc::clone(project.records);
    let ticket = ticket.to_owned();
    let tree = Arc::clone(project.tree);
    let repo = project.folder.to_path_buf();
    let worktree = project.paths.worktree(&ticket);
    let base = project.base.to_owned();
    move |event: SessionEvent| {
        // The fence runs git, so it answers before the records lock is held
        let ending = match &event {
            SessionEvent::TurnEnded { .. } => Some(turn_end::check(&tree, &repo, &worktree, &base)),
            _ => None,
        };
        let mut records = lock(&kept);
        match &event {
            SessionEvent::Started { session_id } => {
                lost(
                    &ticket,
                    records.set_session(&ticket, session_id),
                    "its session id",
                );
            }
            SessionEvent::Failed { .. } => {
                lost(
                    &ticket,
                    records.set_status(&ticket, Status::Paused),
                    "its pause",
                );
            }
            _ => {}
        }
        // A turn the ceiling cancelled stays as the ceiling left it: Paused
        let settled = ending
            .as_ref()
            .filter(|_| watch.turn_ended() == Verdict::Ended);
        if let Some(ending) = settled {
            lost(
                &ticket,
                records.set_status(&ticket, ending.status()),
                "its end",
            );
        }
        drop(records);
        if let Some(ending) = settled {
            waits(&ticket, ending);
        }
        events(event);
    }
}

/// Name what a held branch holds its ticket for
///
/// The names reach kelpie's log until the app shows a held ticket on its card and
/// in the attention count (`docs/design/ux.md`, Attention).
fn waits(ticket: &str, ending: &Ending) {
    if let Ending::Held(held) = ending {
        eprintln!(
            "kelpie: ticket {ticket} waits on the maintainer: {}",
            held.line()
        );
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
