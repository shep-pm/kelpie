//! The route from an agent to kelpie's tools, through a relay and one door

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, BufRead, BufReader, Write, pipe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::network::REQUEST_NETWORK;
use super::relay::{is_relay, relay, settings};
use super::*;
use crate::session::fake::{FakeAgent, Turn};
use crate::session::{Choice, SandboxAnswer, SessionEvent};

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(10);

/// A folder of its own under the temporary folder, removed when it drops
///
/// Named short, since on macOS a socket's path may be 103 bytes at most.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let folder = env::temp_dir().join(format!(
            "kt{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("the scratch folder is made");
        Self(folder)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A relay run on a thread of its own, with the agent's end of its stdio
struct Relayed {
    to_relay: io::PipeWriter,
    lines: Receiver<String>,
    ended: JoinHandle<io::Result<()>>,
}

fn relayed(door: &Door, token: &str) -> Relayed {
    let (input, to_relay) = pipe().unwrap();
    let (from_relay, output) = pipe().unwrap();
    let (door, token) = (door.clone(), token.to_owned());
    let ended = thread::spawn(move || relay(&door, &token, input, output));
    let (sender, lines) = channel();
    thread::spawn(move || {
        for line in BufReader::new(from_relay).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    Relayed {
        to_relay,
        lines,
        ended,
    }
}

impl Relayed {
    fn say(&mut self, message: &Value) {
        writeln!(self.to_relay, "{message}").unwrap();
    }

    fn hear(&self) -> Value {
        let line = self
            .lines
            .recv_timeout(WAIT)
            .expect("the relay carried nothing back");
        serde_json::from_str(&line).unwrap_or_else(|error| panic!("{error}: {line}"))
    }

    /// Open the MCP conversation: `initialize`, answered, then `initialized`
    fn open(&mut self) -> Value {
        self.say(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"},
            },
        }));
        let reply = self.hear();
        self.say(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        reply
    }

    /// Close the agent's end, and wait for the relay to end
    fn finish(self) -> io::Result<()> {
        drop(self.to_relay);
        self.ended.join().unwrap()
    }
}

/// Wait until `done` holds
fn until(what: &str, done: impl Fn() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < WAIT, "{what}");
        thread::sleep(Duration::from_millis(10));
    }
}

fn started(scratch: &Scratch) -> Tools {
    Tools::start(&scratch.0).expect("the door opens")
}

#[test]
fn the_relay_carries_mcp_to_kelpie_and_back() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let session = tools.session();
    let mut relayed = relayed(session.door(), session.token());

    let initialized = relayed.open();
    relayed.say(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let listed = relayed.hear();

    assert_eq!(initialized["id"], 1);
    assert!(
        initialized["result"]["capabilities"]["tools"].is_object(),
        "{initialized}"
    );
    assert_eq!(listed["id"], 2);
    let names: Vec<&Value> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| &tool["name"])
        .collect();
    assert_eq!(names, [REQUEST_NETWORK], "{listed}");
    relayed
        .finish()
        .expect("the relay ends once its input does");
}

#[test]
fn a_relay_without_a_live_token_is_refused() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let dropped = tools.session();
    let (door, old) = (dropped.door().clone(), dropped.token().to_owned());
    drop(dropped);

    for (what, token) in [
        ("a wrong token", "0".repeat(32)),
        ("a token that was never given", String::new()),
        ("a dropped session's token", old),
    ] {
        let mut relayed = relayed(&door, &token);
        // Whatever it asks, nothing answers
        let _ = relayed
            .to_relay
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
        let error = relayed
            .ended
            .join()
            .unwrap()
            .expect_err(&format!("{what} was served"));
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted, "{what}");
        assert!(relayed.lines.try_recv().is_err(), "{what} got an answer");
    }
    assert!(tools.connected().is_empty());
}

#[test]
fn a_relay_with_no_token_in_its_environment_says_which_is_missing() {
    let environment = |door: Option<&str>, token: Option<&str>| {
        let (door, token) = (door.map(str::to_owned), token.map(str::to_owned));
        move |name: &str| match name {
            DOOR_VAR => door.clone(),
            TOKEN_VAR => token.clone(),
            _ => None,
        }
    };

    let no_token = settings(environment(Some("127.0.0.1:9"), None)).unwrap_err();
    let empty_token = settings(environment(Some("127.0.0.1:9"), Some(""))).unwrap_err();
    let no_door = settings(environment(None, Some("abc"))).unwrap_err();
    let bad_door = settings(environment(Some("not a door"), Some("abc"))).unwrap_err();
    let (door, token) = settings(environment(Some("127.0.0.1:9"), Some("abc"))).unwrap();

    assert!(no_token.to_string().contains(TOKEN_VAR), "{no_token}");
    assert!(empty_token.to_string().contains(TOKEN_VAR), "{empty_token}");
    assert!(no_door.to_string().contains(DOOR_VAR), "{no_door}");
    assert_eq!(bad_door.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(
        (door.to_string(), token.as_str()),
        ("127.0.0.1:9".to_owned(), "abc")
    );
}

#[test]
fn the_token_says_which_session_is_asking() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let (first, second) = (tools.session(), tools.session());
    assert_ne!(first.key(), second.key());
    assert_ne!(first.token(), second.token());

    let mut one = relayed(first.door(), first.token());
    one.open();
    until("the first session never showed as connected", || {
        tools.connected() == [first.key()]
    });
    let mut other = relayed(second.door(), second.token());
    other.open();
    until("the second session never showed as connected", || {
        let mut connected = tools.connected();
        connected.sort();
        connected == [first.key(), second.key()]
    });

    // One session's token does not act as another's: ending the first leaves the second
    drop(first);
    assert!(one.finish().is_ok());
    until("the first session stayed connected after it ended", || {
        tools.connected() == [second.key()]
    });
    other.say(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    assert_eq!(other.hear()["id"], 2);
}

#[test]
fn dropping_a_sessions_tools_ends_its_connection() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let session = tools.session();
    let mut relayed = relayed(session.door(), session.token());
    relayed.open();

    drop(session);

    // The relay ends though its input is still open: kelpie closed the door's end
    let ended = relayed.ended;
    until("the relay outlived its session's tools", || {
        ended.is_finished()
    });
    assert!(ended.join().unwrap().is_ok());
}

#[test]
fn a_session_dropped_before_its_harness_says_hello_is_no_longer_served() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let session = tools.session();
    let key = session.key();
    // The relay is connected with a live token, and its harness says nothing
    let silent = relayed(session.door(), session.token());
    until("the connection was never counted", || {
        tools.connected() == [key]
    });

    drop(session);

    until("a dropped session was still reported as served", || {
        tools.connected().is_empty()
    });
    drop(silent);
}

#[test]
fn a_harness_that_never_says_hello_is_let_go() {
    let scratch = Scratch::new();
    let tools = Tools::start_within(&scratch.0, Duration::from_millis(200)).unwrap();
    let session = tools.session();
    let silent = relayed(session.door(), session.token());
    until("the connection was never counted", || {
        !tools.connected().is_empty()
    });

    // Nothing is sent, and the session's tools stay live
    until("a connection that never said hello was kept", || {
        tools.connected().is_empty()
    });

    let ended = silent.ended;
    until("the relay outlived the deadline", || ended.is_finished());
    drop(session);
}

#[test]
fn dropping_the_tools_closes_the_door() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let session = tools.session();
    let door = session.door().clone();
    let mut relayed = relayed(&door, session.token());
    relayed.open();

    drop(tools);

    let ended = relayed.ended;
    until("the relay outlived the door", || ended.is_finished());
    assert!(
        door.connect().is_err(),
        "something still answers at {door} after the tools dropped"
    );
}

#[test]
fn the_server_is_handed_on_new_load_and_resume() {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let session_tools = tools.session();
    let news = FakeAgent::new();
    let resumes = FakeAgent::new().can_resume().knows("old");
    let loads = FakeAgent::new().can_load().knows("old");

    let say = |agent: &FakeAgent, resume: Option<&str>| {
        let session = agent
            .session(
                PathBuf::from("/project"),
                resume.map(Into::into),
                Choice::default(),
            )
            .with_tools(session_tools.clone());
        let (sender, heard) = channel();
        session.send("Hello", move |event| {
            let _ = sender.send(event);
        });
        while !matches!(
            heard.recv_timeout(WAIT).unwrap(),
            SessionEvent::TurnEnded { .. }
        ) {}
    };
    say(&news, None);
    say(&resumes, Some("old"));
    say(&loads, Some("old"));

    let wanted = json!([{
        "name": SERVER_NAME,
        "command": std::env::current_exe().unwrap(),
        "args": ["tools"],
        "env": [
            {"name": DOOR_VAR, "value": session_tools.door().to_string()},
            {"name": TOKEN_VAR, "value": session_tools.token()},
        ],
    }]);
    assert_eq!(news.params("session/new")[0]["mcpServers"], wanted);
    assert_eq!(resumes.params("session/resume")[0]["mcpServers"], wanted);
    assert_eq!(loads.params("session/load")[0]["mcpServers"], wanted);
}

#[test]
fn a_session_with_no_tools_hands_its_harness_no_server() {
    let agent = FakeAgent::new();
    let session = agent.session(PathBuf::from("/project"), None, Choice::default());
    let (sender, heard) = channel();

    session.send("Hello", move |event| {
        let _ = sender.send(event);
    });
    while !matches!(
        heard.recv_timeout(WAIT).unwrap(),
        SessionEvent::TurnEnded { .. }
    ) {}

    assert_eq!(agent.params("session/new")[0]["mcpServers"], json!([]));
}

#[test]
fn the_relay_runs_only_as_kelpie_tools() {
    let args = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();

    assert!(is_relay(args(&["kelpie", "tools"])));
    for other in [
        &["kelpie"][..],
        &["kelpie", "tools", "extra"],
        &["kelpie", "toolsx"],
        &["kelpie", "--tools"],
        &["kelpie", "Tools"],
        &["tools"],
        &[],
    ] {
        assert!(!is_relay(args(other)), "{other:?} started the relay");
    }
    // Run as the test binary is, it returns at once instead of taking over
    run_from_args();
}

#[test]
fn the_door_is_one_address_the_policy_can_name() {
    let socket = Door::parse("/data/kelpie/0123.sock").unwrap();
    let tcp = Door::parse("127.0.0.1:4242").unwrap();

    assert_eq!(socket.socket(), Some(Path::new("/data/kelpie/0123.sock")));
    assert_eq!(socket.tcp(), None);
    assert_eq!(tcp.tcp(), Some("127.0.0.1:4242".parse().unwrap()));
    assert_eq!(tcp.socket(), None);
    assert_eq!(tcp.to_string(), "127.0.0.1:4242");
    assert_eq!(socket.to_string(), "/data/kelpie/0123.sock");
    assert!(Door::parse("kelpie").is_err());
}

#[test]
fn the_door_is_the_platforms_own() {
    let scratch = Scratch::new();
    let tools = started(&scratch);

    let door = tools.door();

    if cfg!(target_os = "macos") {
        assert!(door.socket().is_some_and(Path::exists), "{door}");
    } else {
        let address = door.tcp().expect("a TCP door");
        assert!(
            address.ip().is_loopback() && address.ip().is_ipv4(),
            "{door}"
        );
        assert_ne!(address.port(), 0);
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! The socket the door is on macOS

    use super::*;
    use crate::tools::door::LONGEST_SOCKET;

    #[test]
    fn the_socket_path_fits_the_macos_limit() {
        let scratch = Scratch::new();
        // A data folder whose own path would put the socket past the limit
        let long = scratch.0.join("a".repeat(60)).join("b".repeat(60));
        fs::create_dir_all(&long).unwrap();

        let tools = Tools::start(&long).unwrap();

        let socket = tools.door().socket().unwrap().to_owned();
        assert!(!socket.starts_with(&long), "{}", socket.display());
        assert!(
            socket.as_os_str().len() <= LONGEST_SOCKET,
            "{}",
            socket.display()
        );
        // It binds and answers, which a path past the limit could not
        let session = tools.session();
        let mut relayed = relayed(session.door(), session.token());
        assert_eq!(relayed.open()["id"], 1);
    }

    #[test]
    fn a_short_data_folder_keeps_the_socket_in_it() {
        let scratch = Scratch::new();
        // A temporary folder that is itself long (a CI runner's) leaves no room
        // for the name, and the socket goes to the temporary folder instead
        let fits = scratch.0.as_os_str().len() + "/".len() + 32 + ".sock".len() <= LONGEST_SOCKET;

        let tools = started(&scratch);

        let socket = tools.door().socket().unwrap();
        let wanted = if fits {
            scratch.0.clone()
        } else {
            env::temp_dir()
        };
        assert_eq!(socket.parent(), Some(wanted.as_path()));
        let name = socket.file_name().unwrap().to_string_lossy();
        let stem = name.strip_suffix(".sock").expect("a .sock file");
        let stem = stem.strip_prefix("kelpie-").unwrap_or(stem);
        assert_eq!(stem.len(), 32);
        assert!(stem.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn the_next_run_binds_the_same_socket_and_replaces_one_a_killed_run_left() {
        let scratch = Scratch::new();
        let first = started(&scratch);
        let socket = first.door().socket().unwrap().to_owned();
        drop(first);
        // As a killed run leaves it: the file stays, nothing listens
        fs::write(&socket, b"left behind").unwrap();

        let second = started(&scratch);

        assert_eq!(second.door().socket(), Some(socket.as_path()));
        let session = second.session();
        let mut relayed = relayed(session.door(), session.token());
        assert_eq!(relayed.open()["id"], 1);
    }

    #[test]
    fn dropping_the_tools_removes_the_socket() {
        let scratch = Scratch::new();
        let tools = started(&scratch);
        let socket = tools.door().socket().unwrap().to_owned();
        assert!(socket.exists());

        drop(tools);

        assert!(!socket.exists(), "{} was left behind", socket.display());
    }
}

/// The hosts a sandbox lets through, as a list
#[derive(Debug, Default)]
struct Hosts(Mutex<Vec<String>>);

impl Hosts {
    fn allowing(hosts: &[&str]) -> Arc<Self> {
        Arc::new(Self(Mutex::new(
            hosts.iter().map(|host| (*host).to_owned()).collect(),
        )))
    }

    fn list(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl Reach for Hosts {
    fn allows(&self, host: &str) -> bool {
        self.list().iter().any(|allowed| allowed == host)
    }

    fn allow(&self, host: &str) {
        self.0.lock().unwrap().push(host.to_owned());
    }
}

/// A session's agent calling kelpie's `request_network`, with the turn's sink and
/// the sandbox standing by
struct Asking {
    relayed: Relayed,
    events: Receiver<SessionEvent>,
    hosts: Arc<Hosts>,
    session: SessionTools,
    // Dropped last: the door and the scratch folder it sits in outlive the rest
    _tools: Tools,
    _scratch: Scratch,
}

/// The agent's end, in a turn, on a sandbox that already lets `allowed` through
fn asking(allowed: &[&str]) -> Asking {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let session = tools.session();
    let hosts = Hosts::allowing(allowed);
    let (sender, events) = channel();
    session.asks().bind(hosts.clone());
    session
        .asks()
        .emit_to(Some(Arc::new(move |event| sender.send(event).is_ok())));
    let mut relayed = relayed(session.door(), session.token());
    relayed.open();
    Asking {
        relayed,
        events,
        hosts,
        session,
        _tools: tools,
        _scratch: scratch,
    }
}

impl Asking {
    /// The agent calls `request_network`
    fn calls(&mut self, arguments: &Value) {
        self.relayed.say(&json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": {"name": REQUEST_NETWORK, "arguments": arguments},
        }));
    }

    /// The agent asks for `host`, and says why
    fn asks_for(&mut self, host: &str, why: &str) {
        self.calls(&json!({"host": host, "why": why}));
    }

    /// The request the maintainer is shown: its id, host and why
    fn shown(&self) -> (String, String, String) {
        match self.events.recv_timeout(WAIT) {
            Ok(SessionEvent::SandboxRequest { id, host, why }) => (id, host, why),
            other => panic!("no request was shown: {other:?}"),
        }
    }

    /// What the agent hears back, as the tool's one line, and whether it is an error
    fn heard(&self) -> (String, bool) {
        let reply = self.relayed.hear();
        assert_eq!(reply["id"], 7, "{reply}");
        let result = &reply["result"];
        (
            result["content"][0]["text"].as_str().unwrap().to_owned(),
            result["isError"].as_bool().unwrap_or(false),
        )
    }
}

#[test]
fn request_network_waits_for_the_answer() {
    let mut agent = asking(&[]);

    agent.asks_for("docs.rs", "to read the crate's docs");

    let (id, host, why) = agent.shown();
    assert_eq!(
        (host.as_str(), why.as_str()),
        ("docs.rs", "to read the crate's docs")
    );
    // Nothing is said until the maintainer answers
    assert!(agent.relayed.lines.try_recv().is_err());
    assert!(agent.session.asks().answer(&id, SandboxAnswer::Once));
    assert_eq!(
        agent.heard(),
        ("docs.rs is allowed for this session".to_owned(), false)
    );
    assert_eq!(agent.hosts.list(), ["docs.rs"]);
}

#[test]
fn allow_for_the_project_tells_the_agent_it_was_saved() {
    let mut agent = asking(&[]);
    agent.asks_for("docs.rs", "to read the crate's docs");
    let (id, ..) = agent.shown();

    agent.session.asks().answer(&id, SandboxAnswer::Project);

    assert_eq!(
        agent.heard().0,
        "docs.rs is allowed and saved for this project"
    );
    assert_eq!(agent.hosts.list(), ["docs.rs"]);
}

#[test]
fn deny_changes_nothing() {
    let mut agent = asking(&["api.anthropic.com"]);
    agent.asks_for("docs.rs", "to read the crate's docs");
    let (id, ..) = agent.shown();

    agent.session.asks().answer(&id, SandboxAnswer::Deny);

    assert_eq!(
        agent.heard(),
        ("denied by the maintainer".to_owned(), false)
    );
    assert_eq!(agent.hosts.list(), ["api.anthropic.com"]);
}

#[test]
fn a_host_already_allowed_asks_nothing() {
    let mut agent = asking(&["docs.rs"]);

    agent.asks_for("Docs.rs", "to read the crate's docs");

    assert_eq!(
        agent.heard(),
        ("docs.rs is already allowed".to_owned(), false)
    );
    assert!(agent.events.try_recv().is_err(), "the maintainer was asked");
}

#[test]
fn a_url_or_port_is_not_a_host() {
    let mut agent = asking(&[]);

    for (given, form) in [
        ("https://docs.rs/serde", "a URL"),
        ("docs.rs/serde", "a path"),
        ("docs.rs:8080", "a port"),
        ("[::1]:8080", "a port"),
    ] {
        agent.asks_for(given, "to read the crate's docs");

        let (line, failed) = agent.heard();
        assert!(failed, "{given} was taken as a host: {line}");
        assert!(line.contains(form), "{given}: {line}");
        assert!(agent.events.try_recv().is_err(), "{given}: asked anyway");
    }
    // The control: a host alone is asked about, so the refusals above are the form's
    agent.asks_for("*.docs.rs", "to read the crate's docs");
    assert_eq!(agent.shown().1, "*.docs.rs");
}

#[test]
fn a_call_that_is_not_a_request_is_told_what_it_lacks() {
    let mut agent = asking(&[]);

    agent.calls(&json!({"host": "docs.rs"}));

    let (line, failed) = agent.heard();
    assert!(failed && line.contains("`why`"), "{line}");
}

/// The relay's end of a session whose agent is inside a turn that waits for its cancel
struct InATurn {
    asking: Asking,
    session: crate::session::Session,
    sink: Receiver<SessionEvent>,
    _agent: FakeAgent,
}

fn in_a_turn() -> InATurn {
    let scratch = Scratch::new();
    let tools = started(&scratch);
    let session_tools = tools.session();
    let hosts = Hosts::allowing(&[]);
    session_tools.asks().bind(hosts.clone());
    let agent = FakeAgent::new().turn(Turn::new().until_cancelled());
    let session = agent
        .session(PathBuf::from("/project"), None, Choice::default())
        .with_tools(session_tools.clone());
    let (sender, sink) = channel();
    session.send("Go", move |event| {
        let _ = sender.send(event);
    });
    // The harness has opened the session: the turn is in flight
    while !matches!(
        sink.recv_timeout(WAIT).unwrap(),
        SessionEvent::Started { .. }
    ) {}
    let mut relayed = relayed(session_tools.door(), session_tools.token());
    relayed.open();
    InATurn {
        asking: Asking {
            relayed,
            events: channel().1,
            hosts,
            session: session_tools,
            _tools: tools,
            _scratch: scratch,
        },
        session,
        sink,
        _agent: agent,
    }
}

impl InATurn {
    /// The request the turn's sink was handed
    fn shown(&self) -> String {
        loop {
            if let SessionEvent::SandboxRequest { id, .. } = self.sink.recv_timeout(WAIT).unwrap() {
                return id;
            }
        }
    }
}

#[test]
fn an_interrupt_answers_denied() {
    let mut turn = in_a_turn();
    turn.asking.asks_for("docs.rs", "to read the crate's docs");
    let id = turn.shown();

    turn.session.cancel();

    assert_eq!(turn.asking.heard().0, "denied by the maintainer");
    assert!(
        !turn.session.answer(&id, SandboxAnswer::Once),
        "a cancelled request was still waiting"
    );
    assert!(turn.asking.hosts.list().is_empty());
}

#[test]
fn a_dropped_session_answers_denied() {
    let mut turn = in_a_turn();
    turn.asking.asks_for("docs.rs", "to read the crate's docs");
    turn.shown();

    drop(turn.session);

    assert_eq!(turn.asking.heard().0, "denied by the maintainer");
    assert!(turn.asking.hosts.list().is_empty());
}

#[test]
fn a_turn_that_ended_answers_what_it_left_waiting_denied() {
    let mut turn = in_a_turn();
    turn.asking.asks_for("docs.rs", "to read the crate's docs");
    let id = turn.shown();

    // The agent ended its turn with the call still open
    turn.session.cancel();
    while !matches!(
        turn.sink.recv_timeout(WAIT).unwrap(),
        SessionEvent::TurnEnded { .. }
    ) {}

    assert_eq!(turn.asking.heard().0, "denied by the maintainer");
    assert!(!turn.session.answer(&id, SandboxAnswer::Once));
}

#[test]
fn allow_once_widens_the_running_sandbox() {
    let mut turn = in_a_turn();
    turn.asking.hosts.allow("api.anthropic.com");
    turn.asking.asks_for("docs.rs", "to read the crate's docs");
    let id = turn.shown();
    assert!(
        !turn.asking.hosts.allows("docs.rs"),
        "refused before the answer"
    );

    assert!(turn.session.answer(&id, SandboxAnswer::Once));

    assert_eq!(turn.asking.heard().0, "docs.rs is allowed for this session");
    assert_eq!(turn.asking.hosts.list(), ["api.anthropic.com", "docs.rs"]);
}
