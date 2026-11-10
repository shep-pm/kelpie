//! Kelpie's own log of a session: one JSON line per [`LogEntry`]
//!
//! [`Log`] appends every message the maintainer sent and every event the session
//! streamed, so history shows on relaunch on any harness and starts no process.
//! The harness's own transcript is only used to resume (`docs/design/pm-chat.md`,
//! History). [`read`] gives the entries back.
//!
//! [`Log::close`] ends the writing for good: a quit stops the harness, and what
//! that does to the session is not the maintainer's turn, so a turn a quit cut
//! off stays without its end and relaunches as interrupted.
//!
//! A line is written whole and flushed before the next, so a crash can leave a
//! torn last line and nothing else. [`read`] drops it, and so does [`Log::open`]
//! before it appends after it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use kelpie_protocol::{LogEntry, SessionEvent};

use super::Sink;

/// A log file open for appending, shared by every clone
#[derive(Debug, Clone)]
pub struct Log {
    /// `None` once closed
    file: Arc<Mutex<Option<File>>>,
}

impl Log {
    /// Open the log at `path` for appending, making it and its folder when they are not there
    ///
    /// A torn last line left by a crash is cut off first, so the next entry
    /// starts a line of its own. Lines before it are not read, so a log with a
    /// bad one in the middle still opens: [`read`] is what names it.
    ///
    /// # Errors
    ///
    /// The folder or the file cannot be made, read or written.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(folder) = path.parent() {
            fs::create_dir_all(folder)?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(path)?;
        let bytes = fs::read(path)?;
        let whole = whole_len(&bytes);
        if whole < bytes.len() {
            file.set_len(u64::try_from(whole).map_err(io::Error::other)?)?;
        }
        if whole > 0 && bytes.get(whole - 1) != Some(&b'\n') {
            file.write_all(b"\n")?;
        }
        Ok(Self {
            file: Arc::new(Mutex::new(Some(file))),
        })
    }

    /// Append `entry` as one line, and flush it
    ///
    /// # Errors
    ///
    /// The entry cannot be written.
    pub fn append(&self, entry: &LogEntry) -> io::Result<()> {
        let mut line = serde_json::to_vec(entry).map_err(io::Error::other)?;
        line.push(b'\n');
        let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(file) = file.as_mut() else {
            return Ok(());
        };
        file.write_all(&line)?;
        file.flush()
    }

    /// Stop writing, for every clone: later entries are dropped without an error
    pub fn close(&self) {
        self.file
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

/// The entries the log at `path` holds, oldest first
///
/// A log that is not there is an empty history, and a torn last line is
/// dropped. Reading starts no process.
///
/// # Errors
///
/// The file cannot be read, or a line before the last is not an entry.
pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<LogEntry>> {
    match fs::read(path) {
        Ok(bytes) => parse(&bytes).map(|(entries, _)| entries),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// How many bytes of `bytes` end with a whole last line
///
/// Only the last line is looked at: it is whole when it is an entry, and
/// otherwise the crash tore it, so it ends where the line before did.
fn whole_len(bytes: &[u8]) -> usize {
    let content = bytes.trim_ascii_end().len();
    if content == 0 {
        return 0;
    }
    let start = bytes[..content]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    if serde_json::from_slice::<LogEntry>(&bytes[start..content]).is_err() {
        return start;
    }
    match bytes[content..].iter().position(|byte| *byte == b'\n') {
        Some(at) => content + at + 1,
        None => content,
    }
}

/// The entries in `bytes`, and how many bytes up to the end of the last good line
fn parse(bytes: &[u8]) -> io::Result<(Vec<LogEntry>, usize)> {
    let mut entries = Vec::new();
    let mut whole = 0;
    let mut end = 0;
    for (number, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        end += line.len();
        let body = line.trim_ascii();
        if body.is_empty() {
            continue;
        }
        match serde_json::from_slice(body) {
            Ok(entry) => {
                entries.push(entry);
                whole = end;
            }
            // The last line is the one a crash tears
            Err(_) if bytes[end..].trim_ascii().is_empty() => break,
            Err(error) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "line {} of the session log is not an entry: {error}",
                        number + 1
                    ),
                ));
            }
        }
    }
    Ok((entries, whole))
}

/// `sink`, with `text` and every event it is given written to `log` first
///
/// A write that fails does not stop the turn: the event still reaches `sink`,
/// and one `Failed` after it says the log is not being kept.
pub(super) fn logged(
    log: &Log,
    text: &str,
    mut sink: impl FnMut(SessionEvent) + Send + 'static,
) -> Sink {
    let log = log.clone();
    let mut failure = log
        .append(&LogEntry::User {
            text: text.to_owned(),
        })
        .err();
    let mut reported = false;
    Box::new(move |event| {
        if let Err(error) = log.append(&LogEntry::Event(event.clone())) {
            failure.get_or_insert(error);
        }
        sink(event);
        if !reported && let Some(error) = failure.take() {
            reported = true;
            sink(SessionEvent::Failed {
                message: format!("could not write the session log: {error}"),
            });
        }
    })
}
