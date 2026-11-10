//! Slots: how many of a project's workers run at once
//!
//! A slot bounds a worker's turns. A worker takes one when it starts and gives
//! it back while it is Paused, waiting on the maintainer, or ended (SK ADR
//! 0007), so an unanswered ask never stalls the project. A worker that gave one
//! back waits in line for a freed slot, ahead of any worker not started yet, so
//! no turn runs past the project's cap.

use super::Worker;

/// One worker's standing in its project's slots
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    /// It holds one of the project's slots
    Held,

    /// It needs a slot and none is free
    Waiting,

    /// It needs none: Paused, waiting on the maintainer, or ended
    Without,
}

/// Give each worker in `workers` its seat under `max`, oldest first
///
/// `workers` is in the order the project started them, which is the order freed
/// slots go out in. A worker holding a slot keeps it while it needs one.
/// Returns whether any seat changed.
pub fn seat(workers: &mut [Worker], max: usize) -> bool {
    let was: Vec<Seat> = workers.iter().map(|worker| worker.seat).collect();
    for worker in workers.iter_mut() {
        worker.seat = if !worker.status.needs_slot() {
            Seat::Without
        } else if worker.seat == Seat::Held {
            Seat::Held
        } else {
            Seat::Waiting
        };
    }
    let held = workers
        .iter()
        .filter(|worker| worker.seat == Seat::Held)
        .count();
    for worker in workers
        .iter_mut()
        .filter(|worker| worker.seat == Seat::Waiting)
        .take(max.saturating_sub(held))
    {
        worker.seat = Seat::Held;
    }
    !workers
        .iter()
        .map(|worker| worker.seat)
        .eq(was.iter().copied())
}

#[cfg(test)]
mod tests {
    use super::{Seat, seat};
    use crate::worker::{Status, Worker};

    #[test]
    fn a_third_worker_waits_while_two_hold_slots() {
        let mut workers = [
            Worker::new(Status::Running),
            Worker::new(Status::Running),
            Worker::new(Status::Running),
        ];

        assert!(seat(&mut workers, 2));
        assert_eq!(
            seats(&workers),
            [Seat::Held, Seat::Held, Seat::Waiting],
            "the third waits for a slot"
        );

        assert!(seat(&mut workers, 3), "the cap is the project's");
        assert_eq!(seats(&workers), [Seat::Held, Seat::Held, Seat::Held]);

        assert!(!seat(&mut workers, 3), "nothing changed to report");
    }

    #[test]
    fn a_paused_worker_or_one_waiting_on_the_maintainer_frees_its_slot() {
        let mut workers = [
            seated(Status::Running, Seat::Held),
            seated(Status::Paused, Seat::Held),
            seated(Status::WaitingOnMaintainer, Seat::Held),
            seated(Status::Running, Seat::Waiting),
            seated(Status::Ended, Seat::Held),
        ];

        assert!(seat(&mut workers, 2));

        assert_eq!(
            seats(&workers),
            [
                Seat::Held,
                Seat::Without,
                Seat::Without,
                Seat::Held,
                Seat::Without,
            ],
            "the three that need no slot give theirs back, and the worker waiting for one takes it"
        );
    }

    #[test]
    fn a_freed_slot_goes_to_the_oldest_worker_waiting_for_one() {
        let mut workers = [
            seated(Status::Running, Seat::Held),
            seated(Status::Running, Seat::Without), // resumed, and gave its slot back
            seated(Status::Running, Seat::Without), // started after it
        ];

        assert!(seat(&mut workers, 2));

        assert_eq!(
            seats(&workers),
            [Seat::Held, Seat::Held, Seat::Waiting],
            "the resumed worker takes the freed slot ahead of the newer one"
        );
    }

    #[test]
    fn no_slot_holds_every_worker_waiting() {
        let mut none: [Worker; 0] = [];
        assert!(!seat(&mut none, 2), "an empty project seats nothing");

        let mut workers = [Worker::new(Status::Running), Worker::new(Status::Running)];
        assert!(seat(&mut workers, 0));
        assert_eq!(
            seats(&workers),
            [Seat::Waiting, Seat::Waiting],
            "no worker starts while the cap is 0"
        );
    }

    fn seated(status: Status, seat: Seat) -> Worker {
        Worker { status, seat }
    }

    fn seats(workers: &[Worker]) -> Vec<Seat> {
        workers.iter().map(|worker| worker.seat).collect()
    }
}
