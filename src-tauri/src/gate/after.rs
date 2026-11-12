//! What a pass does once the pull request is merged: close the issue, stop the worker, clean up
//!
//! A merge by kelpie or by hand leaves nothing behind (`docs/design/gate-and-merge.md`,
//! After the merge). The ticket is Done once its issue is closed, which the board reads, and
//! nothing is said to the maintainer: an OS notification fires only when work waits on them.
//! The engine decides the issue from plain values ([`kelpie_engine::gate::closing`]); this
//! module reads the forge and acts, as SK's `runner/close_merged.rs` and
//! `runner/merge/cleanup.rs` do (`docs/lifted.md`).
//!
//! - The worker's session stops and its worktree, build folder and remote branch go
//!   ([`worktree::remove`], which checks the branch gone with `ls-remote`).
//! - An issue the pull request names is left to GitHub for `FORGE_CLOSE_WAIT`. One still
//!   open after that, or named by no closing reference, gets one comment and a close
//!   through [`Tickets`], with `TRIES` tries.
//! - The worker's record goes last, once both hold, since it is what brings the next read
//!   back to the ticket.
//!
//! A step that fails is tried on the next read, and what holds is kept in `gate.json`.
//! Cleaning up a worker that is no longer as the pass found it waits: the maintainer
//! resumed it.

use kelpie_engine::gate::{Close, Kept, Merged, closing, comment};
use kelpie_engine::github::Timestamp;
use kelpie_engine::ticket::ForgeError;
use kelpie_engine::worktree::{self, Remote};

use super::act::Carried;
use super::{Gate, Work, lock};

impl Gate {
    /// Carry out what a merge leaves to do, once for each read until it all holds
    pub(super) fn after_merge(&self, work: &Work, kept: &mut Kept, now: Timestamp) -> Carried {
        let mut merged = kept
            .merged
            .take()
            .unwrap_or_else(|| Merged::new(now, kept.names));
        let done = self.settle(work, kept.pull, &mut merged, now);
        let carried = if done { Carried::Forget } else { Carried::Keep };
        if matches!(carried, Carried::Keep) {
            kept.merged = Some(merged);
        }
        carried
    }

    /// One read's steps, `true` once everything holds and the record is gone
    fn settle(&self, work: &Work, pull: Option<u64>, merged: &mut Merged, now: Timestamp) -> bool {
        if !merged.cleaned {
            if !self.as_found(work) {
                eprintln!(
                    "kelpie: ticket {} is not where its pass left it, so it is not cleaned up yet",
                    work.ticket
                );
                return false;
            }
            (self.stop)(&work.ticket);
            if !self.clean(work) {
                return false;
            }
            merged.clean();
        }
        if !merged.settled {
            self.close_issue(work, pull, merged, now);
        }
        merged.settled && self.drop_record(work)
    }

    /// Whether the worker's record is as the pass began it
    fn as_found(&self, work: &Work) -> bool {
        let Some(records) = self.records() else {
            return false;
        };
        lock(&records)
            .get(&work.ticket)
            .is_some_and(|record| work.standing.holds(record))
    }

    /// Remove the worktree, the build folder and the branches, `false` when that failed
    fn clean(&self, work: &Work) -> bool {
        let checkout = lock(&self.checkout);
        let removed = worktree::remove(
            &self.tree,
            &self.folder,
            &self.paths.worktree(&work.ticket),
            &work.branch,
            &self.paths.build_dir(&work.ticket),
            Some(&Remote::Origin),
        );
        drop(checkout);
        match removed {
            Ok(()) => true,
            Err(why) => {
                eprintln!(
                    "kelpie: ticket {}'s worktree was not removed: {why}",
                    work.ticket
                );
                false
            }
        }
    }

    /// Comment once and close the issue where GitHub has not, once the wait is over
    ///
    /// Settles `merged` when the issue is closed or the tries are used up, whether they were
    /// closes the forge refused or reads of the issue that failed.
    fn close_issue(&self, work: &Work, pull: Option<u64>, merged: &mut Merged, now: Timestamp) {
        let open = match self.tickets.issue(&work.id) {
            Ok(issue) => issue.open,
            Err(why) => {
                eprintln!(
                    "kelpie: issue {} was not read to close it: {why}",
                    work.ticket
                );
                if merged.read_failed(now) {
                    eprintln!(
                        "kelpie: issue {} could not be read after its tries, and is left as it is",
                        work.ticket
                    );
                    merged.settle();
                }
                return;
            }
        };
        merged.read();
        match closing(merged, open, now) {
            Close::Closed => merged.settle(),
            Close::Wait => {}
            Close::GiveUp => {
                eprintln!(
                    "kelpie: issue {} was not closed after its tries, and is left open",
                    work.ticket
                );
                merged.settle();
            }
            Close::Write { comment: first } => {
                if first {
                    // Said once, whether or not it went through: the close matters more
                    merged.commented();
                    let said = comment(pull.unwrap_or_default());
                    if let Err(why) = self.tickets.comment(&work.id, &said) {
                        eprintln!("kelpie: issue {} was not commented on: {why}", work.ticket);
                    }
                }
                match self.tickets.close(&work.id) {
                    Ok(()) => merged.settle(),
                    Err(why) => self.close_failed(work, merged, &why),
                }
            }
        }
    }

    fn close_failed(&self, work: &Work, merged: &mut Merged, why: &ForgeError) {
        eprintln!("kelpie: issue {} was not closed: {why}", work.ticket);
        merged.close_failed();
    }

    /// Remove the worker's record, `false` when that failed
    fn drop_record(&self, work: &Work) -> bool {
        let Some(records) = self.records() else {
            return false;
        };
        let removed = lock(&records).remove(&work.ticket);
        match removed {
            Ok(_) => true,
            Err(why) => {
                self.lost(&work.ticket, Err(why), "its removal");
                false
            }
        }
    }
}
