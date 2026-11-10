//! The agent's side of kelpie's tools, for a test
//!
//! [`FakeTools`] dials a session's door with its token and calls
//! `request_network` over MCP, as a harness's relay and the agent behind it do
//! (`docs/design/kelpie-tools.md`, The route out). A test of a session, or of a
//! command that answers one, needs no MCP client and no door code of its own.

use std::io::{self, BufRead, BufReader, Write};
use std::time::Duration;

use serde_json::{Value, json};

use super::SessionTools;
use super::door::Stream;

/// How long a fake waits for a line kelpie should have sent
const WAIT: Duration = Duration::from_secs(10);

/// The id of every call a fake makes, so the line it reads back is its own
const CALL: u64 = 1;

/// The MCP version a fake says it speaks
const VERSION: &str = "2025-06-18";

/// The agent's side of kelpie's tools: a connection holding a session's token
#[derive(Debug)]
pub struct FakeTools {
    to_door: Stream,
    from_door: BufReader<Stream>,
}

impl FakeTools {
    /// Dial the door `tools` names with its token, and open the MCP conversation
    ///
    /// # Errors
    ///
    /// The door cannot be dialled, or answers with a line that is not JSON.
    pub fn connect(tools: &SessionTools) -> io::Result<Self> {
        let to_door = tools.door().connect()?;
        let from_door = BufReader::new(to_door.try_clone()?);
        to_door.set_read_timeout(WAIT)?;
        let mut asked = Self { to_door, from_door };
        writeln!(asked.to_door, "{}", tools.token())?;
        asked.say(json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {
                "protocolVersion": VERSION,
                "capabilities": {},
                "clientInfo": {"name": "kelpie-test", "version": "0"},
            },
        }))?;
        asked.line()?;
        asked.say(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
        Ok(asked)
    }

    /// Call `request_network` for `host`, saying `why`
    ///
    /// # Errors
    ///
    /// The door closes before taking the call.
    pub fn ask(&mut self, host: &str, why: &str) -> io::Result<()> {
        self.say(json!({
            "jsonrpc": "2.0", "id": CALL, "method": "tools/call",
            "params": {"name": "request_network", "arguments": {"host": host, "why": why}},
        }))
    }

    /// The one line the tool answered with, and whether it was an error
    ///
    /// # Errors
    ///
    /// The door closes before answering, or its answer is not a tool result.
    pub fn answered(&mut self) -> io::Result<(String, bool)> {
        let reply = self.line()?;
        if reply["id"] != json!(CALL) {
            return Err(not_an_answer(&reply));
        }
        let result = &reply["result"];
        let Some(line) = result["content"][0]["text"].as_str() else {
            return Err(not_an_answer(&reply));
        };
        Ok((
            line.to_owned(),
            result["isError"].as_bool().unwrap_or(false),
        ))
    }

    fn say(&mut self, message: Value) -> io::Result<()> {
        writeln!(self.to_door, "{message}")
    }

    /// The next whole line the door carried, or an error when it carried none in [`WAIT`]
    fn line(&mut self) -> io::Result<Value> {
        let mut line = String::new();
        if self.from_door.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the door answered nothing",
            ));
        }
        serde_json::from_str(&line).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{error}: {}", line.trim_end()),
            )
        })
    }
}

fn not_an_answer(reply: &Value) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("not a tool answer: {reply}"),
    )
}
