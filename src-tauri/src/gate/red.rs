//! What a pass does about a red run: the worker's to fix, or rerun when it is likely not
//!
//! The order only says the head is red ([`kelpie_engine::gate::Step::Red`]). The engine's
//! [`gate::red`] decides what the red run comes to, from what is read here: the end of each
//! failed run's log, and the files the branch changed. The worker's credential cannot read
//! CI, so the log travels in the prompt (`docs/design/gate-and-merge.md`, Red CI).
//!
//! - A run likely not the worker's is started again through `Ci::rerun`, once per head.
//!   A rerun the forge refuses leaves the red run to the worker, as SK does.
//! - A fix turn goes through [`Fix`](super::Fix), counts against the project's fix attempts,
//!   and is kept in `gate.json` with the head it was sent for.
//! - A ruling is a worker parked on the maintainer, as every other ruling is.

use kelpie_engine::ci::FailedCheck;
use kelpie_engine::gate::{self, Kept, Limits, Logs, Red};
use kelpie_engine::github::Timestamp;
use kelpie_engine::worktree::changed::changed_files;

use super::{Gate, Work, lock};
use crate::workers::BASE;

/// The most failed runs' logs a fix turn carries
const LOGS: usize = 3;

impl Gate {
    /// Settle a red `head`: rerun it, rule on it, or send the worker back with its logs
    ///
    /// `true` when it came to a ruling.
    pub(super) fn red(
        &self,
        work: &Work,
        kept: &mut Kept,
        head: &str,
        checks: &[FailedCheck],
        limits: &Limits,
        now: Timestamp,
    ) -> bool {
        let logs = self.logs_of(head);
        let changed = self.changed(work, head);
        let mut answer = gate::red(head, checks, &logs, &changed, kept, limits);
        if let Red::Rerun(runs) = &answer {
            if self.rerun_all(runs, kept, head, now) {
                return false;
            }
            // Nothing could be started again, so the red run is the worker's, with its
            // logs: deciding as though the logs named a changed file
            answer = gate::red(head, checks, &logs, &[], kept, limits);
        }
        match answer {
            Red::Rule(ruling) => return self.rule(work, kept, &ruling, head),
            Red::FixTurn(prompt) => self.send_red(work, kept, head, &prompt),
            Red::Rerun(_) => {}
        }
        false
    }

    /// The end of each failed run's log of `head`, at most [`LOGS`] of them
    fn logs_of(&self, head: &str) -> Logs {
        match self.ci.logs(head, false) {
            Ok(mut logs) => {
                logs.truncate(LOGS);
                Logs::Read(logs)
            }
            Err(why) => Logs::Refused(why.to_string()),
        }
    }

    /// The files the branch changed from its base, none when git cannot say
    ///
    /// None leaves the run to the worker: a branch with no changed file read is not
    /// one whose logs can be called unrelated.
    fn changed(&self, work: &Work, head: &str) -> Vec<String> {
        let checkout = lock(&self.checkout);
        let changed = changed_files(&self.tree, &self.folder, head, BASE);
        drop(checkout);
        changed.unwrap_or_else(|why| {
            eprintln!(
                "kelpie: ticket {}'s changed files were not read: {why}",
                work.ticket
            );
            Vec::new()
        })
    }

    /// Start each of `runs` again, and keep the head as rerun when the forge took any
    fn rerun_all(&self, runs: &[u64], kept: &mut Kept, head: &str, now: Timestamp) -> bool {
        let mut started = false;
        for run in runs {
            match self.ci.rerun(*run) {
                Ok(()) => started = true,
                Err(why) => eprintln!("kelpie: run {run} was not started again: {why}"),
            }
        }
        if started {
            kept.reran_failed(head, now);
        }
        started
    }

    /// Send the worker back with the red prompt, and keep the turn once it was sent
    fn send_red(&self, work: &Work, kept: &mut Kept, head: &str, prompt: &str) {
        match (self.fix)(&work.ticket, prompt) {
            Ok(()) => kept.fix_turn_sent(head, true),
            Err(why) => eprintln!(
                "kelpie: ticket {}'s worker was not sent back for its red run: {why}",
                work.ticket
            ),
        }
    }
}
