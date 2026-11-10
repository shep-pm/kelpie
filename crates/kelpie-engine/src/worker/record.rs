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

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
        Ok(Self { file, list })
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
