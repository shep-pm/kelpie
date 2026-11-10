//! A session in a sandbox sets the harness's mode that stops its own prompts

use super::*;
use crate::session::fake::FakeBridge;

/// What a sandboxed agent's mode setting looks like on the wire
fn set_mode(value: &str) -> serde_json::Value {
    json!({"sessionId": "fake-1", "configId": "mode", "value": value})
}

/// An agent that offers the mode and runs in a sandbox, as each bridge version's does
fn confined() -> FakeAgent {
    FakeAgent::new()
        .offers_modes("default", &["default", "bypassPermissions"])
        .sandboxed("bypassPermissions")
}

#[test]
fn a_sandboxed_session_sets_the_mode_that_stops_the_harnesss_own_prompts() {
    let agent = FakeAgent::new().offers_modes("default", &["default", "bypassPermissions"]);

    let heard = say(&session(&agent).with_mode("bypassPermissions"), "Hello");

    assert_eq!(heard, [started("fake-1"), ended()]);
    assert_eq!(
        agent.params("session/set_config_option"),
        [json!({"sessionId": "fake-1", "configId": "mode", "value": "bypassPermissions"})]
    );
}

#[test]
fn a_request_that_arrives_in_spite_of_that_mode_is_still_refused() {
    let agent = FakeAgent::new()
        .offers_modes("default", &["default", "bypassPermissions"])
        .turn(Turn::new().permission("t1", "Write notes.md"));

    let heard = say(&session(&agent).with_mode("bypassPermissions"), "Write it");

    assert_eq!(
        heard,
        [
            started("fake-1"),
            step("t1", "Write notes.md", ToolStatus::Refused),
            ended(),
        ]
    );
}

#[test]
fn a_session_with_no_sandbox_leaves_the_mode_alone() {
    let agent = FakeAgent::new().offers_modes("default", &["default", "bypassPermissions"]);

    say(&session(&agent), "Hello");

    assert!(agent.params("session/set_config_option").is_empty());
}

#[test]
fn a_harness_that_lists_no_mode_says_so_and_the_turn_goes_on() {
    let agent = FakeAgent::new();

    let heard = say(&session(&agent).with_mode("bypassPermissions"), "Hello");

    assert_eq!(heard[0], started("fake-1"));
    assert!(
        matches!(&heard[1], SessionEvent::Failed { message } if message.contains("mode")),
        "{heard:?}"
    );
    assert_eq!(heard[2], ended());
}

#[test]
fn a_launcher_that_runs_in_a_sandbox_sets_the_mode_without_the_session_being_told() {
    let agent = confined();

    say(&session(&agent), "Hello");

    assert_eq!(
        agent.params("session/set_config_option"),
        [set_mode("bypassPermissions")]
    );
}

#[test]
fn a_bridge_version_swapped_in_runs_in_its_own_sandbox_with_the_mode_set() {
    let old = confined().turn(Turn::new().text("old"));
    let new = confined()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("new"));
    let bridge = FakeBridge::new("0.101.1")
        .agent("0.101.1", old.clone())
        .agent("0.102.0", new.clone());
    let session = Session::on_bridge(
        bridge.bridge(),
        PathBuf::from("/project"),
        None,
        Choice::default(),
    );

    say(&session, "first");
    let bridge = bridge.ready("0.102.0");
    say(&session, "second");

    assert_eq!(bridge.started().len(), 2);
    assert_eq!(
        old.params("session/set_config_option"),
        [set_mode("bypassPermissions")]
    );
    assert_eq!(
        new.params("session/set_config_option"),
        [set_mode("bypassPermissions")],
        "the swapped-in version ran without the mode that stops its prompts"
    );
}

#[test]
fn a_bridge_version_that_does_not_run_in_a_sandbox_keeps_its_prompts() {
    let old = confined().turn(Turn::new().text("old"));
    let new = FakeAgent::new()
        .offers_modes("default", &["default", "bypassPermissions"])
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("new"));
    let bridge = FakeBridge::new("0.101.1")
        .agent("0.101.1", old)
        .agent("0.102.0", new.clone());
    let session = Session::on_bridge(
        bridge.bridge(),
        PathBuf::from("/project"),
        None,
        Choice::default(),
    );

    say(&session, "first");
    let _bridge = bridge.ready("0.102.0");
    say(&session, "second");

    assert!(new.params("session/set_config_option").is_empty());
}
