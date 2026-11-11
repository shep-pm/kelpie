//! The agent's side of kelpie's tools, for a test
//!
//! [`FakeTools`] dials a session's door with its token and calls its tools over
//! MCP, as a harness's relay and the agent behind it do
//! (`docs/design/kelpie-tools.md`, The route out). A test of a session, or of a
//! command that answers one, needs no MCP client and no door code of its own.

use std::io::{self, BufRead, BufReader, Write};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use super::SessionTools;
use super::door::Stream;
use super::network::REQUEST_NETWORK;
use super::start_worker::START_WORKER;

/// How long a fake waits for a line kelpie should have sent
const WAIT: Duration = Duration::from_secs(10);

/// The id of every call a fake makes, so the line it reads back is its own
const CALL: u64 = 1;

/// The id of the `tools/list` a fake makes
const LIST: u64 = 2;

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
        self.call(REQUEST_NETWORK, json!({"host": host, "why": why}))
    }

    /// Call `start_worker` on `ticket`, as text or as a number
    ///
    /// # Errors
    ///
    /// The door closes before taking the call.
    pub fn start_worker(&mut self, ticket: impl Serialize) -> io::Result<()> {
        self.call(START_WORKER, json!({"ticket": ticket}))
    }

    /// Call `name` with `arguments`, as the agent's client sends them
    ///
    /// # Errors
    ///
    /// The door closes before taking the call.
    pub fn call(&mut self, name: &str, arguments: Value) -> io::Result<()> {
        self.say(json!({
            "jsonrpc": "2.0", "id": CALL, "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }))
    }

    /// The tools kelpie lists for this session, in the order it lists them
    ///
    /// # Errors
    ///
    /// The door closes before answering, or its answer is not a tool list.
    pub fn listed(&mut self) -> io::Result<Vec<String>> {
        self.say(json!({"jsonrpc": "2.0", "id": LIST, "method": "tools/list"}))?;
        let reply = self.line()?;
        if reply["id"] != json!(LIST) {
            return Err(not_an_answer(&reply));
        }
        let Some(tools) = reply["result"]["tools"].as_array() else {
            return Err(not_an_answer(&reply));
        };
        let mut names = Vec::new();
        for tool in tools {
            names.push(
                tool["name"]
                    .as_str()
                    .ok_or_else(|| not_an_answer(&reply))?
                    .to_owned(),
            );
        }
        Ok(names)
    }

    /// The one line the tool answered with, and whether it was an error
    ///
    /// A tool name the session is not given is answered as a refused request, so
    /// its message is read as the error line it is to the agent.
    ///
    /// # Errors
    ///
    /// The door closes before answering, or its answer is not a tool result.
    pub fn answered(&mut self) -> io::Result<(String, bool)> {
        let reply = self.line()?;
        if reply["id"] != json!(CALL) {
            return Err(not_an_answer(&reply));
        }
        if let Some(message) = reply["error"]["message"].as_str() {
            return Ok((message.to_owned(), true));
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
