//! Workers: the sessions that build one ticket each
//!
//! A worker is a session with a worktree of its own doing one ticket
//! (`docs/design/workers.md`). What kelpie decides about the set of them is
//! here: a project runs at most its `worker-slots` of them at once, and one
//! that is Paused or waiting on the maintainer gives its slot back (SK ADR
//! 0007). The session, the worktree and the ticket come with the code that
//! starts a worker.
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

pub mod slots;

pub use slots::{Seat, seat};

/// One of a project's workers, as the slots see it
///
/// The session, the worktree and the ticket come with the code that starts a
/// worker; these are the two facts the slot count reads.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
