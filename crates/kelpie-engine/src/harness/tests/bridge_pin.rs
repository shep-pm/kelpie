//! The bridge kelpie pins for Claude Code, and what a bridge says when the
//! harness behind it is too old for it
//!
//! `claude::pin` and the `TooOld` end of a session

use std::sync::mpsc;

use super::claude::pin;
use crate::bridge::Platform;
use crate::session::{Choice, SessionEvent};

use super::*;

#[test]
fn the_pin_holds_one_runnable_archive_per_platform() {
    let pin = pin();

    assert_eq!(pin.id, "claude-acp");
    assert_eq!(pin.version, "0.101.1");
    assert_eq!(pin.platforms.len(), 4);
    for (platform, download) in pin.platforms {
        let url = download.url.as_str();
        let archive = download.url.rsplit('/').next().unwrap();
        let expected = match platform {
            Platform::MacOsArm64 => "claude-agent-acp-darwin-arm64.zip",
            Platform::MacOsX64 => "claude-agent-acp-darwin-x64.zip",
            Platform::LinuxX64 => "claude-agent-acp-linux-x64.tar.gz",
            Platform::LinuxArm64 => "claude-agent-acp-linux-arm64.tar.gz",
            Platform::WindowsX64 => "no windows archive is pinned yet",
        };
        assert_eq!(archive, expected, "{url}");
        assert_eq!(download.cmd, "claude-agent-acp");
        assert!(
            download
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
                && download.sha256.len() == 64,
            "{:?}",
            download.sha256
        );
    }
}

/// A bridge that says the harness behind it is too old ends the session with
/// [`HarnessError::TooOld`], which carries the harness's own update command
///
/// Kelpie never runs that command (`docs/design/bridge-updates.md`, Scope): the
/// message is what the maintainer reads and runs themselves.
#[test]
fn a_harness_too_old_names_its_update_command() {
    use crate::session::fake::{FakeAgent, HARNESS};

    let agent = FakeAgent::new()
        .fails_new("Claude Code 1.0.0 is too old: this bridge needs 2.0.0 or later")
        .says_outdated(&["too old"], "claude update");
    let session = agent.session(PathBuf::from("/project"), None, Choice::default());

    let (sender, heard) = mpsc::channel();
    session.send("Hello", move |event| {
        let _ = sender.send(event);
    });
    let event = heard.recv_timeout(TURN).expect("no event from the session");
    let SessionEvent::Failed { message } = event else {
        panic!("a bridge that refused its harness did not end the send with Failed: {event:?}");
    };
    assert!(
        message.contains(&format!("{HARNESS} is too old")),
        "{message}"
    );
    assert!(message.contains("claude update"), "{message}");
    assert!(
        message.contains("too old: this bridge needs 2.0.0"),
        "the bridge's own words are gone: {message}"
    );
}
