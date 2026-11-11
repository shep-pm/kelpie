//! A worker's feed: the channels watching it, and what is forwarded to them
//!
//! A closed tab hides a worker and does not stop it, so the window reads a
//! worker's past from its log and follows it live from any moment
//! (`docs/design/tickets.md`, The worker's tab). [`Feed`] is the live half: the
//! worker's session sink forwards each [`SessionEvent`] into it, and it sends the
//! event on to every channel watching.
//!
//! [`Feed::watch`] reads the past and attaches the channel under the lock
//! [`Feed::forward`] holds while it sends, so an event forwarded meanwhile waits
//! for the attach. The session writes an event to the log before it reaches the
//! sink (`session/log.rs`, `logged`), so an event is in the entries read or sent
//! on the channel. One forwarded after the log wrote it and before the watcher
//! took the lock is in both: the window tells them apart by their order, since
//! the entries come first.

use std::sync::{Mutex, MutexGuard, PoisonError};

use kelpie_protocol::SessionEvent;

/// What a watching channel is: it takes an event, and answers whether it was sent
type Watcher = Box<dyn Fn(SessionEvent) -> bool + Send>;

/// The channels watching one worker
#[derive(Default)]
pub struct Feed {
    watchers: Mutex<Vec<Watcher>>,
}

// The watchers are closures, which no derive can print
impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed")
            .field("watching", &self.lock().len())
            .finish()
    }
}

impl Feed {
    /// A feed no channel watches
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Watcher>> {
        self.watchers.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Send `event` to every channel watching, and drop each one whose send fails
    ///
    /// A closed tab's channel fails its next send, which is how it is forgotten.
    pub fn forward(&self, event: SessionEvent) {
        self.lock().retain(|send| send(event.clone()));
    }

    /// Read the worker's past with `read`, then attach `send` to follow it live
    ///
    /// Both happen under the lock [`Feed::forward`] takes, so no event is
    /// forwarded between them: it is in what `read` returned, or `send` gets it.
    ///
    /// # Errors
    ///
    /// `read`'s own: the channel is not attached then.
    pub fn watch<T, E>(
        &self,
        read: impl FnOnce() -> Result<T, E>,
        send: impl Fn(SessionEvent) -> bool + Send + 'static,
    ) -> Result<T, E> {
        let mut watchers = self.lock();
        let past = read()?;
        watchers.push(Box::new(send));
        Ok(past)
    }
}
