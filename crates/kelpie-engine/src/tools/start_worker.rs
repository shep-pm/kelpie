//! `start_worker`: the PM starts a worker on a ticket
//!
//! The second of kelpie's tools (`docs/design/kelpie-tools.md`, The role split),
//! and the PM's alone: a worker's session is never given it. The tool hands the
//! ticket to the app's one start function, which is what the Start button and a
//! drop on Building call, so the PM starts work the way the maintainer does.
//! Whether the ticket is Ready is the start's business, as it is the button's.

use std::fmt;
use std::sync::Arc;

use rmcp::model::Tool;
use serde_json::{Map, Value};

use crate::ticket::TicketId;

/// The name the tool goes by
pub(super) const START_WORKER: &str = "start_worker";

/// What `start_worker` starts a worker through
///
/// The engine starts no worker of its own: the app's starter is the one start
/// function the Start button and a drop on Building call. A starter's line is
/// the tool's answer, and its error is the tool's error line.
pub trait WorkerStarter: Send + Sync + fmt::Debug {
    /// Start a worker on `ticket`, and say in one line what happened
    ///
    /// # Errors
    ///
    /// Why nothing started, for the agent to read.
    fn start(&self, ticket: &str) -> Result<String, String>;
}

/// Handle one `start_worker` call, and give the line the agent hears
///
/// The start runs on a blocking thread: it prepares a worktree and starts a
/// harness, which the server's own thread must not wait on.
///
/// # Errors
///
/// What is wrong with the call, or what the start could not do.
pub(super) async fn call(
    starter: Arc<dyn WorkerStarter>,
    arguments: &Map<String, Value>,
) -> Result<String, String> {
    let ticket = ticket(arguments)?;
    match tokio::task::spawn_blocking(move || starter.start(&ticket)).await {
        Ok(line) => line,
        // A start that ended on the blocking thread without answering
        Err(ended) => Err(format!("kelpie could not start a worker: {ended}")),
    }
}

/// The ticket the call names, as the starter should read it
///
/// A number as text or as a JSON integer. `#155` is taken as `155`, since that
/// is how kelpie's board writes a ticket.
fn ticket(arguments: &Map<String, Value>) -> Result<String, String> {
    let given = match arguments.get("ticket") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => {
            return Err(format!(
                "{START_WORKER} takes a `ticket`: an issue number such as 155"
            ));
        }
    };
    let written = given.trim();
    let number = written.strip_prefix('#').unwrap_or(written);
    number
        .parse::<TicketId>()
        .map(|id| id.to_string())
        .map_err(|error| format!("{given} is not a ticket: {error}"))
}

/// The tool, as a harness lists it
pub(super) fn tool() -> Tool {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "ticket": {
                "type": ["string", "integer"],
                "description": "The ticket's issue number, such as 155 or #155.",
            },
        },
        "required": ["ticket"],
    });
    let serde_json::Value::Object(schema) = schema else {
        unreachable!("a JSON object literal")
    };
    Tool::new(
        START_WORKER,
        "Start a worker on a ticket. Says in one line whether the worker started, or that the ticket is waiting for a free slot.",
        schema,
    )
}
