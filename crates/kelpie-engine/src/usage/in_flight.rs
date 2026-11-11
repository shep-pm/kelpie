//! The turns open now, shared outside the records' lock
//!
//! Each turn's draft goes in as its turn starts and comes out when its end is
//! heard, which writes its line. A kelpie that stops hears no more ends, so
//! [`InFlight::stop_all`] takes every draft left and writes each a
//! [`Stopped`](super::Ended::Stopped) line. Whichever takes a draft first writes
//! its line, so none is written twice.
//!
//! The drafts are held behind a lock of their own, which is never taken while the
//! records' lock is held, so a quit writes while a turn's end holds that.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use kelpie_protocol::Usage;

use super::{Draft, Ended, Line, Timestamp, append_to};
use crate::session::SessionId;

/// Which turn a draft belongs to
///
/// A ticket has one turn running at a time, but the turn its notes start opens
/// its draft before the one it follows is dropped, so a draft is keyed by its
/// turn and never by its ticket.
// not a wire format: a turn's id lives only while kelpie runs
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnId(u64);

/// What a project's turns are metered against
#[derive(Debug)]
struct Shared {
    /// The file each line is appended to
    file: PathBuf,
    /// The id the next draft gets
    next: AtomicU64,
    /// Each turn running now, oldest first
    drafts: Mutex<BTreeMap<TurnId, Draft>>,
    /// Whether a quit has taken every draft left
    stopping: AtomicBool,
}

/// The drafts open now, shared between a project's turns and a quit
#[derive(Debug, Clone)]
pub struct InFlight(Arc<Shared>);

impl InFlight {
    /// The drafts open now, each line appended to the file at `file`
    #[must_use]
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self(Arc::new(Shared {
            file: file.into(),
            next: AtomicU64::default(),
            drafts: Mutex::new(BTreeMap::new()),
            stopping: AtomicBool::new(false),
        }))
    }

    /// Keep `draft` as a turn running now, and answer which turn it is
    ///
    /// A draft is kept nowhere but memory, so a turn that started after a quit
    /// took every draft left has none: its line is never written.
    pub fn open(&self, draft: Draft) -> TurnId {
        let id = TurnId(self.0.next.fetch_add(1, Ordering::Relaxed));
        if !self.0.stopping.load(Ordering::Relaxed) {
            lock(&self.0.drafts).insert(id, draft);
        }
        id
    }

    /// Keep `session` as the id `turn`'s harness gave its session
    pub fn set_session(&self, turn: TurnId, session: &SessionId) {
        if let Some(draft) = lock(&self.0.drafts).get_mut(&turn) {
            draft.session = Some(session.clone());
        }
    }

    /// Take `turn`'s draft and write its line, `None` when none is open for it
    ///
    /// The draft is taken whether or not the line is written, so a turn whose
    /// file cannot be written does not write it again.
    ///
    /// # Errors
    ///
    /// The [`io::Error`] that opening or writing the file failed with.
    pub fn end(
        &self,
        turn: TurnId,
        at: Timestamp,
        ended: Ended,
        spent: Option<Usage>,
    ) -> io::Result<Option<Line>> {
        let draft = lock(&self.0.drafts).remove(&turn);
        let Some(draft) = draft else {
            return Ok(None);
        };
        let line = draft.line(at, ended, spent);
        append_to(&self.0.file, &line)?;
        Ok(Some(line))
    }

    /// Take every draft left and write each a [`Stopped`](Ended::Stopped) line at `at`
    ///
    /// No draft is kept after this, so a second call writes none, and a turn
    /// whose end is heard after it writes none either. Returns the lines
    /// written, oldest turn first.
    ///
    /// A line that cannot be written is named on stderr and let go, so one
    /// project's file costs the quit nothing.
    pub fn stop_all(&self, at: Timestamp) -> Vec<Line> {
        let left = {
            let mut drafts = lock(&self.0.drafts);
            self.0.stopping.store(true, Ordering::Relaxed);
            std::mem::take(&mut *drafts)
        };
        let mut written = Vec::new();
        for draft in left.into_values() {
            let line = draft.line(at, Ended::Stopped, None);
            if let Err(error) = append_to(&self.0.file, &line) {
                eprintln!(
                    "kelpie: {} wrote no line for ticket {}: {error}",
                    self.0.file.display(),
                    draft.ticket
                );
                continue;
            }
            written.push(line);
        }
        written
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
