//! The last lines a harness wrote to stderr, for the message when it exits

use std::collections::VecDeque;
use std::io::{BufRead as _, BufReader, Read};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Duration;

/// Lines kept
const LINES: usize = 8;
/// Characters kept of each line
const WIDTH: usize = 300;

/// Reads a pipe on a thread of its own until it closes, keeping the last lines
#[derive(Debug, Clone, Default)]
pub(super) struct Tail(Arc<Inner>);

#[derive(Debug, Default)]
struct Inner {
    kept: Mutex<Kept>,
    /// Signalled when the pipe closes
    closed: Condvar,
}

#[derive(Debug, Default)]
struct Kept {
    lines: VecDeque<String>,
    /// The thread read to the end of the pipe
    done: bool,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Tail {
    /// Start reading `pipe`
    ///
    /// A harness that writes to a pipe nobody reads stops once it fills, so
    /// the thread reads until the pipe closes.
    pub(super) fn collect(pipe: impl Read + Send + 'static) -> Self {
        let tail = Self::default();
        let inner = Arc::clone(&tail.0);
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
                    let mut kept = inner.lock();
                    if kept.lines.len() == LINES {
                        kept.lines.pop_front();
                    }
                    kept.lines.push_back(text);
                }
                inner.lock().done = true;
                inner.closed.notify_all();
            });
        tail
    }

    /// Wait up to `within` for the pipe to close, so the lines are all of them
    ///
    /// A harness that has exited has written everything it will, but the thread
    /// reading it may not have caught up. A grandchild holding the pipe open
    /// makes this wait the whole `within`.
    pub(super) fn settle(&self, within: Duration) {
        let kept = self.0.lock();
        let _ = self
            .0
            .closed
            .wait_timeout_while(kept, within, |kept| !kept.done);
    }

    /// The lines kept, joined, oldest first
    pub(super) fn lines(&self) -> String {
        let kept = self.0.lock();
        kept.lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" / ")
    }
}
