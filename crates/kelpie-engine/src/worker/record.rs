//! The record of each worker: its ticket, its branch, its session and its state
//!
//! One file per project, `<data_dir>/projects/<id>/workers.json`, kept outside
//! every folder a worker writes: its worktree, its clone's git dir and its state
//! folder. A worker that could read its own record could free its own slot or
//! mark itself done, so the file sits in the project's folder, and the worker's
//! sandbox closes kelpie's data folder whole except those three
//! ([`crate::sandbox::worker`]).
//!
//! The list is in the order the project started its workers, which is the order
//! freed slots go out in ([`crate::worker::slots`]). A record is kept when its
//! ticket is started, so a ticket waiting for a slot is listed before it runs,
//! and a launch that ended mid-turn finds who held a slot.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ceiling::Watch;
use super::turn_events::Events;
use super::{Seat, Status, Worker, slots};
use crate::session::SessionId;

/// One worker, as kelpie keeps it
///
/// A ticket kept for a slot is [`Status::Running`] with
/// [`Seat::Waiting`]: it needs a slot and has none.
// wire format: `workers.json` holds these entries
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The ticket it works on
    pub ticket: String,

    /// The branch its worktree is on, `kelpie/<ticket>`
    pub branch: String,

    /// The id the harness gave its session, kept to resume it.
    /// `None` until a turn starts one.
    pub session: Option<SessionId>,

    /// What it is doing
    pub status: Status,

    /// Its standing in the project's slots
    pub seat: Seat,

    /// What the maintainer told it that its next prompt carries, oldest first
    ///
    /// A Note waits here while a turn runs, and the turn's end sends it
    /// (`docs/design/workers.md`, Talking to a worker). A record from before
    /// notes had none.
    #[serde(default)]
    pub notes: Vec<String>,

    /// Why it waits on the maintainer, a line the board's ticket carries
    ///
    /// Set with [`Status::WaitingOnMaintainer`] and cleared with any other
    /// state. A record from before reasons had none.
    #[serde(default)]
    pub reason: Option<String>,
}

impl Record {
    /// A new record for `ticket` on `branch`: its turn is next to start, and it
    /// holds no slot yet
    #[must_use]
    pub fn new(ticket: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            ticket: ticket.into(),
            branch: branch.into(),
            session: None,
            status: Status::Running,
            seat: Seat::Without,
            notes: Vec::new(),
            reason: None,
        }
    }

    /// As the slots see it
    #[must_use]
    pub fn worker(&self) -> Worker {
        Worker {
            status: self.status,
            seat: self.seat,
        }
    }
}

/// A project's worker records, and the file that keeps them
#[derive(Debug)]
pub struct Records {
    file: PathBuf,
    list: Vec<Record>,
    /// The clock of each ticket's latest turn, which is never written: a launch
    /// has no turn running
    watches: HashMap<String, Watch>,
    /// Where each ticket's turns send their events, which is never written
    events: HashMap<String, Events>,
    /// What Stop and tell said to a ticket whose cancelled turn has not ended yet,
    /// which the turn's end sends as the next prompt. Never written: a launch has no
    /// turn cancelling.
    redirects: HashMap<String, String>,
}

impl Records {
    /// The records `file` holds, none when it is not there yet
    ///
    /// # Errors
    ///
    /// The file cannot be read, or is not a list of records.
    pub fn load(file: PathBuf) -> io::Result<Self> {
        let list = match fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is not a list of workers: {error}", file.display()),
                )
            })?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        Ok(Self {
            file,
            list,
            watches: HashMap::new(),
            events: HashMap::new(),
            redirects: HashMap::new(),
        })
    }

    /// The file the records are kept in
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The records, in the order the project started its workers
    #[must_use]
    pub fn list(&self) -> &[Record] {
        &self.list
    }

    /// The record of `ticket`, `None` when the project has no worker on it
    #[must_use]
    pub fn get(&self, ticket: &str) -> Option<&Record> {
        self.list.iter().find(|record| record.ticket == ticket)
    }

    /// Retire `ticket`'s last turn clock, so its ceiling never acts on a record
    /// a later start of the ticket owns
    ///
    /// A turn whose harness exited ends with no `TurnEnded`, so its clock would
    /// otherwise run on and pause the worker a new start began.
    pub(super) fn retire_watch(&mut self, ticket: &str) {
        if let Some(watch) = self.watches.remove(ticket) {
            watch.retire();
        }
    }

    /// Keep `watch` as the clock of `ticket`'s running turn, retiring the last one
    pub(super) fn keep_watch(&mut self, ticket: &str, watch: Watch) {
        self.retire_watch(ticket);
        self.watches.insert(ticket.to_owned(), watch);
    }

    /// `ticket`'s latest turn clock, `None` before its first turn this launch
    pub(super) fn watch_of(&self, ticket: &str) -> Option<Watch> {
        self.watches.get(ticket).cloned()
    }

    /// Keep `events` as where `ticket`'s turns send what they say, so a turn
    /// that starts later reaches the caller that started the worker
    pub(super) fn keep_events(&mut self, ticket: &str, events: Events) {
        self.events.insert(ticket.to_owned(), events);
    }

    /// Where `ticket`'s turns send their events, `None` before its first turn this launch
    pub(super) fn events_of(&self, ticket: &str) -> Option<Events> {
        self.events.get(ticket).cloned()
    }

    /// Keep `text` as what `ticket`'s cancelled turn's end sends next, after
    /// whatever was kept before it, one to a paragraph
    pub(super) fn redirect(&mut self, ticket: &str, text: &str) {
        self.redirects
            .entry(ticket.to_owned())
            .and_modify(|kept| {
                kept.push_str("\n\n");
                kept.push_str(text);
            })
            .or_insert_with(|| text.to_owned());
    }

    /// Take what `ticket`'s cancelled turn's end sends next
    pub(super) fn take_redirect(&mut self, ticket: &str) -> Option<String> {
        self.redirects.remove(ticket)
    }

    /// Add `text` to `ticket`'s notes and write the file, `false` when it has no record
    ///
    /// # Errors
    ///
    /// The file cannot be written. The note is not kept then.
    pub fn add_note(&mut self, ticket: &str, text: &str) -> io::Result<bool> {
        let Some(record) = self.list.iter_mut().find(|record| record.ticket == ticket) else {
            return Ok(false);
        };
        record.notes.push(text.to_owned());
        if let Err(error) = write(&self.file, &self.list) {
            if let Some(record) = self.list.iter_mut().find(|record| record.ticket == ticket) {
                record.notes.pop();
            }
            return Err(error);
        }
        Ok(true)
    }

    /// Take `ticket`'s notes, oldest first, and write the file
    ///
    /// # Errors
    ///
    /// The file cannot be written. The notes stay in the record then.
    pub fn take_notes(&mut self, ticket: &str) -> io::Result<Vec<String>> {
        let Some(record) = self.list.iter_mut().find(|record| record.ticket == ticket) else {
            return Ok(Vec::new());
        };
        if record.notes.is_empty() {
            return Ok(Vec::new());
        }
        let notes = std::mem::take(&mut record.notes);
        if let Err(error) = write(&self.file, &self.list) {
            if let Some(record) = self.list.iter_mut().find(|record| record.ticket == ticket) {
                record.notes = notes;
            }
            return Err(error);
        }
        Ok(notes)
    }

    /// Keep `record`, in its place when its ticket is already listed and at the
    /// end when it is not, and write the file
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn keep(&mut self, record: Record) -> io::Result<()> {
        match self
            .list
            .iter_mut()
            .find(|held| held.ticket == record.ticket)
        {
            Some(held) => *held = record,
            None => self.list.push(record),
        }
        write(&self.file, &self.list)
    }

    /// Drop `ticket`'s record and write the file, `false` when it had none
    ///
    /// For a start that held a slot and never began a worker: nothing runs for
    /// the ticket and nothing is queued for it, so it is listed nowhere.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn remove(&mut self, ticket: &str) -> io::Result<bool> {
        let Some(at) = self.list.iter().position(|record| record.ticket == ticket) else {
            return Ok(false);
        };
        self.list.remove(at);
        write(&self.file, &self.list)?;
        Ok(true)
    }

    /// Keep `status` as `ticket`'s state, give its slot back when the new state
    /// needs none, and write the file when either changed
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn set_status(&mut self, ticket: &str, status: Status) -> io::Result<()> {
        let Some(record) = self.list.iter_mut().find(|record| record.ticket == ticket) else {
            return Ok(());
        };
        if record.status == status {
            return Ok(());
        }
        record.status = status;
        if !status.needs_slot() {
            record.seat = Seat::Without;
        }
        write(&self.file, &self.list)
    }

    /// Keep `reason` as why `ticket` waits on the maintainer, and write the file when it changed
    ///
    /// It goes with the status: the caller sets both under one hold of the records'
    /// lock, so a reader never finds a held worker with no reason or a running one
    /// with a stale one.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn set_reason(&mut self, ticket: &str, reason: Option<String>) -> io::Result<()> {
        let Some(record) = self.list.iter_mut().find(|record| record.ticket == ticket) else {
            return Ok(());
        };
        if record.reason == reason {
            return Ok(());
        }
        record.reason = reason;
        write(&self.file, &self.list)
    }

    /// Keep `session` as the id `ticket`'s harness gave, and write the file when it changed
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn set_session(&mut self, ticket: &str, session: &SessionId) -> io::Result<()> {
        let Some(record) = self.list.iter_mut().find(|record| record.ticket == ticket) else {
            return Ok(());
        };
        if record.session.as_ref() == Some(session) {
            return Ok(());
        }
        record.session = Some(session.clone());
        write(&self.file, &self.list)
    }

    /// Give each record its seat under `max`, oldest first, and write the file when any changed
    ///
    /// See [`super::slots::seat`]. A record that is Paused or ended
    /// gives its slot back, and one that needs a slot takes a freed one ahead of
    /// any ticket started after it.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn seat(&mut self, max: usize) -> io::Result<bool> {
        let mut workers: Vec<Worker> = self.list.iter().map(Record::worker).collect();
        let changed = slots::seat(&mut workers, max);
        if !changed {
            return Ok(false);
        }
        for (record, worker) in self.list.iter_mut().zip(&workers) {
            record.seat = worker.seat;
        }
        write(&self.file, &self.list)?;
        Ok(true)
    }

    /// Mark every worker kelpie was mid-turn with as Paused, and write the file
    ///
    /// For the launch after a quit, crash or kill: the process went with the run
    /// that started it (ADR 0001), and a record that held a slot was mid-turn or
    /// next to start. Each becomes [`Status::Paused`],
    /// which holds no slot and resumes only when the maintainer says so. A ticket
    /// that was waiting for a slot had nothing running and stays queued.
    ///
    /// Returns the tickets paused, in the order they were started.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn pause_mid_turn(&mut self) -> io::Result<Vec<String>> {
        let mut paused = Vec::new();
        for record in &mut self.list {
            if record.status == Status::Running && record.seat == Seat::Held {
                record.status = Status::Paused;
                record.seat = Seat::Without;
                paused.push(record.ticket.clone());
            }
        }
        if paused.is_empty() {
            return Ok(paused);
        }
        write(&self.file, &self.list)?;
        Ok(paused)
    }
}

/// Write `list` beside `file` and rename it into place, so a crash leaves the
/// old list or the new one
fn write(file: &Path, list: &[Record]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder)?;
    }
    let json = serde_json::to_vec_pretty(list).map_err(io::Error::other)?;
    let partial = file.with_extension("json.partial");
    fs::write(&partial, json)?;
    fs::rename(&partial, file)
}
