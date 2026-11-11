//! Holding every call while a rate limit is used up, and spacing the rest
//!
//! The board polls about once a minute, and a used-up limit must stop the
//! polling and show when it lifts (`docs/design/tickets.md`, GitHub, Staying
//! current). [`Hold`] wraps a [`Gh`]: a call that GitHub answers with a limit
//! used up holds every later call until the limit resets, read once from
//! `gh api rate_limit` (which counts against no limit), or for [`FALLBACK`]
//! when nothing says when. A held call fails at once with
//! [`ForgeError::Held`] and starts no child.
//!
//! One process is one pace, since no second budget runs through the App: a
//! [`Pace`] lets calls through no faster than [`PER_MINUTE`] a minute and
//! [`AT_ONCE`] a second, both well under GitHub's documented secondary limits
//! of 900 points a minute and 100 concurrent requests.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::failed::says_used_up;
use super::{Gh, RateLimits};
use crate::github::{Clock, SystemClock, Timestamp};
use crate::ticket::ForgeError;

/// How long a hold lasts when GitHub cannot say when its limit resets
pub const FALLBACK: u64 = 10 * 60;

/// The most calls let through in any minute
pub const PER_MINUTE: usize = 60;

/// The most calls let through in one second
pub const AT_ONCE: usize = 5;

// Seconds a call counts toward the minute's cap
const MINUTE: u64 = 60;

// The longest a `Retry-After` or a reset holds, so a wrong header cannot hold
// the board for good
const LONGEST_WAIT: u64 = 60 * 60;

/// What a call is told when it asks to go
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    /// Go now: the call is counted
    Go,
    /// Not yet: ask again after this many seconds
    Wait(u64),
    /// The limit is used up until then: make no call
    Held(Timestamp),
}

/// The hold and the pacer of this process's calls
pub trait Pace: Send + Sync + fmt::Debug {
    /// Whether a call may go at `now`, counting it when it may
    fn pass(&self, now: Timestamp) -> Pass;

    /// Until when every call is held, while that is after `now`
    fn held(&self, now: Timestamp) -> Option<Timestamp>;

    /// Holds every call until `until`, or the hold already standing if that is later
    fn hold(&self, until: Timestamp);
}

/// A [`Clock`] a call can wait on until the [`Pace`] lets it go
pub trait Wait: Clock {
    /// Blocks the calling thread for `seconds`
    fn wait(&self, seconds: u64);
}

impl Wait for SystemClock {
    fn wait(&self, seconds: u64) {
        std::thread::sleep(Duration::from_secs(seconds));
    }
}

/// The pacing rule alone, with no clock and no hold
#[derive(Debug, Clone)]
pub struct CallPacer {
    per_minute: usize,
    at_once: usize,
    // When each call in the last minute went, oldest first
    grants: VecDeque<u64>,
}

impl CallPacer {
    /// A pacer that lets through `per_minute` calls a minute and `at_once` a second
    #[must_use]
    pub fn new(per_minute: usize, at_once: usize) -> Self {
        Self {
            per_minute,
            at_once,
            grants: VecDeque::new(),
        }
    }

    /// `None` when a call may go at `now`, which counts it, or the seconds it waits
    pub fn ask(&mut self, now: Timestamp) -> Option<u64> {
        let now = now.secs();
        while let Some(at) = self.grants.front().copied()
            && at.saturating_add(MINUTE) <= now
        {
            self.grants.pop_front();
        }
        let mut wait = 0;
        if self.grants.len() >= self.per_minute {
            // The oldest call leaves the minute, and frees a place, at `at + MINUTE`
            let leaves = |at: u64| at.saturating_add(MINUTE).saturating_sub(now).max(1);
            wait = wait.max(self.grants.front().map_or(1, |at| leaves(*at)));
        }
        if self.grants.iter().filter(|at| **at == now).count() >= self.at_once {
            wait = wait.max(1);
        }
        if wait > 0 {
            return Some(wait);
        }
        self.grants.push_back(now);
        None
    }
}

#[derive(Debug)]
struct Standing {
    pacer: CallPacer,
    until: Option<Timestamp>,
}

/// A [`Pace`] kept in this process
#[derive(Debug)]
pub struct LocalPace(Mutex<Standing>);

impl LocalPace {
    /// A pace with the limits of [`PER_MINUTE`] and [`AT_ONCE`]
    #[must_use]
    pub fn new() -> Self {
        Self::with(CallPacer::new(PER_MINUTE, AT_ONCE))
    }

    /// A pace spacing calls as `pacer` does
    #[must_use]
    pub fn with(pacer: CallPacer) -> Self {
        Self(Mutex::new(Standing { pacer, until: None }))
    }

    fn standing(&self) -> MutexGuard<'_, Standing> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for LocalPace {
    fn default() -> Self {
        Self::new()
    }
}

impl Pace for LocalPace {
    fn pass(&self, now: Timestamp) -> Pass {
        let mut standing = self.standing();
        if let Some(until) = standing.until.filter(|until| *until > now) {
            return Pass::Held(until);
        }
        match standing.pacer.ask(now) {
            None => Pass::Go,
            Some(seconds) => Pass::Wait(seconds),
        }
    }

    fn held(&self, now: Timestamp) -> Option<Timestamp> {
        let mut standing = self.standing();
        if standing.until.is_some_and(|until| until <= now) {
            standing.until = None;
        }
        standing.until
    }

    fn hold(&self, until: Timestamp) {
        let mut standing = self.standing();
        standing.until = standing.until.max(Some(until));
    }
}

/// A [`Gh`] whose calls are held while a rate limit is used up, and paced
///
/// Cloning a hold gives another handle to the same one, so one repo's calls are one
/// set: [`super::GithubSource`] reads its issues and [`super::GithubPulls`] opens its
/// drafts through the same hold, and no second budget runs.
#[derive(Clone)]
pub struct Hold {
    shared: Arc<Shared>,
}

/// What every handle of one hold shares
struct Shared {
    gh: Gh,
    pace: Box<dyn Pace>,
    clock: Arc<dyn Wait>,
}

impl fmt::Debug for Hold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Hold")
            .field("gh", &self.shared.gh)
            .field("pace", &self.shared.pace)
            .finish_non_exhaustive()
    }
}

impl Hold {
    /// Calls through `gh` at [`PER_MINUTE`] and [`AT_ONCE`], by the OS clock
    #[must_use]
    pub fn new(gh: Gh) -> Self {
        Self::with(gh, Box::new(LocalPace::new()), Arc::new(SystemClock))
    }

    /// Calls through `gh`, held and spaced by `pace`, by `clock`'s time
    #[must_use]
    pub fn with(gh: Gh, pace: Box<dyn Pace>, clock: Arc<dyn Wait>) -> Self {
        Self {
            shared: Arc::new(Shared { gh, pace, clock }),
        }
    }

    /// When the hold lifts, while a limit is used up
    #[must_use]
    pub fn lifts_at(&self) -> Option<Timestamp> {
        self.shared.pace.held(self.shared.clock.now())
    }

    /// What `gh` printed on stdout for a call of `args`, as [`Gh::call`] says
    ///
    /// # Errors
    ///
    /// [`ForgeError::Held`] while a limit is used up, and otherwise those of
    /// [`Gh::call`].
    pub fn call(&self, args: &[&str]) -> Result<Vec<u8>, ForgeError> {
        self.guarded(|gh| gh.call(args))
    }

    /// The login `gh` holds, as [`Gh::viewer`] says
    ///
    /// # Errors
    ///
    /// As [`Hold::call`].
    pub fn viewer(&self) -> Result<String, ForgeError> {
        self.guarded(Gh::viewer)
    }

    /// The login's limits, as [`Gh::rate_limit`] says
    ///
    /// # Errors
    ///
    /// As [`Hold::call`].
    pub fn rate_limit(&self) -> Result<RateLimits, ForgeError> {
        self.guarded(Gh::rate_limit)
    }

    // `call` once the pace lets it go, and a hold set when it finds a limit used up
    pub(super) fn guarded<T>(
        &self,
        call: impl FnOnce(&Gh) -> Result<T, ForgeError>,
    ) -> Result<T, ForgeError> {
        if let Err(until) = self.admit() {
            return Err(ForgeError::Held(until));
        }
        let now = self.shared.clock.now();
        let result = call(&self.shared.gh);
        if let Err(error) = &result
            && used_up(error)
        {
            self.shared.pace.hold(self.lifts_from(error, now));
        }
        result
    }

    // Waits until the pace lets a call go, or says until when calls are held
    fn admit(&self) -> Result<(), Timestamp> {
        loop {
            match self.shared.pace.pass(self.shared.clock.now()) {
                Pass::Go => return Ok(()),
                Pass::Held(until) => return Err(until),
                Pass::Wait(seconds) => self.shared.clock.wait(seconds),
            }
        }
    }

    // What the answer itself says of when to ask again comes first, then the
    // limit's reset, then the fallback
    fn lifts_from(&self, error: &ForgeError, now: Timestamp) -> Timestamp {
        said_until(error, now)
            .or_else(|| self.reset(now))
            .unwrap_or_else(|| Timestamp::from_secs(now.secs().saturating_add(FALLBACK)))
    }

    // The read is a call to GitHub too, so it takes its turn in the pace. A
    // hold set meanwhile answers for it.
    fn reset(&self, now: Timestamp) -> Option<Timestamp> {
        self.admit().ok()?;
        let limits = self.shared.gh.rate_limit().ok()?;
        [limits.core, limits.graphql, limits.search]
            .into_iter()
            .filter(|bucket| bucket.remaining == 0 && bucket.reset > now)
            .map(|bucket| bucket.reset)
            .max()
    }
}

// Whether `error` is GitHub saying a rate limit is used up
fn used_up(error: &ForgeError) -> bool {
    match error {
        ForgeError::Limited { .. } => true,
        ForgeError::Failed(stderr) => says_used_up(stderr),
        _ => false,
    }
}

// `Retry-After` from `now` when the answer gave one, else its reset when that
// is still ahead. A wait is at least a second and at most an hour.
fn said_until(error: &ForgeError, now: Timestamp) -> Option<Timestamp> {
    let ForgeError::Limited { cooldown, .. } = error else {
        return None;
    };
    let after = |seconds: u64| Timestamp::from_secs(now.secs().saturating_add(seconds));
    if let Some(wait) = cooldown.retry_after {
        return Some(after(wait.clamp(1, LONGEST_WAIT)));
    }
    cooldown
        .reset
        .filter(|reset| *reset > now)
        .map(|reset| reset.min(after(LONGEST_WAIT)))
}
