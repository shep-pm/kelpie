//! The lease socket: asks, grants, and the places given back when a connection closes

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::run::RunId;
use crate::settings::{ConfigDir, Settings};

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(10);

/// A folder of its own under the temporary folder, removed when it drops
///
/// Named short, since on macOS a socket's path may be 103 bytes at most.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let folder = std::env::temp_dir().join(format!("kelpie-lease-{}", short_run()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        Self(folder)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

/// A random folder name short enough that a session's `lease.sock` inside it is
/// still a path macOS will bind
fn short_run() -> String {
    RunId::new().unwrap().to_string()[..8].to_owned()
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The leases of a test, with `capacity` places for `cargo-test`
fn leases(dir: &Scratch, capacity: u32) -> Leases {
    let config = dir.path().join("config");
    let file = config.join("settings.json");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        &file,
        format!(r#"{{"lease-capacity": {{"cargo-test": {capacity}}}}}"#),
    )
    .unwrap();
    Leases::new(&Settings::load(&ConfigDir::at(&config), "web").unwrap())
}

/// The connection one knock makes: a Unix socket (macOS) or loopback TCP (Linux)
#[cfg(target_os = "macos")]
type Stream = std::os::unix::net::UnixStream;
/// The connection one knock makes
#[cfg(target_os = "linux")]
type Stream = std::net::TcpStream;

/// One connection to a session's lease socket, with the answers it is given
struct Knock {
    reader: BufReader<Stream>,
}

/// Connect to `door` and ask kelpie for `take`, keeping the connection open
fn knocked(door: &Door, take: &str) -> Knock {
    let stream = match (door.socket(), door.tcp()) {
        #[cfg(target_os = "macos")]
        (Some(path), _) => Stream::connect(path).unwrap(),
        #[cfg(target_os = "linux")]
        (_, Some(address)) => Stream::connect(address).unwrap(),
        _ => panic!("a lease door names a socket on macOS and an address on Linux"),
    };
    stream.set_read_timeout(Some(WAIT)).unwrap();
    let mut reader = BufReader::new(stream);
    let ask = Ask {
        take: take.to_owned(),
        pid: std::process::id(),
        what: "cargo test".to_owned(),
        running: false,
    };
    let line = serde_json::to_string(&ask).unwrap();
    reader.get_mut().write_all(line.as_bytes()).unwrap();
    reader.get_mut().write_all(b"\n").unwrap();
    Knock { reader }
}

impl Knock {
    /// Kelpie's next answer line
    fn answer(&mut self) -> Value {
        let mut line = String::new();
        let read = self.reader.read_line(&mut line).unwrap();
        assert_ne!(read, 0, "kelpie closed the socket without answering");
        serde_json::from_str(line.trim_end()).unwrap()
    }
}

/// How many places of `name` are held, as kelpie's lease status says
fn holders(leases: &Leases, name: &str) -> usize {
    leases
        .status()
        .iter()
        .find(|status| status.kind == name)
        .map_or(0, |status| status.holders.len())
}

/// Wait until nobody queues for `name`
///
/// A closed waiter leaves the queue when kelpie's task for its connection hears
/// the close, which is after the client's `drop` returns.
fn wait_for_an_empty_queue(leases: &Leases, name: &str) {
    let queued = || {
        leases
            .status()
            .iter()
            .find(|status| status.kind == name)
            .map_or(0, |status| status.queue.len())
    };
    let deadline = std::time::Instant::now() + WAIT;
    while queued() != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the closed waiter never left the queue"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn a_second_holder_waits_while_the_first_holds_it() {
    let dir = Scratch::new();
    let leases = leases(&dir, 1);
    let session = leases.session(&dir.path().join("w")).unwrap();

    let mut first = knocked(session.door(), "cargo-test");
    assert_eq!(first.answer(), json!("granted"));
    let mut second = knocked(session.door(), "cargo-test");
    assert_eq!(second.answer(), json!({"queued": 0}));
    assert_eq!(holders(&leases, "cargo-test"), 1);

    // The grant arrives when the holder's connection closes, however it closed
    drop(first);
    assert_eq!(second.answer(), json!("granted"));
    assert_eq!(holders(&leases, "cargo-test"), 1);
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn a_waiter_that_closes_loses_its_place_and_grants_nothing() {
    let dir = Scratch::new();
    let leases = leases(&dir, 1);
    let session = leases.session(&dir.path().join("w")).unwrap();

    let mut holder = knocked(session.door(), "cargo-test");
    assert_eq!(holder.answer(), json!("granted"));
    let mut waiter = knocked(session.door(), "cargo-test");
    assert_eq!(waiter.answer(), json!({"queued": 0}));
    drop(waiter);
    wait_for_an_empty_queue(&leases, "cargo-test");

    // The withdrawn ask granted nothing, and the next ask queues behind the holder
    let mut third = knocked(session.door(), "cargo-test");
    assert_eq!(third.answer(), json!({"queued": 0}));
    drop(holder);
    assert_eq!(third.answer(), json!("granted"));
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn a_name_that_is_no_lease_is_refused_naming_it() {
    let dir = Scratch::new();
    let leases = leases(&dir, 1);
    let session = leases.session(&dir.path().join("w")).unwrap();

    let mut knocked = knocked(session.door(), "Cargo Test");
    let answer = knocked.answer();
    assert!(
        answer["error"]
            .as_str()
            .is_some_and(|why| why.contains("Cargo Test")),
        "{answer}"
    );
    assert_eq!(
        holders(&leases, "cargo-test"),
        0,
        "a refused ask holds nothing"
    );
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn each_session_is_asked_at_its_own_socket() {
    let dir = Scratch::new();
    let leases = leases(&dir, 2);
    let one = leases.session(&dir.path().join("one")).unwrap();
    let other = leases.session(&dir.path().join("two")).unwrap();
    assert_ne!(one.door(), other.door(), "one socket per session");

    // Both sockets feed the same lease, so a holder in one session holds a place
    // the other session's commands queue behind
    let mut first = knocked(one.door(), "cargo-test");
    assert_eq!(first.answer(), json!("granted"));
    let mut second = knocked(other.door(), "cargo-test");
    assert_eq!(second.answer(), json!("granted"));
    let mut third = knocked(one.door(), "cargo-test");
    assert_eq!(third.answer(), json!({"queued": 0}));
    assert_eq!(holders(&leases, "cargo-test"), 2);
}

#[test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn a_session_that_ends_gives_back_every_place_its_connections_held() {
    let dir = Scratch::new();
    let leases = leases(&dir, 2);
    let session = leases.session(&dir.path().join("w")).unwrap();

    let mut held = knocked(session.door(), "cargo-test");
    assert_eq!(held.answer(), json!("granted"));
    assert_eq!(holders(&leases, "cargo-test"), 1);
    drop(session);
    drop(held);

    let deadline = std::time::Instant::now() + WAIT;
    while holders(&leases, "cargo-test") != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the place is still held"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The lease book every session's socket tasks share
fn book(leases: &Leases) -> std::sync::Arc<Shared> {
    std::sync::Arc::clone(&leases.shared)
}

/// A taker with `n` as its connection, for a book asked to by hand
fn taker(n: u64) -> Taker {
    Taker {
        connection: Connection(n),
        pid: u32::try_from(n).unwrap_or(1),
        what: "cargo test".to_owned(),
    }
}

#[test]
fn a_grant_made_while_the_queued_answer_is_written_still_wakes_the_waiter() {
    let dir = Scratch::new();
    let leases = leases(&dir, 1);
    let shared = book(&leases);
    let now = Timestamp::now();

    assert!(matches!(
        shared.ask("cargo-test", taker(1), false, now),
        Took::Granted
    ));
    let Took::Queued { ahead, wake } = shared.ask("cargo-test", taker(2), false, now) else {
        panic!("the second ask queues behind the holder");
    };
    assert_eq!(ahead, 0);
    assert!(!wake.is_cancelled(), "the waiter has not been granted yet");

    // The holder's connection closes while the waiter's queued answer is still
    // on its way out, which is the moment kelpie grants the waiter
    shared.leave("cargo-test", Connection(1), now);

    assert!(
        wake.is_cancelled(),
        "the waiter was granted, so it must hear it and not line up again"
    );
    assert_eq!(
        holders(&leases, "cargo-test"),
        1,
        "the place went to the waiter"
    );
}

#[test]
fn a_lease_name_is_lowercase_letters_digits_and_dashes() {
    for bad in [
        "",
        "Cargo-Test",
        "a.b",
        "a b",
        "a_b",
        &"x".repeat(LONGEST_NAME + 1),
    ] {
        assert_eq!(named(bad), None, "{bad:?}");
    }
    assert_eq!(named("cargo-test").as_deref(), Some("cargo-test"));
    assert_eq!(named("e2e").as_deref(), Some("e2e"));
}
