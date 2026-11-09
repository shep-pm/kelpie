//! Protocol: the types the engine and the app share
//!
//! A session's events ([`SessionEvent`]) are shaped after ACP's
//! `session/update`, but no harness owns them and this crate depends on none:
//! not the engine, not Tauri, not ACP (ADR 0007). Everything here is wire
//! format, serialized into kelpie's session log and across the app's IPC.
//!
//! # Quick start
//!
//! ```
//! use kelpie_protocol::SessionEvent;
//!
//! let event = SessionEvent::Text {
//!     delta: "Hello".to_owned(),
//! };
//! assert_eq!(
//!     serde_json::to_string(&event)?,
//!     r#"{"kind":"text","delta":"Hello"}"#
//! );
//! # Ok::<(), serde_json::Error>(())
//! ```

#![doc(test(attr(deny(warnings))))]

use std::fmt;

use serde::{Deserialize, Serialize};

/// The id a harness gave a session
///
/// Kelpie keeps it to resume the session after the harness's process stopped.
// wire format: changing this is a breaking change
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl SessionId {
    /// The id as the harness spelled it
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for SessionId {
    fn from(id: &str) -> Self {
        Self(id.to_owned())
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One thing a session did, in the order it happened
// wire format: changing this is a breaking change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive] // new kinds of event arrive with new harness features
pub enum SessionEvent {
    /// The harness has a session ready, new or resumed
    Started {
        /// The id to resume the session with
        session_id: SessionId,
    },
    /// A piece of the reply
    Text {
        /// The next characters of the reply
        delta: String,
    },
    /// A piece of the harness's reasoning, shown folded
    Thought {
        /// The next characters of the reasoning
        delta: String,
    },
    /// A tool call, each time its title or status changes
    ToolStep {
        /// The harness's id for the call, the same on every event of one call
        id: String,
        /// One line saying what the call does
        title: String,
        /// How far the call got
        status: ToolStatus,
    },
    /// The turn is over
    TurnEnded {
        /// Why it ended
        stop_reason: StopReason,
        /// The tokens the turn used, when the harness says
        usage: Option<Usage>,
    },
    /// Something went wrong
    ///
    /// Usually the turn or the session did not survive it and nothing follows.
    /// A session that started over, or kept its default model or effort, says
    /// so with a `Failed` and then goes on with `Started` or `TurnEnded`.
    Failed {
        /// What happened, for the maintainer to read
        message: String,
    },
}

/// How far a tool call got
// wire format: changing this is a breaking change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive] // harnesses report more states over time
pub enum ToolStatus {
    /// Announced, not started
    Pending,
    /// Running
    Running,
    /// Finished
    Done,
    /// Ended with an error
    Failed,
    /// Kelpie declined to let it run
    Refused,
}

/// Why a turn ended
// wire format: changing this is a breaking change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive] // the spec adds stop reasons over time
pub enum StopReason {
    /// The model finished its reply
    EndTurn,
    /// The model reached its token limit
    MaxTokens,
    /// The harness reached its limit of model requests in one turn
    MaxTurnRequests,
    /// The model declined to continue
    Refusal,
    /// The turn was cancelled
    Cancelled,
}

/// The tokens one turn used
// wire format: changing this is a breaking change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Tokens sent to the model
    pub input: u64,
    /// Tokens the model wrote
    pub output: u64,
    /// Input tokens the prompt cache served, `0` when the harness does not say
    pub cache_read: u64,
    /// Input tokens written to the prompt cache, `0` when the harness does not say
    pub cache_write: u64,
    /// How long the written cache lasts, only from a harness that reports it
    pub cache_lifetime: Option<CacheLifetime>,
}

/// How long a prompt cache entry lives
// wire format: changing this is a breaking change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive] // providers offer more lifetimes over time
pub enum CacheLifetime {
    /// Five minutes
    FiveMinutes,
    /// One hour
    OneHour,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_is_tagged_with_its_kind() {
        let events = [
            (
                SessionEvent::Started {
                    session_id: "s1".into(),
                },
                r#"{"kind":"started","session_id":"s1"}"#,
            ),
            (
                SessionEvent::ToolStep {
                    id: "t1".to_owned(),
                    title: "Read".to_owned(),
                    status: ToolStatus::Refused,
                },
                r#"{"kind":"tool_step","id":"t1","title":"Read","status":"refused"}"#,
            ),
            (
                SessionEvent::TurnEnded {
                    stop_reason: StopReason::EndTurn,
                    usage: Some(Usage {
                        input: 3,
                        output: 4,
                        cache_read: 5,
                        cache_write: 6,
                        cache_lifetime: Some(CacheLifetime::OneHour),
                    }),
                },
                concat!(
                    r#"{"kind":"turn_ended","stop_reason":"end_turn","usage":"#,
                    r#"{"input":3,"output":4,"cache_read":5,"cache_write":6,"#,
                    r#""cache_lifetime":"one_hour"}}"#
                ),
            ),
            (
                SessionEvent::Failed {
                    message: "gone".to_owned(),
                },
                r#"{"kind":"failed","message":"gone"}"#,
            ),
        ];
        for (event, json) in events {
            assert_eq!(serde_json::to_string(&event).unwrap(), json);
            assert_eq!(serde_json::from_str::<SessionEvent>(json).unwrap(), event);
        }
    }
}
