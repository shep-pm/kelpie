//! The facts the gate keeps about each ticket's pull request between passes
//!
//! A pass reads the pull request once and decides from what it read and from
//! what an earlier pass left. [`Kept`] is what one ticket leaves, and [`Book`]
//! holds every ticket's in `gate.json`, in the project's folder beside
//! `workers.json`, written whole each time. SK kept these on its work item
//! (`core/work_item/`: `known`, `Phase::Ci`, `counts`, `conflict`, `red_head`,
//! `reran`, `merge_queued`) and in memory (`lag.rs`, `queued`'s `Reruns`).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::github::Timestamp;
use crate::ticket::TicketId;

/// The file in a project's folder the gate's facts are kept in
pub const FILE: &str = "gate.json";

/// The labels and draft state a pull request had when the gate first saw it
///
/// Anything else a pass finds is someone else's change.
// wire format: `gate.json` holds this
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen {
    /// Its labels' names
    pub labels: Vec<String>,
    /// Whether it was a draft
    pub draft: bool,
}

/// The head a CI wait is for, and when it began
// wire format: `gate.json` holds this
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wait {
    /// The head the wait is for
    pub head: String,
    /// When the wait began
    pub since: Timestamp,
}

/// The conflict turns a ticket has had, and what the last was for
// wire format: `gate.json` holds this
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    /// The head that conflicted
    pub head: String,
    /// The base commit it conflicted with
    pub base: String,
    /// The turns sent so far
    pub turns: u32,
}

/// A forge head the branch has not caught up to, and when that began
// wire format: `gate.json` holds this
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lag {
    /// The head the forge shows
    pub head: String,
    /// When the forge first showed it while the branch was elsewhere
    pub since: Timestamp,
}

/// The runs rerun for a head's queued checks
// wire format: `gate.json` holds this
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedReruns {
    /// The head they were rerun at
    pub head: String,
    /// The runs
    pub runs: Vec<u64>,
}

/// A merge kelpie sent, recorded before the call is made
///
/// A relaunch that finds one watches the pull request rather than merging again.
// wire format: `gate.json` holds this
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    /// The head the merge was pinned to
    pub head: String,
    /// The merge queue's removals of the pull request when the merge was sent
    pub removals: u32,
    /// When it was sent, or last seen in the merge queue
    pub since: Timestamp,
    /// Whether the call is known to have left the pull request open and waiting, in the queue or armed
    #[serde(default)]
    pub queued: bool,
}

/// What the gate keeps about one ticket's pull request
// wire format: `gate.json` holds this
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kept {
    /// The heads the worker or kelpie pushed, which the head moving to is no foreign change
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pushed: Vec<String>,
    /// The labels and draft state first seen
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<Seen>,
    /// The head the wait for CI is on, and when it began
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<Wait>,
    /// The heads whose failed runs were rerun
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reran: Vec<String>,
    /// The runs rerun for stuck checks
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued: Option<QueuedReruns>,
    /// The heads a fix turn was sent for, or the merge queue removed
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub red: Vec<String>,
    /// The fix turns used on red runs, which count against the project's fix attempts
    #[serde(default)]
    pub fix_turns: u32,
    /// The conflict turns used, with the head and base the last was for
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Conflict>,
    /// When the forge's head began to lag the branch
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lag: Option<Lag>,
    /// The head the maintainer's yes was given on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yes: Option<String>,
    /// The merge sent on the yes, which the gate watches until it merges or the queue removes it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<Sent>,
    /// The merges the forge refused
    #[serde(default)]
    pub merge_refusals: u32,
    /// The merge queue removals already sent back
    #[serde(default)]
    pub queue_removals: u32,
    /// The pull request's number, as the last listing had it, so a pull request that
    /// left the listing is read once with its number
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<u64>,
    /// The head the gate last ruled at, which a worker waiting on that ruling gets no
    /// pass for until its head moves
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ruled: Option<String>,
    /// The reason the gate put on the worker's record when it ruled, which tells the
    /// gate's wait from one of the worker's own
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ruled_for: Option<String>,
    /// The head the gate found ready to merge, which the ticket's needs-you line says
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<String>,
}

impl Kept {
    /// Records that the worker or kelpie pushed `head`
    pub fn pushed(&mut self, head: &str) {
        if !self.pushed.iter().any(|known| known == head) {
            self.pushed.push(head.to_owned());
        }
    }

    /// Records that kelpie marked the pull request ready, so that is no foreign change
    pub fn marked_ready(&mut self) {
        if let Some(seen) = &mut self.seen {
            seen.draft = false;
        }
    }

    /// Starts the wait for CI again from `now`, on the head it is on
    pub fn restart_wait(&mut self, now: Timestamp) {
        if let Some(wait) = &mut self.wait {
            wait.since = now;
        }
    }

    /// Records that the failed runs of `head` were rerun, and starts the wait again
    ///
    /// The checks read red for a moment after a rerun starts, so the verdict
    /// waits out the time a new head's checks do.
    pub fn reran_failed(&mut self, head: &str, now: Timestamp) {
        if !self.reran.iter().any(|known| known == head) {
            self.reran.push(head.to_owned());
        }
        self.restart_wait(now);
    }

    /// Records that `run`, a stuck check's, was rerun at `head`, and starts the wait again
    pub fn reran_queued(&mut self, head: &str, run: u64, now: Timestamp) {
        match &mut self.queued {
            Some(queued) if queued.head == head => queued.runs.push(run),
            _ => {
                self.queued = Some(QueuedReruns {
                    head: head.to_owned(),
                    runs: vec![run],
                });
            }
        }
        self.restart_wait(now);
    }

    /// Records a fix turn sent for a red run at `head`
    ///
    /// `counted` is false for a merge queue removal, which the project's fix
    /// attempts do not cap.
    pub fn fix_turn_sent(&mut self, head: &str, counted: bool) {
        if counted {
            self.fix_turns = self.fix_turns.saturating_add(1);
        }
        if !self.red.iter().any(|known| known == head) {
            self.red.push(head.to_owned());
        }
    }

    /// Records a conflict turn sent for `head` against `base`
    pub fn conflict_sent(&mut self, head: &str, base: &str) {
        let turns = self.conflict.as_ref().map_or(0, |sent| sent.turns) + 1;
        self.conflict = Some(Conflict {
            head: head.to_owned(),
            base: base.to_owned(),
            turns,
        });
    }

    /// Records that the gate ruled at `head` with `reason`, which is no longer ready
    pub fn ruled_at(&mut self, head: &str, reason: &str) {
        self.ruled = Some(head.to_owned());
        self.ruled_for = Some(reason.to_owned());
        self.ready = None;
    }

    /// Gives up the ruling, so the gate passes over the ticket again
    pub fn unruled(&mut self) {
        self.ruled = None;
        self.ruled_for = None;
    }

    /// Whether a worker waiting with `reason` is waiting on the gate's ruling
    ///
    /// A worker that waits for another reason, such as a question or a hand-over's
    /// ruling, is not, whatever the gate ruled before.
    #[must_use]
    pub fn rules_with(&self, reason: Option<&str>) -> bool {
        self.ruled.is_some() && self.ruled_for.is_some() && self.ruled_for.as_deref() == reason
    }

    /// Whether the gate ruled at `head` and nothing has moved it since
    #[must_use]
    pub fn rules_at(&self, head: &str) -> bool {
        self.ruled.as_deref() == Some(head)
    }

    /// Records that `head` is ready to merge on the maintainer's yes
    pub fn ready_at(&mut self, head: &str) {
        self.ready = Some(head.to_owned());
    }

    /// Records the maintainer's yes, which holds only for `head`
    pub fn said_yes(&mut self, head: &str) {
        self.yes = Some(head.to_owned());
    }

    /// Whether the maintainer's yes was given on `head`
    #[must_use]
    pub fn yes_holds(&self, head: &str) -> bool {
        self.yes.as_deref() == Some(head)
    }

    /// Withdraws the yes, so the gate runs again
    pub fn withdraw_yes(&mut self) {
        self.yes = None;
    }

    /// Records that a merge pinned to `head` is about to be sent, with the queue's `removals` so far
    ///
    /// The caller saves it before the call, so a relaunch watches the pull request
    /// rather than merging it twice.
    pub fn merge_sent(&mut self, head: &str, removals: u32, now: Timestamp) {
        self.sent = Some(Sent {
            head: head.to_owned(),
            removals,
            since: now,
            queued: false,
        });
    }

    /// Records that the sent merge left the pull request open and waiting, seen at `now`
    pub fn merge_waiting(&mut self, now: Timestamp) {
        if let Some(sent) = &mut self.sent {
            sent.queued = true;
            sent.since = now;
        }
    }

    /// Gives up the sent merge, which the pull request is no longer waiting on
    pub fn merge_gone(&mut self) {
        self.sent = None;
    }

    /// Records a merge the forge refused, and how many it has refused
    pub fn merge_refused(&mut self) -> u32 {
        self.merge_refusals = self.merge_refusals.saturating_add(1);
        self.merge_refusals
    }

    /// Records that the merge queue's removals up to `count` were sent back
    pub fn removals_seen(&mut self, count: u32) {
        self.queue_removals = count;
    }
}

/// Every ticket's [`Kept`], and the file that keeps them
#[derive(Debug)]
pub struct Book {
    file: PathBuf,
    tickets: BTreeMap<TicketId, Kept>,
}

impl Book {
    /// The facts `file` holds, none when it is not there yet
    ///
    /// # Errors
    ///
    /// The file cannot be read, or is not a map of tickets' facts.
    pub fn load(file: PathBuf) -> io::Result<Self> {
        let tickets = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is not the gate's facts: {error}", file.display()),
                )
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { file, tickets })
    }

    /// The file the facts are kept in
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// What is kept about `ticket`, none before a pass first saw it
    #[must_use]
    pub fn get(&self, ticket: TicketId) -> Option<&Kept> {
        self.tickets.get(&ticket)
    }

    /// Every ticket with something kept, in ticket order
    pub fn iter(&self) -> impl Iterator<Item = (TicketId, &Kept)> {
        self.tickets.iter().map(|(ticket, kept)| (*ticket, kept))
    }

    /// What is kept about `ticket`, to read and change, started empty when none is
    pub fn entry(&mut self, ticket: TicketId) -> &mut Kept {
        self.tickets.entry(ticket).or_default()
    }

    /// Forgets `ticket`, whose pull request merged or which ended
    pub fn forget(&mut self, ticket: TicketId) {
        self.tickets.remove(&ticket);
    }

    /// Writes every ticket's facts, whole, beside the file and renamed into place
    ///
    /// A crash leaves the old facts or the new ones.
    ///
    /// # Errors
    ///
    /// The file or its folder cannot be written.
    pub fn save(&self) -> io::Result<()> {
        if let Some(folder) = self.file.parent() {
            fs::create_dir_all(folder)?;
        }
        let json = serde_json::to_vec_pretty(&self.tickets).map_err(io::Error::other)?;
        let partial = self.file.with_extension("json.partial");
        fs::write(&partial, json)?;
        fs::rename(&partial, &self.file)
    }
}
