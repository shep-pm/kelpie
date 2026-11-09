//! The last lines a harness wrote to stderr, for the message when it exits

use std::collections::VecDeque;
use std::io::{BufRead as _, BufReader, Read};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

/// Lines kept
const LINES: usize = 8;
/// Characters kept of each line
const WIDTH: usize = 300;

/// Reads a pipe on a thread of its own until it closes, keeping the last lines
#[derive(Debug, Clone, Default)]
pub(super) struct Tail(Arc<Mutex<VecDeque<String>>>);

impl Tail {
    /// Start reading `pipe`
    ///
    /// A harness that writes to a pipe nobody reads stops once it fills, so
    /// the thread reads until the pipe closes.
    pub(super) fn collect(pipe: impl Read + Send + 'static) -> Self {
        let tail = Self::default();
        let kept = Arc::clone(&tail.0);
        // A thread that cannot start leaves the tail empty, and the pipe unread
        let _ = thread::Builder::new()
            .name("kelpie-stderr".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(pipe);
                let mut line = Vec::new();
                while reader
                    .read_until(b'\n', &mut line)
                    .is_ok_and(|read| read > 0)
                {
                    let text = String::from_utf8_lossy(&line);
                    let text: String = text.trim_end().chars().take(WIDTH).collect();
                    line.clear();
                    if text.is_empty() {
                        continue;
                    }
                    let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
                    if kept.len() == LINES {
                        kept.pop_front();
                    }
                    kept.push_back(text);
                }
            });
        tail
    }

    /// The lines kept, joined, oldest first
    pub(super) fn lines(&self) -> String {
        let kept = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        kept.iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" / ")
    }
}
