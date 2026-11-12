//! What a pass does about the [`Step`] the order found
//!
//! The order only says what to do ([`kelpie_engine::gate::pass`]). Each step is carried out
//! here, and what it came to is recorded in the ticket's [`Kept`] with the engine's own
//! methods, once it has been done:
//!
//! - [`Step::CatchUp`] rebases the branch onto the latest base and pushes it. The head it
//!   pushed is kelpie's own, so the head moving to it is no foreign change. A conflict is
//!   the worker's next turn, with the merge left in progress in its worktree for it to
//!   resolve, until the turns are used up and the merge is given up.
//! - [`Step::Rerun`] starts the stuck run again, once per head.
//! - [`Step::Rule`] parks the worker waiting on the maintainer with the ruling's reason, as a
//!   hand-over's ruling does, and the gate passes it no more until its head moves.
//! - [`Step::Yes`] keeps the head ready to merge, which the ticket's needs-you line says.
//!   The maintainer's yes merges it, which `merge` carries out on the ticket's next pass.
//! - [`Step::Red`] is red CI, which `red` settles: a rerun of a run likely not the worker's,
//!   a ruling, or the worker's next turn with the failing logs.
//! - [`Step::Done`] ends the ticket's time in the gate. Cleaning up after the merge comes later.

use kelpie_engine::gate::{Kept, Limits, Look, Pull, Ruling, Step, conflicted};
use kelpie_engine::github::Timestamp;
use kelpie_engine::worker::Status;
use kelpie_engine::worktree::{self, Rebase, Remote};

use super::{Gate, Work, lock};
use crate::workers::BASE;

/// What the book does with a ticket after its pass
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Carried {
    /// Keep what the pass left
    Keep,
    /// The ticket left the gate
    Forget,
}

impl Gate {
    /// Carry out `step`, changing `kept` by what it came to
    pub(super) fn carry_out(
        &self,
        work: &Work,
        look: &Look,
        kept: &mut Kept,
        step: Step,
        limits: &Limits,
        now: Timestamp,
    ) -> Carried {
        let head = match &look.pull {
            Pull::Open(pull) => pull.head.as_str(),
            // A pull request that is not open has no head to act on
            _ => "",
        };
        match step {
            Step::Done => return Carried::Forget,
            Step::Rule(ruling) => {
                self.rule(work, kept, &ruling, head);
            }
            other => {
                let ruled = match other {
                    Step::CatchUp => self.catch_up(work, kept, head),
                    Step::Conflict(prompt) => {
                        self.send_conflict(work, kept, head, "", &prompt);
                        false
                    }
                    Step::Rerun(runs) => {
                        self.rerun(&runs, kept, head, now);
                        false
                    }
                    Step::Red(checks) => self.red(work, kept, head, &checks, limits, now),
                    Step::Yes => {
                        kept.ready_at(head);
                        false
                    }
                    _ => false,
                };
                // The gate has something to do, so a worker waiting on an old ruling is
                // back in play. A ruling the step came to is written over that wait
                // while the record is as the pass began it, so it is not reinstated first.
                if !ruled {
                    self.reinstated(work, kept);
                }
            }
        }
        Carried::Keep
    }

    /// Rebase the branch onto the latest base and push it, and say what the push left
    ///
    /// `true` when the catch-up came to a ruling.
    fn catch_up(&self, work: &Work, kept: &mut Kept, head: &str) -> bool {
        let worktree = self.paths.worktree(&work.ticket);
        let checkout = lock(&self.checkout);
        let caught = worktree::rebase(
            &self.tree,
            &self.folder,
            &worktree,
            &work.branch,
            head,
            true,
            &Remote::Origin,
            BASE,
        );
        drop(checkout);
        match caught {
            Ok(Rebase::Pushed(pushed)) => kept.pushed(&pushed),
            Ok(Rebase::Conflicts { base, files }) => match conflicted(kept, head, &base, &files) {
                Step::Conflict(prompt) => self.send_conflict(work, kept, head, &base, &prompt),
                Step::Rule(ruling) => {
                    self.give_up_merge(work);
                    return self.rule(work, kept, &ruling, head);
                }
                _ => {}
            },
            Ok(Rebase::Refused(why)) => {
                return self.rule(work, kept, &Ruling::CatchUpRefused { why }, head);
            }
            Err(why) => eprintln!(
                "kelpie: ticket {} was not caught up with its base: {why}",
                work.ticket
            ),
        }
        false
    }

    /// Send the worker back to resolve the merge catch-up left in progress
    fn send_conflict(&self, work: &Work, kept: &mut Kept, head: &str, base: &str, prompt: &str) {
        match (self.fix)(&work.ticket, prompt) {
            Ok(()) => kept.conflict_sent(head, base),
            Err(why) => {
                // Left in progress, the merge would be refused on the next look, and the
                // worker was not told to resolve it
                self.give_up_merge(work);
                eprintln!(
                    "kelpie: ticket {}'s worker was not sent back for its conflict: {why}",
                    work.ticket
                );
            }
        }
    }

    /// Give up the merge catch-up left in progress, which leaves the worktree at its head
    fn give_up_merge(&self, work: &Work) {
        let worktree = self.paths.worktree(&work.ticket);
        if let Err(why) = worktree::abort_merge(&self.tree, &self.folder, &worktree) {
            eprintln!(
                "kelpie: ticket {}'s merge was not given up: {why}",
                work.ticket
            );
        }
    }

    /// Start the stuck runs again, until the forge accepts one, which ends the stuck queue
    fn rerun(&self, runs: &[u64], kept: &mut Kept, head: &str, now: Timestamp) {
        for run in runs {
            match self.ci.rerun(*run) {
                Ok(()) => {
                    kept.reran_queued(head, *run, now);
                    return;
                }
                Err(why) => eprintln!("kelpie: run {run} was not started again: {why}"),
            }
        }
    }

    /// Park the worker on the maintainer with `ruling`'s reason, which holds no slot
    ///
    /// The reason goes with the status under one hold of the records' lock, as the engine's
    /// own settling does. A record the maintainer moved while the pass was at work is left
    /// as they left it, and the head is not marked ruled: that worker is not waiting.
    pub(super) fn rule(&self, work: &Work, kept: &mut Kept, ruling: &Ruling, head: &str) -> bool {
        let Some(records) = self.records() else {
            return false;
        };
        let mut records = lock(&records);
        if records
            .get(&work.ticket)
            .is_none_or(|record| !work.standing.holds(record))
        {
            eprintln!(
                "kelpie: ticket {} is not where its pass left it, so its ruling is not recorded",
                work.ticket
            );
            return false;
        }
        let reason = ruling.reason();
        let written = records
            .set_status(&work.ticket, Status::WaitingOnMaintainer)
            .and_then(|()| records.set_reason(&work.ticket, Some(reason.clone())));
        drop(records);
        self.lost(&work.ticket, written, "the gate's ruling");
        kept.ruled_at(head, &reason, ruling.link());
        if matches!(ruling, Ruling::Closed) {
            // Nothing is left to read once the pull request is closed
            kept.pull = None;
        }
        true
    }

    /// Set the worker Ended and clear the reason, when it was waiting on a ruling of the gate's
    ///
    /// Only the record the pass began from is written ([`Standing`]).
    fn reinstated(&self, work: &Work, kept: &mut Kept) {
        let was_ruled = kept.ruled.is_some();
        kept.unruled();
        if !was_ruled || work.standing.status != Status::WaitingOnMaintainer {
            return;
        }
        let Some(records) = self.records() else {
            return;
        };
        let mut records = lock(&records);
        if records
            .get(&work.ticket)
            .is_none_or(|record| !work.standing.holds(record))
        {
            return;
        }
        let written = records
            .set_status(&work.ticket, Status::Ended)
            .and_then(|()| records.set_reason(&work.ticket, None));
        drop(records);
        self.lost(&work.ticket, written, "the gate's ruling going away");
    }
}
