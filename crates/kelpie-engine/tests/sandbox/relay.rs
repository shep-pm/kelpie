//! kelpie's tools door, reached from a relay running inside a sandbox
//!
//! The relay is this binary run as `kelpie tools`, so it is a child of
//! `tests/sandbox.rs`, which is its own `main` (crabpot's Linux helper is this
//! binary run again).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::thread;

use kelpie_engine::sandbox::{Reads, Sandbox};
use kelpie_engine::tools::{DOOR_VAR, SessionKey, SessionTools, TOKEN_VAR, Tools};
use serde_json::{Value, json};

use super::WAIT;
use super::host::working_sandbox_with_door;
use super::network::DIAL;
use super::support::Scratch;

/// The agent's end of an MCP conversation with the relay running in a sandbox
///
/// Returns what the server said, and the sessions kelpie was serving while the
/// relay was still up: once the relay is stopped its connection ends, so that
/// is read before.
fn mcp_through_the_sandbox(
    sandbox: &Sandbox,
    exe: &Path,
    tools: &Tools,
    session: &SessionTools,
) -> (Vec<Value>, Vec<SessionKey>) {
    let (door, token) = (session.door(), session.token());
    let mut relay = Command::new(exe);
    relay
        .arg("tools")
        .env(DOOR_VAR, door.to_string())
        .env(TOKEN_VAR, token);
    let mut child = sandbox
        .command(&relay)
        .expect("the relay goes in the sandbox")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the sandbox runs the relay");
    let mut to_relay = child.stdin.take().unwrap();
    let from_relay = child.stdout.take().unwrap();
    let (sender, lines) = channel();
    thread::spawn(move || {
        for line in BufReader::new(from_relay).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let hear = || {
        let line = lines
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("the relay carried nothing back from inside the sandbox"));
        serde_json::from_str::<Value>(&line).unwrap_or_else(|error| panic!("{error}: {line}"))
    };
    let mut say = |message: Value| writeln!(to_relay, "{message}").unwrap();
    say(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"},
        },
    }));
    let initialized = hear();
    say(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    say(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let listed = hear();
    let connected = tools.connected();
    drop(to_relay);
    let _ = child.kill();
    let _ = child.wait();
    (vec![initialized, listed], connected)
}

pub(super) fn the_relay_reaches_kelpie_from_inside_the_sandbox() {
    let scratch = Scratch::new("relay");
    let exe = std::env::current_exe().unwrap();
    let tools = Tools::start(&scratch.outside()).expect("the door opens");
    let session = tools.session();
    let Some(sandbox) = working_sandbox_with_door(
        &scratch,
        "the_relay_reaches_kelpie_from_inside_the_sandbox",
        &exe,
        Reads::Project,
        Some(session.door()),
    ) else {
        return;
    };

    let (heard, connected) = mcp_through_the_sandbox(&sandbox, &exe, &tools, &session);

    assert_eq!(heard[0]["id"], 1, "{}", heard[0]);
    assert!(heard[0]["result"]["capabilities"]["tools"].is_object());
    assert_eq!(heard[1]["id"], 2);
    let listed: Vec<&Value> = heard[1]["result"]["tools"]
        .as_array()
        .expect("a list of tools")
        .iter()
        .map(|tool| &tool["name"])
        .collect();
    assert_eq!(listed, ["request_network"], "{}", heard[1]);
    assert_eq!(
        connected,
        [session.key()],
        "the connection is the session's, by its token"
    );
}

pub(super) fn the_door_opens_nothing_else() {
    let scratch = Scratch::new("door");
    let exe = std::env::current_exe().unwrap();
    let tools = Tools::start(&scratch.outside()).expect("the door opens");
    let session = tools.session();
    let Some(sandbox) = working_sandbox_with_door(
        &scratch,
        "the_door_opens_nothing_else",
        &exe,
        Reads::Project,
        Some(session.door()),
    ) else {
        return;
    };
    let dial_in_sandbox = |address: &str| {
        let mut command = Command::new(&exe);
        command.args([DIAL, address]);
        sandbox
            .command(&command)
            .expect("the dial goes in the sandbox")
            .output()
            .expect("the sandbox runs the dial")
    };
    // The control: the door answers, so the refusals below are the policy's and
    // not a sandbox that dials nothing
    let control = dial_in_sandbox(&session.door().to_string());
    assert!(
        control.status.success(),
        "the door was not reached: {control:?}"
    );
    // Anything else on the loopback, which accepts if it is let through
    let other_port = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let other_port_address = other_port.local_addr().unwrap().to_string();
    let mut others = vec![("another loopback port", other_port_address)];
    // macOS names the door as a socket, so another socket is a test of its own
    let other_socket = scratch.outside().join("other.sock");
    let _listener = if session.door().socket().is_some() {
        let listener = UnixListener::bind(&other_socket).unwrap();
        others.push(("another socket", other_socket.display().to_string()));
        Some(listener)
    } else {
        None
    };

    for (what, address) in others {
        let refused = dial_in_sandbox(&address);

        assert!(
            !refused.status.success(),
            "{what} ({address}) was let through: {refused:?}"
        );
    }
}
