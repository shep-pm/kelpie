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

/// A newer version was ready before this session ever started, so its first
/// send swaps to it. It cannot resume the session the older one made, so the
/// session goes back to the version before, resumes there, and the newer one is
/// marked bad
#[test]
fn a_first_send_on_a_ready_new_version_falls_back_when_it_cannot_resume() {
    let old = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("on the one before"));
    // The newer version has never seen this session
    let new = FakeAgent::new().can_resume();
    let bridge = FakeBridge::new("0.101.1")
        .agent("0.101.1", old.clone())
        .agent("0.102.0", new.clone())
        .ready("0.102.0");
    let path = log_path();
    let session = Session::on_bridge(
        bridge.bridge(),
        PathBuf::from("/project"),
        Some("fake-1".into()),
        Choice::default(),
    )
    .with_log(log::Log::open(&path).unwrap());

    assert_eq!(
        say(&session, "first"),
        [started("fake-1"), text("on the one before"), ended()]
    );

    assert_eq!(
        bridge.started(),
        [version("0.102.0"), version("0.101.1")],
        "the first send swapped to the newer version and went back"
    );
    assert_eq!(bridge.in_use(), version("0.101.1"));
    assert_eq!(session.bridge_version(), Some(version("0.101.1")));
    let bad = bridge.bad();
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert_eq!(bad[0].0, version("0.102.0"));
    assert_eq!(
        new.methods(),
        ["initialize", "session/resume"],
        "the newer version was asked to resume the session"
    );
    assert_eq!(
        log::read(&path).unwrap(),
        [
            user("first"),
            fell_back("0.102.0", "0.101.1"),
            event(started("fake-1")),
            event(text("on the one before")),
            event(ended()),
        ]
    );
}

/// The harness the bridge drives is no longer there, which is not the bridge
/// version's failure: the turn ends with what it said, no version is marked
/// bad, and the session stays on the version it was running on
#[test]
fn a_harness_that_is_not_there_marks_no_bridge_version_bad() {
    let old = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("first"));
    // The newer version runs one turn and then its process is gone, so the
    // next send has to start it again
    let new = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("on the new one").exit());
    let bridge = FakeBridge::new("0.101.1")
        .agent("0.101.1", old.clone())
        .agent("0.102.0", new.clone());
    let session = on(bridge.bridge());

    assert_eq!(
        say(&session, "first"),
        [started("fake-1"), text("first"), ended()]
    );
    let bridge = bridge.ready("0.102.0");
    let (sink, heard) = events();
    session.send("second", sink);
    assert_eq!(next(&heard), started("fake-1"));
    assert_eq!(next(&heard), text("on the new one"));
    assert!(
        matches!(next(&heard), SessionEvent::Failed { .. }),
        "the newer version's process did not end the turn"
    );
    assert_eq!(bridge.in_use(), version("0.102.0"));

    // A removed login, or a `claude` taken off PATH, leaves the bridge version
    // installed and its harness unrunnable
    let bridge = bridge.without("0.102.0");
    let (sink, heard) = events();
    session.send("third", sink);
    let SessionEvent::Failed { message } = next(&heard) else {
        panic!("a harness that is not there did not end the turn with Failed");
    };
    assert!(message.contains("0.102.0"), "{message}");

    assert_eq!(
        bridge.bad(),
        [],
        "the bridge version is not blamed for its harness"
    );
    assert_eq!(bridge.in_use(), version("0.102.0"));
    assert_eq!(bridge.previous(), Some(version("0.101.1")));
    assert_eq!(session.bridge_version(), Some(version("0.102.0")));
}

/// A bridge that says the harness behind it is too old ends the turn with what
/// the person can run, and does not mark the bridge version bad or go back
#[test]
fn a_too_old_harness_shows_its_update_command_and_marks_no_version_bad() {
    let old = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("first"));
    let new = FakeAgent::new()
        .fails_initialize("Claude Code 1.0.0 is too old: this bridge needs 2.0.0 or later")
        .says_outdated(&["too old"], "claude update");
    let bridge = FakeBridge::new("0.101.1")
        .agent("0.101.1", old.clone())
        .agent("0.102.0", new.clone());
    let session = on(bridge.bridge());

    assert_eq!(
        say(&session, "first"),
        [started("fake-1"), text("first"), ended()]
    );
    let bridge = bridge.ready("0.102.0");

    let (sink, heard) = events();
    session.send("second", sink);
    let SessionEvent::Failed { message } = next(&heard) else {
        panic!("a bridge that refused its harness did not end the turn with Failed");
    };
    assert!(message.contains("claude update"), "{message}");
    assert!(
        message.contains("too old: this bridge needs 2.0.0"),
        "the bridge's own words are gone: {message}"
    );

    assert_eq!(
        bridge.bad(),
        [],
        "a harness the person must update is not the bridge version's failure"
    );
    assert_eq!(
        bridge.in_use(),
        version("0.102.0"),
        "the swap stands until the person updates the harness"
    );
    assert_eq!(new.launches(), 1);
}

fn user(text: &str) -> LogEntry {
    LogEntry::User {
        text: text.to_owned(),
    }
}

fn event(event: SessionEvent) -> LogEntry {
    LogEntry::Event(event)
}
