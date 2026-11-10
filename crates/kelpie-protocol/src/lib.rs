//! Protocol: the types the engine and the app share
//!
//! A session's events ([`SessionEvent`]) are shaped after ACP's
//! `session/update`, but no harness owns them and this crate depends on none:
//! not the engine, not Tauri, not ACP (ADR 0007). Everything here is wire
//! format, serialized into kelpie's session log and across the app's IPC.
//!
//! The `specta` feature derives `specta::Type` on every type, which the app
//! turns into TypeScript. The engine leaves it off.
//!
//! What a harness offers ([`Options`]) is listed by the harness itself, which is
//! how kelpie names no model outside the default (ADR 0006).
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
#[cfg_attr(feature = "specta", derive(specta::Type))]
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
#[cfg_attr(feature = "specta", derive(specta::Type))]
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
    /// The agent asked to reach a host the sandbox refuses, and waits for the maintainer
    ///
    /// Answered with `Session::answer`. A turn that ends, or a session that
    /// stops, while it waits answers it denied.
    SandboxRequest {
        /// The request's id, which the answer names
        id: String,
        /// The host the agent wants to reach, a name as the sandbox's network lists take it
        host: String,
        /// Why the agent wants it, in its own words
        why: String,
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

/// One line of a session's log: a message the maintainer sent, or an event the session streamed
///
/// Kelpie keeps its own log of every session, so history shows on any harness
/// without starting one. The log holds what a harness's own transcript does
/// not: what kelpie sent and what it saw.
// wire format: the session log holds one of these per line, so changing this
// is a breaking change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "entry", rename_all = "snake_case")]
#[non_exhaustive] // notes arrive with their own slice
pub enum LogEntry {
    /// A message the maintainer sent, which starts a turn
    User {
        /// What they wrote
        text: String,
    },
    /// An event the session streamed
    Event(SessionEvent),
    /// A line kelpie wrote between two sessions, such as a switch of harness
    ///
    /// The history above it stays, and the session below it starts from a
    /// seed instead of from what is above.
    Divider {
        /// What the line says
        text: String,
    },
}

/// The maintainer's answer to a [`SessionEvent::SandboxRequest`]
// wire format: changing this is a breaking change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive] // an answer may grow a scope, such as every project
pub enum SandboxAnswer {
    /// Let the host through for as long as the session runs
    Once,
    /// Let it through for the session, and the app saves it to the project's allowlist
    Project,
    /// Change nothing
    Deny,
}

/// How far a tool call got
// wire format: changing this is a breaking change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
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
#[cfg_attr(feature = "specta", derive(specta::Type))]
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
#[cfg_attr(feature = "specta", derive(specta::Type))]
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
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive] // providers offer more lifetimes over time
pub enum CacheLifetime {
    /// Five minutes
    FiveMinutes,
    /// One hour
    OneHour,
}

/// What a harness says it offers, listed by the harness itself
///
/// A harness names its own models and efforts, so kelpie holds no model list of
/// its own (ADR 0006). A harness that offers no effort leaves `efforts` empty.
// wire format: changing this is a breaking change
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Options {
    /// The models the harness lists, in the order it lists them
    pub models: Vec<Choice>,
    /// The efforts the harness lists, empty when it offers none
    pub efforts: Vec<Choice>,
}

/// One model or effort a harness lists
// wire format: changing this is a breaking change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Choice {
    /// The value kelpie sends back to the harness to pick this one
    pub id: String,
    /// The label the harness gives it
    pub name: String,
    /// What the harness says about it, `None` when it says nothing
    pub description: Option<String>,
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

    #[test]
    fn a_log_entry_is_one_json_object_tagged_with_its_entry() {
        let entries = [
            (
                LogEntry::User {
                    text: "Hi".to_owned(),
                },
                r#"{"entry":"user","text":"Hi"}"#,
            ),
            (
                LogEntry::Event(SessionEvent::Text {
                    delta: "Hello".to_owned(),
                }),
                r#"{"entry":"event","kind":"text","delta":"Hello"}"#,
            ),
            (
                LogEntry::Divider {
                    text: "Switched".to_owned(),
                },
                r#"{"entry":"divider","text":"Switched"}"#,
            ),
        ];
        for (entry, json) in entries {
            assert_eq!(serde_json::to_string(&entry).unwrap(), json);
            assert_eq!(serde_json::from_str::<LogEntry>(json).unwrap(), entry);
        }
    }

    #[test]
    fn a_choice_keeps_the_id_the_harness_picks_it_with() {
        let options = Options {
            models: vec![Choice {
                id: "claude-opus".to_owned(),
                name: "Opus".to_owned(),
                description: Some("the deepest reply".to_owned()),
            }],
            efforts: Vec::new(),
        };
        let json = concat!(
            r#"{"models":[{"id":"claude-opus","name":"Opus","#,
            r#""description":"the deepest reply"}],"efforts":[]}"#
        );

        assert_eq!(serde_json::to_string(&options).unwrap(), json);
        assert_eq!(serde_json::from_str::<Options>(json).unwrap(), options);
        assert_eq!(
            serde_json::from_str::<Options>(r#"{"models":[],"efforts":[]}"#).unwrap(),
            Options::default()
        );
    }
}
