//! The usage ledger: one line for each of a project's worker turns
//!
//! `<data_dir>/projects/<id>/usage.jsonl` holds one JSON object per line,
//! appended as each turn ends and never rewritten. A line says whose turn it was,
//! what it ran on, how it ended, and the tokens its harness reported.
//!
//! A turn's line is written once, by whoever takes its draft first
//! ([`InFlight`]). Drafts live in memory: a crash writes no line for the turns it
//! cut off, since only a quit writes those.
//!
//! Lifted from SK's `core/usage/`, with its cost and unit weighting, its pacer
//! readings and its per-harness transcript parsers left behind: ACP reports the
//! tokens of every turn (`docs/lift-plan.md`, usage).

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use kelpie_protocol::Usage;
use serde::{Deserialize, Serialize};

use crate::session::SessionId;
use crate::settings::Harness;

pub mod in_flight;

pub use in_flight::{InFlight, TurnId};

#[cfg(test)]
mod tests;

/// The ledger's file name in a project's folder
pub const FILE: &str = "usage.jsonl";

/// A turn's start or end, as whole seconds since the Unix epoch
// wire format: the ledger holds these numbers
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The moment as whole seconds since the Unix epoch
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    /// This moment, as the system's clock reads it
    #[must_use]
    pub fn now() -> Self {
        Self(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        )
    }

    /// The seconds themselves
    #[must_use]
    pub const fn secs(self) -> u64 {
        self.0
    }
}

/// Who a turn was made for
// wire format: the ledger holds these names
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive] // kelpie's other roles arrive with the slices that need them
pub enum Role {
    /// A worker's turn on a ticket
    Worker,
}

/// How a turn ended
// wire format: the ledger holds these names
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive] // a turn can end in a way kelpie has no word for yet
pub enum Ended {
    /// Its harness ended its turn by itself
    Done,

    /// Its turn was cancelled, at its ceiling or by the maintainer
    Cancelled,

    /// Its harness failed the turn and reported no usage
    Failed,

    /// kelpie stopped while its turn ran
    Stopped,
}

/// A worker's turn as it starts: whose it is, what it runs on, and when
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// The ticket it works on
    pub ticket: String,

    /// Who it was made for
    pub role: Role,

    /// The harness its setting names, `None` where no setting names one
    pub harness: Option<Harness>,

    /// The model it was asked for, `None` for the harness's own default
    pub model: Option<String>,

    /// How hard it was asked to think, `None` where its harness offers no effort
    pub effort: Option<String>,

    /// The session it runs in, `None` until its harness gives it an id
    pub session: Option<SessionId>,

    /// When its turn started
    pub started: Timestamp,
}

impl Draft {
    /// The line of a turn that ended at `at` as `ended`, having spent `spent`
    #[must_use]
    pub fn line(&self, at: Timestamp, ended: Ended, spent: Option<Usage>) -> Line {
        Line {
            ticket: self.ticket.clone(),
            role: self.role,
            harness: self.harness,
            model: self.model.clone(),
            effort: self.effort.clone(),
            session: self.session.clone(),
            start: self.started,
            end: at,
            ended,
            seconds: at.secs().saturating_sub(self.started.secs()),
            usage: spent.unwrap_or(Usage {
                input: 0,
                output: 0,
                cache_read: 0,
                cache_write: 0,
                cache_lifetime: None,
            }),
        }
    }
}

/// One line of the ledger: a worker's turn as it ended
// wire format: changing this is a breaking change to the usage ledger
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive] // the ledger grows what a turn spent as its readers do
pub struct Line {
    /// The ticket it worked on
    pub ticket: String,

    /// Who it was made for
    pub role: Role,

    /// The harness its setting named, `None` where no setting named one
    pub harness: Option<Harness>,

    /// The model it was asked for, `None` for the harness's own default
    pub model: Option<String>,

    /// How hard it was asked to think, `None` where its harness offered no effort
    pub effort: Option<String>,

    /// The session it ran in, `None` where its harness never gave it an id
    pub session: Option<SessionId>,

    /// When its turn started
    pub start: Timestamp,

    /// When its turn ended
    pub end: Timestamp,

    /// How it ended
    pub ended: Ended,

    /// Seconds from its start to its end
    pub seconds: u64,

    /// The tokens it used, zeros for each its harness did not report
    pub usage: Usage,
}

/// Append `line` as one JSON line to the file at `path`
///
/// The file is appended and never rewritten, so each turn's end and a quit can
/// both add to it without reading what the other wrote.
///
/// # Errors
///
/// The [`io::Error`] that opening or writing the file failed with.
pub fn append_to(path: &Path, line: &Line) -> io::Result<()> {
    if let Some(folder) = path.parent() {
        fs::create_dir_all(folder)?;
    }
    let mut bytes = serde_json::to_vec(line).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let mut file = (OpenOptions::new().create(true).append(true)).open(path)?;
    file.write_all(&bytes)
}
