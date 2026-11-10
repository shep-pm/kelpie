//! What a session does with the instructions it is given

use super::*;

const INSTRUCTIONS: &str = "You are the project manager.";

fn system_prompt(text: &str) -> serde_json::Value {
    json!({ "mode": "append", "content": [{ "type": "text", "text": text }] })
}

fn prompt_texts(agent: &FakeAgent) -> Vec<String> {
    agent
        .params("session/prompt")
        .iter()
        .map(|params| params["prompt"][0]["text"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn instructions_go_in_the_system_prompt_when_offered() {
    let agent = FakeAgent::new().accepts_system_prompt();

    say(&session(&agent).with_instructions(INSTRUCTIONS), "Hi");

    assert_eq!(
        agent.params("session/new")[0]["systemPrompt"],
        system_prompt(INSTRUCTIONS)
    );
    assert_eq!(prompt_texts(&agent), ["Hi"]);
}

#[test]
fn instructions_lead_the_first_message_otherwise() {
    let agent = FakeAgent::new();
    let session = session(&agent).with_instructions(INSTRUCTIONS);

    say(&session, "Hi");
    say(&session, "Again");

    assert!(agent.params("session/new")[0].get("systemPrompt").is_none());
    assert_eq!(
        prompt_texts(&agent),
        [format!("{INSTRUCTIONS}\n\nHi"), "Again".to_owned()]
    );
}

#[test]
fn instructions_go_in_the_system_prompt_on_resume_and_on_load() {
    let resumes = FakeAgent::new()
        .accepts_system_prompt()
        .can_resume()
        .knows("old");
    let loads = FakeAgent::new()
        .accepts_system_prompt()
        .can_load()
        .knows("old");

    for agent in [&resumes, &loads] {
        let session = agent
            .session(
                PathBuf::from("/project"),
                Some("old".into()),
                Choice::default(),
            )
            .with_instructions(INSTRUCTIONS);
        say(&session, "Back again");
    }

    assert_eq!(
        resumes.params("session/resume")[0]["systemPrompt"],
        system_prompt(INSTRUCTIONS)
    );
    assert_eq!(
        loads.params("session/load")[0]["systemPrompt"],
        system_prompt(INSTRUCTIONS)
    );
    assert_eq!(prompt_texts(&resumes), ["Back again"]);
}

#[test]
fn a_resumed_session_without_a_system_prompt_does_not_get_the_instructions_again() {
    let agent = FakeAgent::new().can_resume().knows("old");

    say(
        &agent
            .session(
                PathBuf::from("/project"),
                Some("old".into()),
                Choice::default(),
            )
            .with_instructions(INSTRUCTIONS),
        "Back again",
    );

    assert_eq!(prompt_texts(&agent), ["Back again"]);
}

#[test]
fn a_resume_that_starts_a_new_session_gives_it_the_instructions() {
    let agent = FakeAgent::new().can_resume();

    let heard = say(
        &agent
            .session(
                PathBuf::from("/project"),
                Some("gone".into()),
                Choice::default(),
            )
            .with_instructions(INSTRUCTIONS),
        "Back again",
    );

    assert!(matches!(heard[0], SessionEvent::Failed { .. }));
    assert_eq!(
        prompt_texts(&agent),
        [format!("{INSTRUCTIONS}\n\nBack again")]
    );
}

#[test]
fn blank_instructions_change_nothing() {
    let agent = FakeAgent::new().accepts_system_prompt();

    say(&session(&agent).with_instructions("  \n"), "Hi");

    assert!(agent.params("session/new")[0].get("systemPrompt").is_none());
    assert_eq!(prompt_texts(&agent), ["Hi"]);
}
