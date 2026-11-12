//! A ticket's reads: the channels watching them, and what is forwarded to them
//!
//! A read runs off the poll thread, and its tab opens when the maintainer clicks a card
//! that says a reviewer is reading, so the window reads the ticket's reads from their logs
//! and follows the one in flight from any moment (`docs/design/review.md`, What the
//! maintainer sees). [`Feed`] is the live half: the read's sink forwards each
//! [`SessionEvent`] into it, and it sends the event on to every channel watching.
//!
//! One [`Feed`] per ticket, so a second look follows the first in the same tab. Each read
//! keeps a log of its own ([`Paths::review_log_file`]), and a watch replays them in the
//! order the reads began with a line naming each read after the first. That line is the
//! divider the tab shows between two reads.
//!
//! A read is named by the look seated on its own round ([`Reads::seated`]), the round being
//! the first with no log yet, which is the one its session opens its log on. A read that
//! never started leaves its round free, so the line above a log always names the read that
//! wrote it, and a feed made after a relaunch names the reads it was told about by the logs
//! they left rather than by their order in the feed.
//!
//! The counting is the worker feed's ([`crate::workers::Feed`]): a session writes an event
//! to its log before it reaches the read's sink, so an event can be in a log and not yet
//! forwarded when a watcher reads it. The feed counts the events it has forwarded and the
//! ones the logs held when it began, so it knows how many the read returned that are still
//! on their way, and the new channel skips exactly those. An event is in the entries or on
//! the channel, never both.
//!
//! A closed tab says so with [`Feed::unwatch`]: a channel's send succeeds while the webview
//! lives, closed tab or not. Nothing that watches a read can stop it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use kelpie_engine::session::log;
use kelpie_engine::worker::Paths;
use kelpie_protocol::{LogEntry, SessionEvent};
use tauri::State;
use tauri::ipc::Channel;

use super::Review;
use crate::state::AppState;

/// What a watching channel is: it takes an event, and answers whether it was sent
type Sender = Box<dyn Fn(SessionEvent) -> bool + Send>;

/// One channel watching a ticket's reads
struct Watcher {
    /// The channel's own id, which names it to [`Feed::unwatch`]
    id: u32,

    send: Sender,

    /// How many of the events forwarded next are in the entries it was given
    skip: usize,
}

/// One read the feed was told about, held by the round it was seated on
#[derive(Debug, Clone)]
struct Look {
    /// The reviewer, as the project lists it
    name: String,

    /// Whether it shows the reviewer what its first look found
    second: bool,
}

#[derive(Default)]
struct Inner {
    watchers: Vec<Watcher>,

    /// Each read the feed was told about, by the round it was seated on
    ///
    /// A round is a read's own number, the log it writes. A read seated on one and never run
    /// leaves it free, and the read that takes the round takes its look with it.
    looks: BTreeMap<u32, Look>,

    /// The events the read logs held when the feed began, plus the ones forwarded since
    seen: usize,
}

/// The channels watching one ticket's reads
#[derive(Default)]
pub struct Feed {
    inner: Mutex<Inner>,
}

// The watchers are closures, which no derive can print
impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.lock();
        f.debug_struct("Feed")
            .field("watching", &inner.watchers.len())
            .field("reads", &inner.looks.len())
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

/// What the line above a read says
///
/// A read that ran before this feed existed has no look seated on its round, so its line
/// counts it instead of naming it.
fn named(look: Option<&Look>, round: u32) -> String {
    match look {
        Some(look) if look.second => format!("{} reads again", look.name),
        Some(look) => format!("{} reads", look.name),
        None => format!("Read {round}"),
    }
}

impl Feed {
    /// A feed no channel watches, for a ticket whose read logs hold `logged` events
    ///
    /// Made before a read's session begins, so none of them is on its way.
    #[must_use]
    pub fn new(logged: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                watchers: Vec::new(),
                looks: BTreeMap::new(),
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
        inner.watchers.retain_mut(|watcher| {
            if watcher.skip > 0 {
                watcher.skip -= 1;
                return true;
            }
            (watcher.send)(event.clone())
        });
    }

    /// A read by `name` is seated on the round `round`, whose log it writes
    ///
    /// `second` is a second look, which the line above it says. A read seated here and never
    /// run leaves its look on its round, and the read that takes that round takes its look,
    /// so the line naming a read always says who wrote its log.
    pub fn began(&self, round: u32, name: &str, second: bool) {
        let mut inner = self.lock();
        inner.looks.insert(
            round,
            Look {
                name: name.to_owned(),
                second,
            },
        );
    }

    /// The reviewer of the highest round seated here, `None` before the first
    #[must_use]
    pub fn reading(&self) -> Option<String> {
        self.lock()
            .looks
            .values()
            .next_back()
            .map(|look| look.name.clone())
    }

    /// Each read seated here, by round, for the line naming a replayed read
    fn looks(&self) -> BTreeMap<u32, Look> {
        self.lock().looks.clone()
    }

    /// Read the ticket's reads with `read`, then attach the channel `id` to follow them live
    ///
    /// Both happen under the lock [`Feed::forward`] takes, so no event is forwarded between
    /// them: it is in what `read` returned, or `send` gets it. A channel already watching
    /// under `id` is replaced.
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

/// Every ticket's read feeds, and the logs each read kept
#[derive(Debug)]
pub struct Reads {
    paths: Paths,

    /// The feeds of the tickets that have been watched or read, by ticket
    feeds: Mutex<BTreeMap<String, Arc<Feed>>>,
}

impl Reads {
    /// The feeds of a project whose reads are kept under `paths`
    #[must_use]
    pub fn new(paths: &Paths) -> Self {
        Self {
            paths: paths.clone(),
            feeds: Mutex::default(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, Arc<Feed>>> {
        self.feeds.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The log of the `round`th read of `ticket`
    fn log_file(&self, ticket: &str, round: u32) -> PathBuf {
        self.paths.review_log_file(ticket, round)
    }

    /// The feed of `ticket`'s reads, made on the first ask
    ///
    /// It counts what the logs of the reads it never saw hold, so a channel that attaches
    /// to it skips only the events still on their way.
    #[must_use]
    pub fn feed(&self, ticket: &str) -> Arc<Feed> {
        let mut feeds = self.lock();
        let feed = feeds.entry(ticket.to_owned()).or_insert_with(|| {
            let logged = (1..)
                .map_while(|round| self.log_file(ticket, round).exists().then_some(round))
                .map(|round| Feed::logged(&self.read_log(ticket, round)))
                .sum();
            Arc::new(Feed::new(logged))
        });
        Arc::clone(feed)
    }

    /// The log of read `round`, empty when the read kept none
    fn read_log(&self, ticket: &str, round: u32) -> Vec<LogEntry> {
        log::read(self.log_file(ticket, round)).unwrap_or_default()
    }

    /// The round read `ticket` is about to run writes, the first with no log yet
    ///
    /// This is the round its session opens its log on
    /// (`kelpie_engine::review::session::SessionReviewer::next_log`), which is why seating a
    /// read here names the log it leaves.
    fn next_round(&self, ticket: &str) -> u32 {
        (1..)
            .find(|round| !self.log_file(ticket, *round).exists())
            .unwrap_or(u32::MAX)
    }

    /// Seat the read `ticket` is about to run, by `name`, on the round its log will be, and
    /// answer the feed its events go to
    ///
    /// A read is seated before its session opens, which is before its log exists, so a read
    /// that never started leaves its round free for the one that runs next, and that one
    /// takes the look with it.
    pub fn seated(&self, ticket: &str, name: &str, second: bool) -> Arc<Feed> {
        let feed = self.feed(ticket);
        let round = self.next_round(ticket);
        feed.began(round, name, second);
        feed
    }

    /// Every read `ticket` has had, oldest first, with the line naming each one after the first
    ///
    /// `looks` is each read the feed had when the channel attached, by round, so a read it
    /// was told about is named by the look seated on its own round, and one it was not is
    /// counted.
    ///
    /// # Errors
    ///
    /// A read's log cannot be read, or holds a line that is not an entry.
    fn past(&self, ticket: &str, looks: &BTreeMap<u32, Look>) -> Result<Vec<LogEntry>, String> {
        let mut entries = Vec::new();
        for round in 1.. {
            let file = self.log_file(ticket, round);
            if !file.exists() {
                break;
            }
            let log = log::read(&file).map_err(|error| {
                format!("the log of read {round} of ticket {ticket} cannot be read: {error}")
            })?;
            if round > 1 {
                entries.push(LogEntry::Divider {
                    text: named(looks.get(&round), round),
                });
            }
            entries.extend(log);
        }
        Ok(entries)
    }

    /// The past of `ticket`'s reads, and `send` attached to follow the read in flight live
    ///
    /// # Errors
    ///
    /// One of the read logs cannot be read, or holds a line that is not an entry.
    pub fn watch(
        &self,
        ticket: &str,
        id: u32,
        send: impl Fn(SessionEvent) -> bool + Send + 'static,
    ) -> Result<Vec<LogEntry>, String> {
        let feed = self.feed(ticket);
        let looks = feed.looks();
        feed.watch(id, || self.past(ticket, &looks), send)
    }

    /// Stop sending `ticket`'s reads to the channel `id`
    ///
    /// A ticket no tab has watched has no feed to stop.
    pub fn unwatch(&self, ticket: &str, id: u32) {
        if let Some(feed) = self.lock().get(ticket) {
            feed.unwatch(id);
        }
    }

    /// The reviewer of the last read of `ticket`, `None` when it has had none
    #[must_use]
    pub fn reviewer_of(&self, ticket: &str) -> Option<String> {
        self.lock().get(ticket).and_then(|feed| feed.reading())
    }
}

impl Review {
    /// Seat the read `ticket` is about to run, by `name`, on the round its log will be, and
    /// answer the feed its events go to
    ///
    /// `second` is a second look, which the line above it says. Seat the read before its
    /// session opens, which is before its log exists.
    pub(crate) fn seated(&self, ticket: &str, name: &str, second: bool) -> Arc<Feed> {
        self.feeds.seated(ticket, name, second)
    }

    /// The logs of `ticket`'s reads, and `send` attached to follow the read in flight live
    ///
    /// # Errors
    ///
    /// One of the read logs cannot be read, or holds a line that is not an entry.
    pub(crate) fn watch(
        &self,
        ticket: &str,
        id: u32,
        send: impl Fn(SessionEvent) -> bool + Send + 'static,
    ) -> Result<Vec<LogEntry>, String> {
        self.feeds.watch(ticket, id, send)
    }

    /// Stop sending `ticket`'s reads to the channel `id`
    pub(crate) fn unwatch(&self, ticket: &str, id: u32) {
        self.feeds.unwatch(ticket, id);
    }

    /// The reviewer of the read `ticket` is on, `None` when it has had none
    pub(crate) fn reviewer_of(&self, ticket: &str) -> Option<String> {
        self.feeds.reviewer_of(ticket)
    }
}

/// The project's review pass, `None` before its board has been watched and its gate built
fn pass(state: &AppState, project: &str) -> Option<Arc<Review>> {
    state.workers().gate_of(project).map(|gate| gate.review())
}

/// Read every log of the ticket's reads, oldest first, and follow what it reads next
///
/// A read that is running streams onto `on_event` from the moment its log ends, and a
/// second look streams onto the same channel behind the line naming it.
///
/// # Errors
///
/// The project has no review pass yet, or one of the read logs cannot be read.
#[tauri::command(async)]
#[specta::specta]
pub fn review_watch(
    state: State<'_, AppState>,
    ticket: String,
    on_event: Channel<SessionEvent>,
) -> Result<Vec<LogEntry>, String> {
    let project = state.project_id(None)?;
    let id = on_event.id();
    let review = pass(&state, &project).ok_or("kelpie has no review pass for this project yet")?;
    review.watch(&ticket, id, move |event| on_event.send(event).is_ok())
}

/// Stop sending the ticket's reads to `on_event`
///
/// A tab calls it when it closes: a channel's send succeeds while the window lives, so
/// nothing else tells the feed the tab is gone. The read it was following goes on.
///
/// # Errors
///
/// The project has no review pass yet.
#[tauri::command]
#[specta::specta]
pub fn review_unwatch(
    state: State<'_, AppState>,
    ticket: String,
    on_event: Channel<SessionEvent>,
) -> Result<(), String> {
    let project = state.project_id(None)?;
    let review = pass(&state, &project).ok_or("kelpie has no review pass for this project yet")?;
    review.unwatch(&ticket, on_event.id());
    Ok(())
}

/// The reviewer of the read the ticket is on, which titles its tab
///
/// `None` when the ticket has had no read kelpie has seen, which is how a tab opened for a
/// card that has moved on from `Reviewing` is titled instead.
///
/// # Errors
///
/// The project has no review pass yet.
#[tauri::command]
#[specta::specta]
pub fn review_reviewer(
    state: State<'_, AppState>,
    ticket: String,
) -> Result<Option<String>, String> {
    let project = state.project_id(None)?;
    let review = pass(&state, &project).ok_or("kelpie has no review pass for this project yet")?;
    Ok(review.reviewer_of(&ticket))
}
