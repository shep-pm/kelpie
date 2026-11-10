//! What the PM runs on, resolved from the settings
//!
//! Each test reads the layers kelpie resolves through, the built-in default, the
//! global `pm` agent and the project's own file, and what reaches the session.

use std::fs;

use crate::settings::{Target, settings_set};

use super::*;

#[test]
fn the_next_send_uses_the_new_model() {
    let agent = FakeAgent::new()
        .offers_models("small", &["small", "large"])
        .turn(Turn::new().text("one"))
        .turn(Turn::new().text("two"));
    let chat = Chat::planned(&agent);
    let project = chat.state().project_id(None).unwrap();
    let layer = || Target::Project {
        id: project.clone(),
    };
    settings_set(chat.state(), layer(), "pm-model".to_owned(), "small".into()).unwrap();
    chat.send("First").until_turn_ends();

    settings_set(chat.state(), layer(), "pm-model".to_owned(), "large".into()).unwrap();
    chat.send("Second").until_turn_ends();

    assert_eq!(agent.launches(), 1, "the same session carries on");
    assert_eq!(
        agent.params("session/set_config_option"),
        [
            serde_json::json!({"sessionId": "fake-1", "configId": "model", "value": "small"}),
            serde_json::json!({"sessionId": "fake-1", "configId": "model", "value": "large"}),
        ]
    );
}

#[test]
fn a_model_the_global_pm_agent_sets_reaches_the_first_send() {
    let agent = FakeAgent::new()
        .offers_models("small", &["small", "large"])
        .turn(Turn::new().text("one"));
    let chat = Chat::planned(&agent);
    settings_set(
        chat.state(),
        Target::Global,
        "pm-model".to_owned(),
        "large".into(),
    )
    .unwrap();

    chat.send("First").until_turn_ends();

    assert_eq!(
        agent.params("session/set_config_option"),
        [serde_json::json!({"sessionId": "fake-1", "configId": "model", "value": "large"})]
    );
}

#[test]
fn with_nothing_set_the_pm_runs_on_kelpies_default() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();

    let plan = plan(chat.state().config(), &project).unwrap();

    assert_eq!(plan.harness, PM_DEFAULT.harness);
    assert_eq!(plan.choice.model.as_deref(), Some(PM_DEFAULT.model));
    assert_eq!(plan.choice.effort.as_deref(), Some(PM_DEFAULT.effort));
}

#[test]
fn another_harness_gets_no_model_kelpie_named_for_the_default_one() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();
    settings_set(
        chat.state(),
        Target::Project {
            id: project.clone(),
        },
        "pm-harness".to_owned(),
        "goose".into(),
    )
    .unwrap();

    let plan = plan(chat.state().config(), &project).unwrap();

    assert_eq!(plan.harness, Harness::Goose);
    assert_eq!(plan.choice, Choice::default());
}

#[test]
fn with_nothing_set_the_sandbox_has_no_extra_domain_and_reads_the_project_alone() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();

    let plan = plan(chat.state().config(), &project).unwrap();

    assert!(plan.network_allowlist.is_empty());
    assert_eq!(plan.reads, Reads::Project);
}

#[test]
fn the_projects_allowlist_and_read_anywhere_settings_reach_the_plan() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();
    let set = |key: &str, value: serde_json::Value| {
        settings_set(
            chat.state(),
            Target::Project {
                id: project.clone(),
            },
            key.to_owned(),
            value,
        )
        .unwrap();
    };
    set(
        "network-allowlist",
        serde_json::json!(["docs.rs", "crates.io"]),
    );
    set("read-anywhere", serde_json::json!(true));

    let plan = plan(chat.state().config(), &project).unwrap();

    assert_eq!(plan.network_allowlist, ["docs.rs", "crates.io"]);
    assert_eq!(plan.reads, Reads::Anywhere);
}

#[test]
fn the_reply_style_is_part_of_the_instructions_unless_the_harness_default_is_chosen() {
    let chat = Chat::on(&FakeAgent::new());
    let project = chat.state().project_id(None).unwrap();
    let instructions = || {
        plan(chat.state().config(), &project)
            .unwrap()
            .instructions
            .unwrap()
    };
    assert!(instructions().contains("## Replies"));
    assert!(instructions().contains("decisions.md"));

    settings_set(
        chat.state(),
        Target::Global,
        "reply-style".to_owned(),
        "harness-default".into(),
    )
    .unwrap();

    assert!(!instructions().contains("## Replies"));
    assert!(instructions().contains("decisions.md"));
}

#[test]
fn a_settings_file_that_cannot_be_used_reaches_the_chat_as_failed() {
    let chat = Chat::on(&FakeAgent::new());
    let file = chat.dir.join("config").join("settings.json");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, "{ not json").unwrap();

    let SessionEvent::Failed { message } = chat.send("Hi").next() else {
        panic!("the chat should hear a Failed");
    };

    assert!(message.contains("settings.json"), "{message}");
}
