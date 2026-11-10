//! Probing a bridge: `initialize` alone, and the capabilities kelpie needs from it
//!
//! `bridge/probe.rs`

use super::*;
use crate::session::fake::FakeAgent;
#[test]

fn the_probe_sends_no_prompt_and_stops_the_bridge() {
    let mut agent = FakeAgent::new().can_resume();

    probe::probed_within(&mut agent, &[Capability::ResumeSession], ANSWER).unwrap();

    assert_eq!(
        agent.methods(),
        ["initialize"],
        "the bridge was asked to initialize, never to open a session or run a prompt"
    );
    assert_eq!(agent.params("initialize")[0]["protocolVersion"], 1);
    assert_eq!(agent.launches(), 1);
    assert_eq!(agent.stopped(), 1, "the bridge was left running");
}

#[test]

fn a_bridge_that_can_load_a_session_keeps_the_resume_capability() {
    let mut agent = FakeAgent::new().can_load();

    probe::probed_within(&mut agent, &[Capability::ResumeSession], ANSWER).unwrap();
}

#[test]

fn a_bridge_that_lost_a_capability_fails_the_probe_naming_it() {
    let mut agent = FakeAgent::new();

    let error = probe::probed_within(&mut agent, &[Capability::ResumeSession], ANSWER).unwrap_err();

    assert!(
        matches!(
            error,
            ProbeError::Missing {
                capability: Capability::ResumeSession
            }
        ),
        "{error}"
    );
    assert!(error.to_string().contains("session/resume"), "{error}");
    assert_eq!(agent.stopped(), 1, "a failed probe still stops the bridge");
}
