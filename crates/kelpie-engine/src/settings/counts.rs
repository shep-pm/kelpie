//! The counts a settings file holds: worker slots, a turn's ceiling, a lease's capacity
//!
//! Each is bounded in the schema, so a write cannot land a file kelpie then
//! cannot read back.

use std::borrow::Cow;
use std::num::NonZeroU32;
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// How many of a project's workers run at once
///
/// 0 is a project holding every worker out of its slots: no worker starts a turn.
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Slots(pub(super) u32);

impl Slots {
    /// How many places there are, as a count kelpie can index with
    #[must_use]
    pub fn places(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// How long a worker's turn may run, in whole minutes
///
/// Never zero: a ceiling of no time would stop every turn before it began. It
/// counts from when the worker's slot was granted ([`crate::worker::ceiling`]).
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnTimeout(pub(super) NonZeroU32);

impl TurnTimeout {
    /// The ceiling as a length of time
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.0.get()) * SECONDS_PER_MINUTE)
    }
}

/// How many commands hold a lease at once
///
/// Never zero: a lease with no place can never be taken, and every command
/// queued behind it then waits forever.
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capacity(pub(super) NonZeroU32);

impl Capacity {
    /// How many places there are, as a count kelpie can index with
    #[must_use]
    pub fn places(self) -> usize {
        usize::try_from(self.0.get()).unwrap_or(usize::MAX)
    }

    /// The count itself, for the lease that holds this many commands at once
    #[must_use]
    pub(crate) const fn count(self) -> NonZeroU32 {
        self.0
    }
}

/// The most a settings file may name, past which it is a typo
///
/// A count this large names more agents or commands than a machine can run at
/// once. Naming it in the schema is what keeps a write from landing a file kelpie
/// then cannot read back.
pub(super) const MAX_COUNT: u32 = 64;

/// The longest turn ceiling a settings file may name, one day in minutes
///
/// Past it a stuck turn holds its slot for longer than a person looks back at it.
pub(super) const MAX_TURN_MINUTES: u32 = 24 * 60;

pub(super) const SECONDS_PER_MINUTE: u64 = 60;

/// A count the schema bounds, as the nonzero one [`Capacity`] holds
pub(super) const fn nonzero(count: u32) -> NonZeroU32 {
    match NonZeroU32::new(count) {
        Some(count) => count,
        None => panic!("a lease always has a place"),
    }
}

impl JsonSchema for Slots {
    fn schema_name() -> Cow<'static, str> {
        "Slots".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 0,
            "maximum": MAX_COUNT,
        })
    }
}

impl JsonSchema for TurnTimeout {
    fn schema_name() -> Cow<'static, str> {
        "TurnTimeout".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_TURN_MINUTES,
        })
    }
}

impl JsonSchema for Capacity {
    fn schema_name() -> Cow<'static, str> {
        "Capacity".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_COUNT,
        })
    }
}

/// How many commands hold [`super::CARGO_TEST`] at once when no layer names it
pub const CARGO_TEST_CAPACITY: Capacity = Capacity(nonzero(3));

/// How many commands hold a lease no layer names a capacity for
pub(super) const ONE_AT_A_TIME: Capacity = Capacity(nonzero(1));

/// How long a worker's turn may run when no layer names a ceiling, as SK's
/// `worker.turn_timeout` has it
pub(super) const SIXTY_MINUTES: TurnTimeout = TurnTimeout(nonzero(60));

/// How many of a project's workers run at once when no layer names a cap
pub(super) const TWO_WORKERS: Slots = Slots(2);
