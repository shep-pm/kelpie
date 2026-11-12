//! Whose issues a project picks up as tickets, the label its Ready tickets
//! carry, and how far back its Done column reaches
//!
//! The board reads these three to say which of a project's issues are tickets,
//! which of them are Ready, and how long a closed one stays on the board
//! (`docs/design/tickets.md`, Where tickets come from, and The board).
//!
//! `pick-up` and `ready-label` are Project only, since the ticket source is the
//! project's. `done-window` is Global and Project, since how far back a person
//! wants to look is theirs.

use std::borrow::Cow;
use std::num::NonZeroU32;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Field, TestCommand, nonzero};

/// Who a project's tickets come from
///
/// An issue carrying the ready label is a ticket whichever way this is set.
// wire format: a settings file holds these strings
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PickUp {
    /// Me: the issues the signed-in login opened
    #[default]
    Me,

    /// Everyone: every open issue
    Everyone,
}

/// How many days a closed ticket stays in the board's Done column
///
/// Never zero: a window of no days would empty Done of every closed ticket,
/// which is the one column that says the work is finished.
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DoneWindow(NonZeroU32);

impl DoneWindow {
    /// The number of days, for the board to keep its closed tickets by
    #[must_use]
    pub fn days(self) -> u32 {
        self.0.get()
    }
}

impl JsonSchema for DoneWindow {
    fn schema_name() -> Cow<'static, str> {
        "DoneWindow".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_DONE_DAYS,
        })
    }
}

/// The longest Done window a settings file may name, a year of days
///
/// Past it a closed ticket is history rather than something the board shows.
const MAX_DONE_DAYS: u32 = 365;

/// How long a closed ticket stays in Done when no layer says
pub const FOURTEEN_DAYS: DoneWindow = DoneWindow(nonzero(14));

/// The label a project's Ready tickets carry when it names none of its own
///
/// The label kelpie's own issues are filed with (`docs/agents/triage-labels.md`).
pub const READY_FOR_AGENT: &str = "ready-for-agent";

/// Why a name cannot be a project's ready label, as the person writing it reads it
///
/// Read after the field's key, as `SettingsError::ValueRefused` writes it:
/// `ready-label must name one label: not empty, and no comma`.
const LABEL_RULE: &str = "must name one label: not empty, and no comma";

impl Field {
    /// A rule of kelpie's own that the settings schema cannot say, as what `value` breaks
    ///
    /// `Settings::set` checks a value against the schema and then against this,
    /// so a value either refuses is refused before anything is written.
    ///
    /// `gh issue list --label` splits its argument on commas, so a ready label
    /// holding one names two labels and never the one meant. An empty one names
    /// none, and JSON Schema's `minLength` is no help beside a comma.
    pub(crate) fn own_rule(self, value: &Value) -> Option<&'static str> {
        match self {
            Self::ReadyLabel => match value.as_str() {
                Some(name) if name.is_empty() || name.contains(',') => Some(LABEL_RULE),
                _ => None,
            },
            Self::TestCommand => value.as_str().and_then(TestCommand::rule),
            _ => None,
        }
    }
}
