//! What a session says about the harness it was asked for its options

use super::*;

/// A fake agent that runs in a sandbox, as the ask's launcher does
fn confined() -> FakeAgent {
    FakeAgent::new().sandboxed("bypassPermissions")
}

/// One value the fake agent lists, which names it the same way it values it
fn listed(value: &str) -> kelpie_protocol::Choice {
    kelpie_protocol::Choice {
        id: value.to_owned(),
        name: value.to_owned(),
        description: None,
    }
}

#[test]
fn asking_options_sends_no_prompt_and_stops_the_harness() {
    let mut agent = confined()
        .can_close()
        .offers_models("small", &["small", "large"])
        .offers_efforts("low", &["low", "high"]);
    let data = data_folder();
    let meta = json!({ "claudeCode": { "emitRawSDKMessages": true } });

    let options = options::offerings_at(
        &mut agent,
        "fake",
        &data,
        &PathBuf::from("/project"),
        Some(&meta),
        true,
    )
    .unwrap();

    assert_eq!(options.models, [listed("small"), listed("large")]);
    assert_eq!(options.efforts, [listed("low"), listed("high")]);
    assert_eq!(
        agent.methods(),
        ["initialize", "session/new", "session/close"],
        "the harness was asked, never prompted"
    );
    assert_eq!(
        agent.params("session/new")[0]["_meta"],
        meta,
        "the harness was asked the way a session would ask it"
    );
    assert_eq!(agent.launches(), 1);
    assert_eq!(
        agent.stopped(),
        1,
        "the harness was stopped, not left running"
    );
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn options_are_read_from_the_cache_until_a_refresh() {
    let mut agent = confined().offers_models("small", &["small", "large"]);
    let data = data_folder();
    let project = PathBuf::from("/project");

    let asked = options::offerings_at(&mut agent, "fake", &data, &project, None, false).unwrap();
    assert_eq!(agent.launches(), 1);

    let kept = options::offerings_at(&mut agent, "fake", &data, &project, None, false).unwrap();
    assert_eq!(kept, asked);
    assert_eq!(
        agent.launches(),
        1,
        "the kept answer is read, not asked again"
    );

    let fresh = options::offerings_at(&mut agent, "fake", &data, &project, None, true).unwrap();
    assert_eq!(fresh, asked);
    assert_eq!(agent.launches(), 2, "a refresh asks the harness again");

    let kept = std::fs::read_to_string(data.join("options").join("fake.json")).unwrap();
    let kept: serde_json::Value = serde_json::from_str(&kept).unwrap();
    assert!(
        kept["asked_at"].as_u64().is_some_and(|at| at > 0),
        "the answer says when it was asked: {kept}"
    );
    assert_eq!(kept["models"][1]["id"], "large");
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn a_harness_with_no_model_option_lists_none() {
    let mut agent = confined();

    let options = options::asked(&mut agent, &PathBuf::from("/project"), None).unwrap();

    assert_eq!(options, kelpie_protocol::Options::default());
    assert!(options.models.is_empty());
    assert!(options.efforts.is_empty());
}

#[test]
fn a_harness_that_never_answers_is_stopped_when_the_deadline_passes() {
    let mut agent = confined().never_answers_session_new();

    let error = options::asked_within(
        &mut agent,
        &PathBuf::from("/project"),
        None,
        Duration::from_millis(200),
    )
    .unwrap_err();

    assert!(
        matches!(error, SessionError::TimedOut { .. }),
        "a silent harness was not called silent: {error}"
    );
    assert_eq!(agent.stopped(), 1, "the silent harness was left running");
}

#[cfg(unix)]
#[test]
fn a_harness_that_exits_before_answering_is_explained_from_its_stderr() {
    use crate::run::RunId;
    use crate::tree::ProcessTree;

    let dir = std::env::temp_dir().join(format!("kelpie-options-{}", RunId::new().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    let tree = Arc::new(ProcessTree::new(RunId::new().unwrap(), dir.join("run.json")).unwrap());
    let mut harness = Harness::new("sh");
    harness.args = vec!["-c".into(), "echo token rejected >&2; exit 3".into()];

    // The sandbox is no part of what is tested here: how a harness that exits is explained
    let error = options::initialized_within(
        &mut *launcher(tree, harness, dir.clone()),
        Duration::from_secs(10),
    )
    .unwrap_err();

    let SessionError::Ended { message } = &error else {
        panic!("a harness that exited did not end the ask: {error}");
    };
    assert!(message.contains("exit status: 3"), "{message}");
    assert!(message.contains("token rejected"), "{message}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_harness_id_that_names_no_file_starts_nothing() {
    let mut agent = confined();
    let data = data_folder();

    let error = options::offerings_at(
        &mut agent,
        "../escape",
        &data,
        &PathBuf::from("/project"),
        None,
        true,
    )
    .expect_err("no answer can be kept under a path");

    assert!(
        matches!(&error, SessionError::Name { name } if name == "../escape"),
        "{error}"
    );
    assert_eq!(agent.launches(), 0, "a bad name started no harness");
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn the_ask_runs_the_harness_in_the_sandbox_it_is_given() {
    let mut agent = confined().offers_models("small", &["small"]);
    let data = data_folder();

    let options = options::offerings_at(
        &mut agent,
        "fake",
        &data,
        &PathBuf::from("/project"),
        None,
        true,
    )
    .unwrap();

    assert_eq!(options.models, [listed("small")]);
    assert_eq!(agent.launches(), 1, "the sandboxed harness was asked");
    std::fs::remove_dir_all(data).unwrap();
}

#[test]
fn an_ask_on_a_launcher_with_no_sandbox_starts_nothing() {
    let mut agent = FakeAgent::new().offers_models("small", &["small"]);
    let data = data_folder();

    let error = options::offerings_at(
        &mut agent,
        "fake",
        &data,
        &PathBuf::from("/project"),
        None,
        true,
    )
    .expect_err("the harness would have run outside a sandbox");

    assert!(matches!(error, SessionError::Start(_)), "{error}");
    assert_eq!(agent.launches(), 0, "the harness was started unsandboxed");
    assert!(
        options::kept(&data, "fake").is_none(),
        "an answer was kept from a harness that never ran"
    );
    std::fs::remove_dir_all(data).unwrap();
}
