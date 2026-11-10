//! Pacing: a limit whose window is spent holds every new turn until it resets
//!
//! A limit at 100% of its 5-hour window or its week holds every new turn until
//! the longer of the two resets (`docs/design/workers.md`, Slots, pacing and
//! leases). Nothing else holds a turn, and usage kelpie cannot read holds
//! nothing. A turn already running is never interrupted: the pacer is asked
//! only between turns.
//!
//! Each limit is keyed by a [`LimitId`], which a harness's adapter names for
//! the limit that harness has. A harness with no limit names none, so a local
//! model is never held and never read. No [`Meter`] reads a harness yet.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::pacer::{LimitId, Timestamp, Utilization, Window, spent};
//!
//! let limit = LimitId::new("claude-code");
//! let usage = Utilization {
//!     session: Window {
//!         used_pct: 60,
//!         resets_at: Timestamp::from_secs(9_000),
//!     },
//!     week: Window {
//!         used_pct: 100,
//!         resets_at: Timestamp::from_secs(90_000),
//!     },
//! };
//! let hold = spent(&limit, &usage).expect("the week is spent");
//! assert_eq!(hold.until, Timestamp::from_secs(90_000));
//! assert_eq!(
//!     hold.reason,
//!     "claude-code's week is at 100%, so no turn starts until it resets"
//! );
//! ```

use std::collections::BTreeMap;
use std::fmt;

#[cfg(test)]
mod tests;

/// Percent of a window at which it is spent, where a turn fails at once
pub const SPENT_PCT: u32 = 100;

/// How long a hold is trusted before usage is read again, in seconds
///
/// A harness prints a reset time to the minute, and a window can reset while
/// its usage is being read, so a hold is never trusted past its reset alone.
pub const RECHECK_SECS: u64 = 600;

/// A moment, as whole seconds since the Unix epoch
///
/// A harness prints a window's reset to the minute, so seconds are finer than
/// anything kelpie compares here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The moment as whole seconds since the Unix epoch
    #[must_use]
    pub fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    /// The seconds themselves
    #[must_use]
    pub fn secs(self) -> u64 {
        self.0
    }
}

/// Which limit's windows a hold is about
///
/// A harness's adapter names it for the limit that harness has, such as
/// `claude-code`. A harness with no limit names none, and is never held.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LimitId(String);

impl LimitId {
    /// The id as the harness's adapter spells it
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The id itself
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LimitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How much of one window a limit has spent
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    /// Whole percent of the window used
    pub used_pct: u32,

    /// When the window resets
    pub resets_at: Timestamp,
}

/// What a limit has spent: its 5-hour window, and its week across every model it covers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utilization {
    /// The 5-hour window
    pub session: Window,

    /// The weekly window
    pub week: Window,
}

/// Reads one limit's usage
pub trait Meter: Send + Sync {
    /// The limit's windows at `now`
    ///
    /// `now` places the reset times, which a harness prints without a year.
    ///
    /// # Errors
    ///
    /// [`MeterError`] when the usage cannot be read.
    fn read(&self, now: Timestamp) -> Result<Utilization, MeterError>;
}

/// Why a limit's usage could not be read
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive] // each harness's meter will fail in its own way
pub enum MeterError {
    /// The harness could not be asked, with what starting it said
    Spawn(String),

    /// The harness answered, but not with usage kelpie can read
    Unreadable(String),
}

impl fmt::Display for MeterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(why) => f.write_str(why),
            Self::Unreadable(why) => f.write_str(why),
        }
    }
}

impl core::error::Error for MeterError {}

/// Why kelpie is holding, and until when
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    /// What the maintainer reads beside it
    pub reason: String,

    /// When the window it names resets
    pub until: Timestamp,
}

/// What one reading of a limit's windows decided
#[derive(Debug, Clone, PartialEq)]
pub struct Assessment {
    /// When kelpie read the usage
    pub read_at: Timestamp,

    /// The hold on every new turn, where a window is spent
    pub hold: Option<Hold>,
}

/// What the next turn for one limit may do
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pace {
    /// A turn may start now
    Clear,

    /// No turn starts until the hold ends
    Held(Hold),

    /// The usage could not be read, so nothing is held; the text is what the read said
    Unreadable(String),
}

/// What `usage` says about `limit` at `now`
#[must_use]
pub fn assess(limit: &LimitId, now: Timestamp, usage: &Utilization) -> Assessment {
    Assessment {
        read_at: now,
        hold: spent(limit, usage),
    }
}

/// The hold on every new turn while `limit`'s 5-hour window or week is at [`SPENT_PCT`],
/// until it resets: the longer of the two holds when both apply
///
/// A spent window fails each turn at once, as a spent week does.
#[must_use]
pub fn spent(limit: &LimitId, usage: &Utilization) -> Option<Hold> {
    [
        spent_window(limit, "5-hour window", &usage.session),
        spent_window(limit, "week", &usage.week),
    ]
    .into_iter()
    .flatten()
    .max_by_key(|hold| hold.until)
}

/// The hold on `limit` while `named` is at [`SPENT_PCT`], until it resets
fn spent_window(limit: &LimitId, named: &str, window: &Window) -> Option<Hold> {
    (window.used_pct >= SPENT_PCT).then(|| Hold {
        reason: format!(
            "{limit}'s {named} is at {}%, so no turn starts until it resets",
            window.used_pct
        ),
        until: window.resets_at,
    })
}

/// What the pacer last read for each limit, and how long it still trusts it
#[derive(Debug, Default)]
pub struct Pacing {
    last: BTreeMap<LimitId, Assessment>,
}

impl Pacing {
    /// Whether a turn may start for `limit` at `now`, reading its usage when
    /// the last reading has gone stale
    ///
    /// A limit with no `meter` is never held and never recorded: kelpie has
    /// nothing to read it with. A meter that cannot be read holds nothing, and
    /// its reason comes back for the maintainer to read.
    pub fn pace(&mut self, limit: &LimitId, meter: Option<&dyn Meter>, now: Timestamp) -> Pace {
        let Some(meter) = meter else {
            return Pace::Clear;
        };
        if let Some(hold) = self.still_held(limit, now) {
            return Pace::Held(hold.clone());
        }
        match meter.read(now) {
            Ok(usage) => {
                let assessment = assess(limit, now, &usage);
                let hold = assessment.hold.clone();
                self.last.insert(limit.clone(), assessment);
                hold.map_or(Pace::Clear, Pace::Held)
            }
            // A reading kelpie does not have cannot hold a turn, so nothing is
            // recorded and the next ask reads again.
            Err(error) => Pace::Unreadable(error.to_string()),
        }
    }

    /// The hold an earlier reading of `limit` found, while it is trusted
    ///
    /// A hold is trusted until [`RECHECK_SECS`] pass or its own reset comes,
    /// whichever comes first.
    #[must_use]
    pub fn still_held(&self, limit: &LimitId, now: Timestamp) -> Option<&Hold> {
        let last = self.last.get(limit)?;
        let trusted = now.secs() < last.read_at.secs().saturating_add(RECHECK_SECS);
        last.hold
            .as_ref()
            .filter(|hold| trusted && now < hold.until)
    }
}
