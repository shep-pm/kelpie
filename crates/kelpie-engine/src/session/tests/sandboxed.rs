//! A session in a sandbox sets the harness's mode that stops its own prompts

use super::*;

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
