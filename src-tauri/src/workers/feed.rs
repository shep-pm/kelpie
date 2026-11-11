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
//! sink (`session/log.rs`, `logged`), so an event can be in the log and not yet
//! forwarded when the watcher reads it. The feed counts the events it has
//! forwarded and the events the log held when it began, so it knows how many
//! the read returned that are still on their way, and the new channel skips
//! exactly those. An event is in the entries or on the channel, never both.
//!
//! The feed also keeps the sandbox requests its worker waits on, which the
//! board's ticket carries as a needs-you line (`docs/design/tickets.md`, The
//! board). A request comes in with its [`SessionEvent::SandboxRequest`] and goes
//! out when it is answered ([`Feed::answered`]), or when a turn ends or fails,
//! since a turn that ends answers it denied.
//!
//! A channel's send succeeds while its webview lives, closed tab or not, so a
//! failed send is not how a tab is forgotten: the window says so with
//! [`Feed::unwatch`], and a channel watching again replaces its earlier watch.

use std::sync::{Mutex, MutexGuard, PoisonError};

use kelpie_protocol::{LogEntry, SessionEvent};

/// What a watching channel is: it takes an event, and answers whether it was sent
type Sender = Box<dyn Fn(SessionEvent) -> bool + Send>;

/// One channel watching a worker
struct Watcher {
    /// The channel's own id, which names it to [`Feed::unwatch`]
    id: u32,

    send: Sender,

    /// How many of the events forwarded next are in the entries it was given
    skip: usize,
}

#[derive(Default)]
struct Inner {
    watchers: Vec<Watcher>,

    /// The sandbox requests waiting on the maintainer, as (id, host), oldest first
    requests: Vec<(String, String)>,

    /// The events the log held when the feed began, plus the ones forwarded since
    seen: usize,
}

/// The channels watching one worker
#[derive(Default)]
pub struct Feed {
    inner: Mutex<Inner>,
}

// The watchers are closures, which no derive can print
impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed")
            .field("watching", &self.lock().watchers.len())
            .finish()
    }
}

/// How many of `entries` are events
fn events_in(entries: &[LogEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| matches!(entry, LogEntry::Event(_)))
        .count()
}

impl Feed {
    /// A feed no channel watches, for a worker whose log holds `logged` events
    ///
    /// Made before the worker's session begins, so none of them is on its way.
    #[must_use]
    pub fn new(logged: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                watchers: Vec::new(),
                requests: Vec::new(),
                seen: logged,
            }),
        }
    }

    /// How many events `entries` holds, for [`Feed::new`]
    #[must_use]
    pub fn logged(entries: &[LogEntry]) -> usize {
        events_in(entries)
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Send `event` to every channel watching, and drop each one whose send fails
    ///
    /// A channel that skips events because its entries held them does not get this one.
    pub fn forward(&self, event: SessionEvent) {
        let mut inner = self.lock();
        inner.seen += 1;
        match &event {
            SessionEvent::SandboxRequest { id, host, .. } => {
                inner.requests.push((id.clone(), host.clone()));
            }
            // A turn that ends, or a session that fails, answers each request denied
            SessionEvent::TurnEnded { .. } | SessionEvent::Failed { .. } => {
                inner.requests.clear();
            }
            _ => {}
        }
        inner.watchers.retain_mut(|watcher| {
            if watcher.skip > 0 {
                watcher.skip -= 1;
                return true;
            }
            (watcher.send)(event.clone())
        });
    }

    /// The hosts the worker's sandbox requests wait on the maintainer for, oldest first
    #[must_use]
    pub fn waiting(&self) -> Vec<String> {
        let inner = self.lock();
        inner
            .requests
            .iter()
            .map(|(_, host)| host.clone())
            .collect()
    }

    /// Forget the sandbox request `id`, which the maintainer answered
    pub fn answered(&self, id: &str) {
        self.lock().requests.retain(|(asked, _)| asked != id);
    }

    /// Read the worker's past with `read`, then attach the channel `id` to follow it live
    ///
    /// Both happen under the lock [`Feed::forward`] takes, so no event is
    /// forwarded between them: it is in what `read` returned, or `send` gets it.
    /// A channel already watching under `id` is replaced.
    ///
    /// # Errors
    ///
    /// `read`'s own: the channel is not attached then.
    pub fn watch<E>(
        &self,
        id: u32,
        read: impl FnOnce() -> Result<Vec<LogEntry>, E>,
        send: impl Fn(SessionEvent) -> bool + Send + 'static,
    ) -> Result<Vec<LogEntry>, E> {
        let mut inner = self.lock();
        let entries = read()?;
        let skip = events_in(&entries).saturating_sub(inner.seen);
        inner.watchers.retain(|watcher| watcher.id != id);
        inner.watchers.push(Watcher {
            id,
            send: Box::new(send),
            skip,
        });
        Ok(entries)
    }

    /// Stop sending to the channel `id`, which a tab does when it closes
    pub fn unwatch(&self, id: u32) {
        self.lock().watchers.retain(|watcher| watcher.id != id);
    }
}
