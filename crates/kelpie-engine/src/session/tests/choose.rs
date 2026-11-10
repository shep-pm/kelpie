//! A choice made between messages reaches the session that is already running

use super::*;

#[test]
fn a_choice_made_between_messages_applies_from_the_next_one_on_the_same_session() {
    let agent = FakeAgent::new()
        .offers_models("small", &["small", "large"])
        .turn(Turn::new().text("one"))
        .turn(Turn::new().text("two"));
    let session = agent.session(
        PathBuf::from("/project"),
        None,
        Choice {
            model: Some("small".to_owned()),
            effort: None,
        },
    );
    say(&session, "First");

    session.choose(Choice {
        model: Some("large".to_owned()),
        effort: None,
    });
    let second = say(&session, "Second");

    assert_eq!(second, [text("two"), ended()]);
    assert_eq!(agent.launches(), 1, "the same session carries on");
    assert_eq!(
        agent.methods(),
        [
            "initialize",
            "session/new",
            "session/set_config_option",
            "session/prompt",
            "session/set_config_option",
            "session/prompt"
        ]
    );
    assert_eq!(
        agent.params("session/set_config_option"),
        [
            json!({"sessionId": "fake-1", "configId": "model", "value": "small"}),
            json!({"sessionId": "fake-1", "configId": "model", "value": "large"}),
        ]
    );
}

#[test]
fn a_choice_that_did_not_change_sets_nothing_again() {
    let agent = FakeAgent::new()
        .offers_models("small", &["small", "large"])
        .turn(Turn::new().text("one"))
        .turn(Turn::new().text("two"));
    let choice = Choice {
        model: Some("large".to_owned()),
        effort: None,
    };
    let session = agent.session(PathBuf::from("/project"), None, choice.clone());
    say(&session, "First");

    session.choose(choice);
    say(&session, "Second");

    assert_eq!(agent.params("session/set_config_option").len(), 1);
}
