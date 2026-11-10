//! A session that moves to a newer bridge version at its next idle point, and
//! one that goes back when the new version fails it
//!
//! `docs/design/bridge-updates.md` (Swapping)

use semver::Version;

use super::*;
use crate::session::fake::{BRIDGE, FakeBridge};

/// A version, which every test here writes as a literal
fn version(text: &str) -> Version {
    Version::parse(text).unwrap()
}

/// A session on `bridge`, with nothing to resume
fn on(bridge: Bridge) -> Session {
    Session::on_bridge(bridge, PathBuf::from("/project"), None, Choice::default())
}

/// A log file's path in a folder of this test's own
fn log_path() -> PathBuf {
    data_folder().join("log.jsonl")
}

/// The divider a session leaves when it moves, as the design words it
fn moved(from: &str, to: &str) -> LogEntry {
    LogEntry::Divider {
        text: format!("Updated {BRIDGE} from {from} to {to}"),
    }
}

/// The divider a session leaves when a version fails it
fn fell_back(from: &str, to: &str) -> LogEntry {
    LogEntry::Divider {
        text: format!("{BRIDGE} {from} failed to resume, back on {to}"),
    }
}

/// A newer version is ready while the session sits idle, and its next send
/// starts on it and resumes the session there
#[test]
fn a_new_bridge_is_used_from_the_next_send() {
    let old = FakeAgent::new().turn(Turn::new().text("on the old one"));
    let new = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("on the new one"));
    let bridge = FakeBridge::new("0.101.1")
        .agent("0.101.1", old.clone())
        .agent("0.102.0", new.clone());
    let path = log_path();
    let session = on(bridge.bridge()).with_log(log::Log::open(&path).unwrap());

    assert_eq!(
        say(&session, "first"),
        [started("fake-1"), text("on the old one"), ended()]
    );
    assert_eq!(session.bridge_version(), Some(version("0.101.1")));

    // The bridge check finds and probes a newer version while the session is idle
    let bridge = bridge.ready("0.102.0");

    assert_eq!(
        say(&session, "second"),
        [started("fake-1"), text("on the new one"), ended()]
    );

    assert_eq!(
        bridge.started(),
        [version("0.101.1"), version("0.102.0")],
        "the session started again on the newer version"
    );
    assert_eq!(bridge.in_use(), version("0.102.0"));
    assert_eq!(bridge.previous(), Some(version("0.101.1")));
    assert_eq!(session.bridge_version(), Some(version("0.102.0")));
    assert_eq!(old.launches(), 1);
    assert_eq!(old.stopped(), 1, "the version it left stopped");
    assert_eq!(new.launches(), 1);
    assert!(
        new.methods().contains(&"session/resume".to_owned()),
        "the move did not resume the session: {:?}",
        new.methods()
    );
    assert_eq!(
        log::read(&path).unwrap(),
        [
            user("first"),
            event(started("fake-1")),
            event(text("on the old one")),
            event(ended()),
            user("second"),
            moved("0.101.1", "0.102.0"),
            event(started("fake-1")),
            event(text("on the new one")),
            event(ended()),
        ]
    );
}

/// A turn in flight is never interrupted: it finishes on the bridge it started
/// on, and so does a message that queued behind it. Only the next idle point moves
#[test]
fn a_running_turn_finishes_on_its_bridge() {
    let old = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("working").until_cancelled())
        .turn(Turn::new().text("queued behind it"));
    let new = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("on the new one"));
    let bridge = FakeBridge::new("0.101.1")
        .agent("0.101.1", old.clone())
        .agent("0.102.0", new.clone());
    let session = on(bridge.bridge());

    let (sink, first) = events();
    session.send("first", sink);
    assert_eq!(next(&first), started("fake-1"));
    assert_eq!(next(&first), text("working"));

    // A newer version is ready mid-turn, and a second message arrives before it ends
    let bridge = bridge.ready("0.102.0");
    let (sink, second) = events();
    session.send("second", sink);
    session.cancel();

    assert_eq!(
        next(&first),
        SessionEvent::TurnEnded {
            stop_reason: StopReason::Cancelled,
            usage: None,
        }
    );
    assert_eq!(next(&second), text("queued behind it"));
    assert_eq!(next(&second), ended());
    assert_eq!(new.launches(), 0, "the newer bridge started no harness");
    assert_eq!(bridge.started(), [version("0.101.1")]);
    assert_eq!(session.bridge_version(), Some(version("0.101.1")));

    // The session is idle now, so this send starts on the newer version
    assert_eq!(
        say(&session, "third"),
        [started("fake-1"), text("on the new one"), ended()]
    );
    assert_eq!(bridge.started(), [version("0.101.1"), version("0.102.0")]);
    assert_eq!(old.launches(), 1);
    assert_eq!(new.launches(), 1);
}

/// The newer version cannot open the session, so it is marked bad, the session
/// goes back to the version before, resumes there, and its message still runs
#[test]
fn a_failed_resume_falls_back_and_marks_the_version_bad() {
    let old = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("first"))
        .turn(Turn::new().text("again on the one before"));
    // The newer version has never seen this session, so it refuses to resume it
    let new = FakeAgent::new().can_resume();
    let bridge = FakeBridge::new("0.101.1")
        .agent("0.101.1", old.clone())
        .agent("0.102.0", new.clone());
    let path = log_path();
    let session = on(bridge.bridge()).with_log(log::Log::open(&path).unwrap());

    assert_eq!(
        say(&session, "first"),
        [started("fake-1"), text("first"), ended()]
    );
    let bridge = bridge.ready("0.102.0");

    assert_eq!(
        say(&session, "second"),
        [started("fake-1"), text("again on the one before"), ended()],
        "the message that the newer version could not run still ran"
    );

    assert_eq!(
        bridge.in_use(),
        version("0.101.1"),
        "the session is back on the version before"
    );
    assert_eq!(bridge.previous(), None);
    assert_eq!(session.bridge_version(), Some(version("0.101.1")));
    let bad = bridge.bad();
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert_eq!(bad[0].0, version("0.102.0"));
    assert!(
        bad[0].1.to_lowercase().contains("session"),
        "the reason is not what the version said: {:?}",
        bad[0].1
    );
    assert_eq!(new.launches(), 1);
    assert_eq!(
        old.launches(),
        2,
        "the session started again on the one before"
    );
    assert_eq!(
        log::read(&path).unwrap(),
        [
            user("first"),
            event(started("fake-1")),
            event(text("first")),
            event(ended()),
            user("second"),
            fell_back("0.102.0", "0.101.1"),
            event(started("fake-1")),
            event(text("again on the one before")),
            event(ended()),
        ]
    );
}

fn user(text: &str) -> LogEntry {
    LogEntry::User {
        text: text.to_owned(),
    }
}

fn event(event: SessionEvent) -> LogEntry {
    LogEntry::Event(event)
}
