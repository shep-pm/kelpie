//! The facts the review keeps about each ticket's pass between steps
//!
//! A step decides from what the pass kept and from what it just read of the pull request.
//! [`Kept`] is what one ticket leaves, and [`Book`] holds every ticket's in `review.json`, in
//! the project's folder beside `workers.json` and `gate.json`, written whole each time. SK kept
//! these on its work item (`core/work_item/`: `review`, `disputes`, `heads`).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Dispute, Heads, Review, TestCheck};
use crate::ticket::TicketId;

/// The file in a project's folder the review's facts are kept in
pub const FILE: &str = "review.json";

/// How a pass ended short of the reads it owed
// wire format: `review.json` holds this
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Short {
    /// Who was missed, none when no reviewer was listed to read at all
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

/// What the review keeps about one ticket's pull request
// wire format: `review.json` holds this
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kept {
    /// Where the pass stands, none before it starts
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,

    /// The reviewers this pass passed over after their calls kept failing
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passed_over: Vec<String>,

    /// What the worker disputed in this pass, with its reason
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disputes: Vec<Dispute>,

    /// The runs of the reviewers' failing tests
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<TestCheck>,

    /// The heads read, vouched for and fixed for nits alone
    #[serde(default)]
    pub heads: Heads,

    /// How the pass ended, when it ended short
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short: Option<Short>,
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
                    format!("{} is not the review's facts: {error}", file.display()),
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

    /// What is kept about `ticket`, none before a pass first started
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
