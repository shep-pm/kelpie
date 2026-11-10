//! `kelpie tools`: the stdio server a harness starts, which carries MCP to kelpie
//!
//! The harness runs this binary as `kelpie tools` with the door and the token in
//! its environment. The relay sends the token first, then copies bytes between
//! its stdin and stdout and the [`Door`]. It parses no MCP, so it needs no change
//! when a tool is added, and it never decides what a session may call: kelpie
//! does, from the token.
//!
//! Lifted from SK's `connect` (`crates/core/src/bridge.rs` at `1dc51c040`),
//! without the request-id tracking and the browser's tool filter, which belong
//! to a relay that reads MCP.

use std::env;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::thread;

use super::door::Door;

/// The variable holding the door: a socket's path, or `host:port`
pub const DOOR_VAR: &str = "KELPIE_TOOLS_DOOR";

/// The variable holding the session's token
pub const TOKEN_VAR: &str = "KELPIE_TOOLS_TOKEN";

/// The one argument that makes the binary the relay
const ARG: &str = "tools";

/// Takes over the process when the binary was started as `kelpie tools`
///
/// A host calls this in `main` after the sandbox's own `run_from_args` and
/// before anything else of its own. It runs the relay and exits when the
/// arguments are exactly `tools`, and returns at once for any other run.
pub fn run_from_args() {
    if !is_relay(env::args_os()) {
        return;
    }
    let result = settings(|name| env::var(name).ok())
        .and_then(|(door, token)| relay(&door, &token, io::stdin(), io::stdout()));
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            eprintln!("kelpie tools: {error}");
            std::process::exit(1);
        }
    }
}

/// Whether `args`, the program's first, then the rest, are exactly `kelpie tools`
pub(super) fn is_relay(args: impl IntoIterator<Item = OsString>) -> bool {
    let mut args = args.into_iter().skip(1);
    args.next().is_some_and(|arg| arg == ARG) && args.next().is_none()
}

/// The door and the token, from the relay's environment
///
/// # Errors
///
/// The variable that is missing or empty, or the door text that is not one.
pub(super) fn settings(var: impl Fn(&str) -> Option<String>) -> io::Result<(Door, String)> {
    let get = |name: &str| {
        var(name).filter(|value| !value.is_empty()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{name} is not set, so there is no door or token to use"),
            )
        })
    };
    Ok((Door::parse(&get(DOOR_VAR)?)?, get(TOKEN_VAR)?))
}

/// Dial `door`, send `token`, and carry `input` to it and what it says to `output`
/// until the door closes
///
/// Ends when kelpie closes the door's connection. `input` ending tells kelpie
/// nothing more is coming, which it takes as the end of the conversation.
///
/// # Errors
///
/// The door cannot be dialled, or closes without a word: kelpie answers nothing
/// to a token that is not live.
pub(super) fn relay(
    door: &Door,
    token: &str,
    mut input: impl Read + Send + 'static,
    mut output: impl Write,
) -> io::Result<()> {
    let mut to_door = door
        .connect()
        .map_err(|error| io::Error::new(error.kind(), format!("cannot reach {door}: {error}")))?;
    let mut from_door = to_door.try_clone()?;
    writeln!(to_door, "{token}")?;
    // Not joined: it may be blocked on `input` when the door closes first
    thread::spawn(move || {
        let mut buf = [0_u8; 8192];
        while let Ok(read) = input.read(&mut buf) {
            if read == 0 || to_door.write_all(&buf[..read]).is_err() {
                break;
            }
        }
        let _ = to_door.finish_writing();
    });
    let mut buf = [0_u8; 8192];
    let mut answered = false;
    loop {
        let read = match from_door.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        answered = true;
        output.write_all(&buf[..read])?;
        output.flush()?;
    }
    if answered {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            format!("{door} closed without answering: the session's token is not live"),
        ))
    }
}
