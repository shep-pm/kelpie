//! Whether CI holds a project's merge, how many fix attempts red CI gets, and
//! how long CI is waited on
//!
//! The gate reads these three (`docs/design/gate-and-merge.md`, Settings): SK's
//! `[ci]` keys `block`, `fix_attempts` and `queued_minutes`, with SK's defaults
//! (`core/settings/ci.rs`). Each is the project's own, since whether a repo runs
//! CI on its pull requests is the repo's.
//!
//! `ci-holds-merge` is a bool in [`Settings`](super::Settings) and
//! [`Resolved`](super::Resolved), so it needs no type here.

use std::borrow::Cow;
use std::num::NonZeroU32;
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use super::counts::SECONDS_PER_MINUTE;
use super::nonzero;

/// How many fix turns a ticket's worker gets on red runs before the gate rules
///
/// A count from 0, and no cap is the field being absent: SK spelled it -1, which
/// a settings file here refuses. 0 rules on the first red run. A head red twice
/// with nothing pushed is ruled on whatever this is, and a fix turn for a pull
/// request the merge queue removed is not counted.
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FixAttempts(u32);

impl FixAttempts {
    /// The most fix turns allowed
    #[must_use]
    pub fn count(self) -> u32 {
        self.0
    }
}

impl JsonSchema for FixAttempts {
    fn schema_name() -> Cow<'static, str> {
        "FixAttempts".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 0,
            "maximum": u32::MAX,
        })
    }
}

/// How many minutes CI is waited on before the gate acts, from 1 to a day
///
/// A check that sits queued or in progress this long is rerun once, and a head
/// with no check waits this long before a ruling. Never zero: a wait of no time
/// would rerun every check the moment it was seen.
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CiWait(NonZeroU32);

impl CiWait {
    /// The wait as a length of time
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.0.get()) * SECONDS_PER_MINUTE)
    }
}

impl JsonSchema for CiWait {
    fn schema_name() -> Cow<'static, str> {
        "CiWait".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_CI_WAIT_MINUTES,
        })
    }
}

/// The longest CI wait a settings file may name, one day in minutes
///
/// Past it a check that never finishes holds a ticket longer than a person looks back at it.
const MAX_CI_WAIT_MINUTES: u32 = 24 * 60;

/// How long CI is waited on when no layer says, an hour no job of a healthy run waits past
pub const AN_HOUR: CiWait = CiWait(nonzero(60));
