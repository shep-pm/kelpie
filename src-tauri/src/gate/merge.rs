//! What a pass does with the maintainer's yes: mark ready, merge pinned to the head, and watch
//!
//! A ready head merges on the maintainer's yes and on nothing else
//! (`docs/design/gate-and-merge.md`, The merge). [`Gate::yes`] keeps the yes with the head
//! it was given on, and the next pass of that ticket carries it out. The engine decides
//! from plain values ([`kelpie_engine::gate::yes`], `queue`, `refused`, `disarm`); this
//! module reads the world and acts on the answer, as SK's `runner/merge/` does
//! (`docs/lifted.md`).
//!
//! - A draft is marked ready, kelpie's own change, and the merge waits out the checks that
//!   starts.
//! - The merge is recorded as sent in `gate.json` before the call, so a relaunch watches
//!   the pull request rather than merging it twice.
//! - Where the merge queue holds the pull request, the line is in the merge queue until it
//!   merges or the queue removes it. A removal goes back to the worker as an uncounted fix
//!   turn naming the merge queue, and a second at one head is the merge refused ruling.
//! - An armed auto-merge on a head gone red, or not the one the gate saw, is disarmed.
//! - A merge the forge refuses goes back through the order once, which catches the branch
//!   up if the base moved, and a second refusal is the merge refused ruling.
//!
//! A merge leaves the ticket for the board to place in Done once its issue closes.

use std::fmt;

use kelpie_engine::gate::{self, Kept, Limits, Line, Look, Merge, Queue, Red, Refusal, Sent, Step};
use kelpie_engine::github::Timestamp;
use kelpie_engine::ticket::{ForgeError, OpenPull, PullState, QueueStanding, TicketId};

use super::act::Carried;
use super::{Gate, Work, lock};

/// A yes that was refused, because the ticket is not ready to merge
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotReady {
    /// The ticket
    pub ticket: String,

    /// The line it shows instead, none before its first pass
    pub line: Option<Line>,
}

impl fmt::Display for NotReady {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.line {
            Some(line) => write!(
                f,
                "ticket {} is not ready to merge: {}",
                self.ticket,
                line.text()
            ),
            None => write!(f, "ticket {} is not ready to merge", self.ticket),
        }
    }
}

impl std::error::Error for NotReady {}

/// What the merge pass left to the rest of the pass
pub(super) enum After {
    /// It came to something: keep, or forget, as said
    Carried(Carried),
    /// The yes is gone, or never held: the order runs
    Order,
}

/// Whether a failed merge call certainly made no merge, so nothing is left to watch
fn never_sent(why: &ForgeError) -> bool {
    matches!(
        why,
        ForgeError::Spawn(_)
            | ForgeError::Held(_)
            | ForgeError::Limited { .. }
            | ForgeError::LocalPath
            | ForgeError::NoIssue { .. }
    )
}

/// The line of a ticket whose yes the pass carried out
pub(super) fn line_of(kept: &Kept) -> Line {
    if kept.sent.as_ref().is_some_and(|sent| sent.queued) {
        Line::InMergeQueue
    } else {
        Line::ReadyToMerge
    }
}

impl Gate {
    /// The maintainer's yes to merging `ticket`, kept with the head the gate found ready
    ///
    /// The merge itself is the ticket's next pass.
    ///
    /// # Errors
    ///
    /// [`NotReady`], with the line the ticket has, when the gate has not found its head
    /// ready to merge.
    pub fn yes(&self, ticket: &str) -> Result<(), NotReady> {
        let id = ticket.parse::<TicketId>().ok();
        let line = id.and_then(|id| self.line_of_id(id));
        let not_ready = || NotReady {
            ticket: ticket.to_owned(),
            line: line.clone(),
        };
        let id = id.ok_or_else(not_ready)?;
        let mut book = lock(&self.book);
        let head = book
            .get(id)
            .and_then(|kept| kept.ready.clone())
            .ok_or_else(not_ready)?;
        book.entry(id).said_yes(&head);
        if let Err(why) = book.save() {
            eprintln!("kelpie: the yes on ticket {ticket} did not keep: {why}");
        }
        Ok(())
    }

    /// The line `ticket` showed after its last pass, none before its first
    #[must_use]
    pub fn line(&self, ticket: &str) -> Option<Line> {
        self.line_of_id(ticket.parse().ok()?)
    }

    fn line_of_id(&self, id: TicketId) -> Option<Line> {
        lock(&self.lines).get(&id).cloned()
    }

    /// Carry out the yes `kept` holds, or watch the merge it sent
    pub(super) fn merge_pass(
        &self,
        work: &Work,
        look: &Look,
        pull: &OpenPull,
        kept: &mut Kept,
        limits: &Limits,
        now: Timestamp,
    ) -> After {
        if let Some(sent) = kept.sent.clone()
            && let Some(after) = self.watch(work, pull, kept, &sent, limits, now)
        {
            return after;
        }
        if kept.yes.is_none() {
            return After::Order;
        }
        // The order's rulings come before a yes: labels changed by hand, a harness's own file
        let mut probe = kept.clone();
        if matches!(gate::pass(look, &mut probe, limits, now), Step::Rule(_)) {
            kept.withdraw_yes();
            return After::Order;
        }
        match gate::yes(look, pull, kept, limits, now) {
            Merge::Withdraw(why) => {
                eprintln!(
                    "kelpie: the yes on ticket {} is withdrawn: {why}",
                    work.ticket
                );
                kept.withdraw_yes();
                After::Order
            }
            Merge::MarkReady => {
                match self.pulls.mark_ready(pull.number) {
                    Ok(()) => {
                        kept.marked_ready();
                        kept.restart_wait(now);
                    }
                    Err(why) => eprintln!(
                        "kelpie: pull request {} was not marked ready: {why}",
                        pull.number
                    ),
                }
                After::Carried(Carried::Keep)
            }
            Merge::Wait => After::Carried(Carried::Keep),
            Merge::Now => self.merge_now(work, pull, kept, now),
        }
    }

    /// Record the merge as sent, then make it, pinned to the head the yes was given on
    fn merge_now(&self, work: &Work, pull: &OpenPull, kept: &mut Kept, now: Timestamp) -> After {
        let standing = match self.queue.standing(pull.number) {
            Ok(standing) => standing,
            Err(why) => {
                eprintln!(
                    "kelpie: pull request {} was not read in the merge queue: {why}",
                    pull.number
                );
                return After::Carried(Carried::Keep);
            }
        };
        kept.merge_sent(&pull.head, standing.removals, now);
        if standing.queued || standing.armed {
            // Already waiting to merge: a second call would only be refused
            kept.merge_waiting(now, standing.queued);
            return After::Carried(Carried::Keep);
        }
        self.keep_now(work, kept);
        match self.pulls.merge(pull.number, &pull.head) {
            Ok(()) => {
                // Where the base has a merge queue, the call only queued the pull request
                if let Ok(after) = self.queue.standing(pull.number)
                    && (after.queued || after.armed)
                {
                    kept.merge_waiting(now, after.queued);
                }
                After::Carried(Carried::Keep)
            }
            Err(ForgeError::Failed(said)) if !ForgeError::Failed(said.clone()).passes() => {
                kept.merge_gone();
                self.merge_refused(work, kept, &pull.head, &said)
            }
            Err(why) => {
                // Not the forge's refusal: the yes stands and the next pass tries again
                eprintln!(
                    "kelpie: the merge of pull request {} did not go through: {why}",
                    pull.number
                );
                if never_sent(&why) {
                    kept.merge_gone();
                }
                // Otherwise the call may have gone out, so `sent` stays for the next pass to watch
                After::Carried(Carried::Keep)
            }
        }
    }

    /// What a merge the forge refused comes to
    fn merge_refused(&self, work: &Work, kept: &mut Kept, head: &str, why: &str) -> After {
        let answer = gate::refused(kept, head, why);
        kept.merge_refused();
        kept.withdraw_yes();
        match answer {
            // The order runs again, and catches the branch up if the base moved
            Refusal::Again => After::Order,
            Refusal::Rule(ruling) => {
                self.rule(work, kept, &ruling, head);
                After::Carried(Carried::Keep)
            }
        }
    }

    /// Watch the merge `sent`, `None` when nothing came of it and the yes goes on
    fn watch(
        &self,
        work: &Work,
        pull: &OpenPull,
        kept: &mut Kept,
        sent: &Sent,
        limits: &Limits,
        now: Timestamp,
    ) -> Option<After> {
        let standing = match self.queue.standing(pull.number) {
            Ok(standing) => standing,
            Err(why) => {
                eprintln!(
                    "kelpie: pull request {} was not read in the merge queue: {why}",
                    pull.number
                );
                return Some(After::Carried(Carried::Keep));
            }
        };
        match gate::queue(sent, &standing, now) {
            Queue::Wait => {
                if standing.queued {
                    kept.merge_waiting(now, true);
                }
                Some(After::Carried(Carried::Keep))
            }
            Queue::Armed => match gate::disarm(sent, pull, limits) {
                Some(why) => {
                    eprintln!(
                        "kelpie: auto-merge on pull request {} is disarmed: {why}",
                        pull.number
                    );
                    match self.pulls.disable_auto_merge(pull.number) {
                        Ok(()) => {
                            kept.merge_gone();
                            kept.withdraw_yes();
                            Some(After::Order)
                        }
                        Err(why) => {
                            eprintln!("kelpie: auto-merge was not disarmed: {why}");
                            Some(After::Carried(Carried::Keep))
                        }
                    }
                }
                None => {
                    kept.merge_waiting(now, false);
                    Some(After::Carried(Carried::Keep))
                }
            },
            Queue::Removed(reason) => {
                let removed = standing.removals > sent.removals;
                if removed {
                    return Some(self.removed(work, kept, sent, &reason, &standing));
                }
                // The call left no trace and the forge had its time. The pull request may
                // have merged since the listing, which is read before it is sent again.
                if self.pulls.state(pull.number) == Ok(PullState::Merged) {
                    return Some(After::Carried(Carried::Forget));
                }
                kept.merge_gone();
                None
            }
        }
    }

    /// The merge queue removed the pull request: back to the worker, uncounted, or a ruling
    fn removed(
        &self,
        work: &Work,
        kept: &mut Kept,
        sent: &Sent,
        reason: &str,
        standing: &QueueStanding,
    ) -> After {
        let head = sent.head.as_str();
        match gate::removed(kept, head, reason) {
            Red::Rule(ruling) => {
                self.rule(work, kept, &ruling, head);
            }
            Red::FixTurn(prompt) => match (self.fix)(&work.ticket, &prompt) {
                Ok(()) => kept.fix_turn_sent(head, false),
                Err(why) => {
                    // Kept as sent, so the next pass sends it again
                    eprintln!(
                        "kelpie: ticket {}'s worker was not sent back for its queue removal: {why}",
                        work.ticket
                    );
                    return After::Carried(Carried::Keep);
                }
            },
            Red::Rerun(_) => {}
        }
        kept.merge_gone();
        kept.withdraw_yes();
        kept.removals_seen(standing.removals);
        After::Carried(Carried::Keep)
    }

    /// Save what the gate keeps of the ticket now, which a call about to be made must not outrun
    fn keep_now(&self, work: &Work, kept: &Kept) {
        let mut book = lock(&self.book);
        *book.entry(work.id) = kept.clone();
        if let Err(why) = book.save() {
            eprintln!(
                "kelpie: the gate's facts of ticket {} did not keep: {why}",
                work.ticket
            );
        }
    }
}
