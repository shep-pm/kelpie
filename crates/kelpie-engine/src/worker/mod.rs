//! Workers: the sessions that build one ticket each
//!
//! A worker is a session with a worktree of its own doing one ticket
//! (`docs/design/workers.md`). What kelpie decides about the set of them is
//! here: a project runs at most its `worker-slots` of them at once, and one
//! that is Paused or waiting on the maintainer gives its slot back (SK ADR
//! 0007). [`start()`] is the one function that begins a worker, for the Start
//! button, the PM's `start_worker` and the board's dispatch alike. [`ceiling`]
//! stops a turn that runs past its `worker-turn-timeout`, [`turn_end`] checks
//! what a turn left on its branch, [`after_turn`] opens or edits the draft a turn
//! hands over from the file its worker left, [`login`] writes the token a worker pushes
//! with into a `gh` folder of its own, [`record`] keeps what each one is
//! doing, outside every folder it writes, and [`note`], [`stop_and_tell`] and
//! [`stop`] are how the maintainer steers one that runs.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::worker::{Seat, Status, Worker, seat};
//!
//! let mut workers = [
//!     Worker::new(Status::Running),
//!     Worker::new(Status::Paused),
//!     Worker::new(Status::Running),
//!     Worker::new(Status::Running),
//! ];
//! seat(&mut workers, 2);
//! assert_eq!(
//!     workers.map(|worker| worker.seat),
//!     [Seat::Held, Seat::Without, Seat::Held, Seat::Waiting]
//! );
//! ```

use serde::{Deserialize, Serialize};

pub mod ceiling;
mod credential;
pub mod hand_over;
mod instructions;
pub mod login;
mod paragraphs;
pub mod record;
pub mod slots;
pub mod start;
mod tell;
pub mod turn_end;
mod turn_events;

pub use hand_over::{HandOver, Outcome, after_turn};
pub use record::{Record, Records};
pub use slots::{Seat, seat};
pub use start::{Opening, Paths, Project, StartError, Started, Ticket, start};
pub use tell::{Noted, note, stop, stop_and_tell};
pub use turn_end::{Ending, Held};

#[cfg(test)]
mod tests;

/// One of a project's workers, as the slots see it
///
/// These are the two facts the slot count reads. [`Record`] carries them with
/// the ticket, the branch and the session that go with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Worker {
    /// What it is doing
    pub status: Status,

    /// Its standing in the project's slots
    pub seat: Seat,
}

impl Worker {
    /// A worker in `status` holding no slot yet
    #[must_use]
    pub fn new(status: Status) -> Self {
        Self {
            status,
            seat: Seat::Without,
        }
    }
}

/// What one of a project's workers is doing
// wire format: `workers.json` holds these states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive] // kelpie grows these states as its workers do
pub enum Status {
    /// Its turn is running, or is next to start: it needs a slot
    Running,

    /// Its process stopped mid-turn, and it resumes only when the maintainer says so
    Paused,

    /// It has asked the maintainer something, and nothing runs while it waits
    WaitingOnMaintainer,

    /// Its turn ended with the work committed
    Ended,
}

impl Status {
    /// Whether a worker in this state needs one of the project's slots
    #[must_use]
    pub fn needs_slot(self) -> bool {
        matches!(self, Self::Running)
    }
}
