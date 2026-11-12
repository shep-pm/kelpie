//! The reviewers a project lists, how its tests are run, and the reviewers' harness, model and
//! effort
//!
//! The five the pass reads (`docs/design/review.md`, Settings), each the project's own.

use serde_json::json;

use super::*;

#[test]
fn reviewers_is_kelpie_reviewer_by_default() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.reviewers.from,
        Layer::BuiltIn,
        "no layer names a reviewer"
    );
    assert_eq!(settings.reviewers.value.names(), ["kelpie:reviewer"]);

    // A project that lists others reads them in the order it wrote them
    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::Reviewers,
        &json!(["acme:security", "kelpie:reviewer"]),
    )
    .unwrap();
    let set = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(set.reviewers.from, Layer::Project);
    assert_eq!(
        set.reviewers.value.names(),
        ["acme:security", "kelpie:reviewer"]
    );

    // Another project keeps the default
    assert_eq!(
        Settings::load(&dir.config(), "paper")
            .unwrap()
            .reviewers
            .value
            .names(),
        ["kelpie:reviewer"]
    );
}

#[test]
fn a_reviewer_listed_twice_is_refused() {
    let dir = Dir::new();
    dir.put(LayerFile::Project("web"), r#"{"ci-holds-merge": false}"#);

    let twice = json!(["kelpie:reviewer", "acme:security", "kelpie:reviewer"]);
    let error = Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::Reviewers,
        &twice,
    )
    .expect_err("a reviewer reads a head once");
    assert!(
        matches!(
            error,
            SettingsError::ValueRefused {
                field: Field::Reviewers,
                ..
            }
        ),
        "{error}"
    );
    assert_eq!(
        dir.text(LayerFile::Project("web")),
        r#"{"ci-holds-merge": false}"#,
        "the list was written anyway"
    );

    // A name is a name: not empty, and a string
    for value in [json!([""]), json!([1]), json!("kelpie:reviewer")] {
        Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::Reviewers,
            &value,
        )
        .expect_err("a list of names is the only value");
    }

    // A file written by hand is refused the same way a write is
    dir.put(
        LayerFile::Project("web"),
        &format!(r#"{{"reviewers": {twice}}}"#),
    );
    let error = Settings::load(&dir.config(), "web").expect_err("a repeat is no list");
    assert!(
        matches!(error, SettingsError::FileRefused { .. }),
        "{error}"
    );
}

#[test]
fn an_empty_reviewers_list_is_allowed() {
    let dir = Dir::new();

    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::Reviewers,
        &json!([]),
    )
    .unwrap();
    let set = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(set.reviewers.from, Layer::Project);
    assert!(
        set.reviewers.value.names().is_empty(),
        "an empty list is no reviewer, not kelpie's"
    );

    // Clearing the field is kelpie's own reviewer again
    Settings::reset(&dir.config(), LayerFile::Project("web"), Field::Reviewers).unwrap();
    let cleared = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(cleared.reviewers.from, Layer::BuiltIn);
    assert_eq!(cleared.reviewers.value.names(), ["kelpie:reviewer"]);
}

#[test]
fn the_test_command_is_absent_by_default() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.test_command,
        Layered {
            value: None,
            from: Layer::BuiltIn,
        },
        "no reviewer is asked for a test until a project names a command"
    );

    for command in [
        "cargo test --test {name}",
        "pytest {test}",
        "make {test} {name}",
    ] {
        Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::TestCommand,
            &json!(command),
        )
        .unwrap();
        let set = Settings::load(&dir.config(), "web").unwrap();
        assert_eq!(set.test_command.from, Layer::Project);
        assert_eq!(
            set.test_command.value.as_ref().map(TestCommand::as_str),
            Some(command)
        );
    }

    // Clearing the field is no command again
    Settings::reset(&dir.config(), LayerFile::Project("web"), Field::TestCommand).unwrap();
    let cleared = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(cleared.test_command.value, None);
    assert_eq!(cleared.test_command.from, Layer::BuiltIn);
}

#[test]
fn a_test_command_without_test_or_name_is_refused() {
    let dir = Dir::new();
    dir.put(LayerFile::Project("web"), r#"{"ci-holds-merge": false}"#);

    for value in [json!("cargo test"), json!(""), json!("{tests} {names}")] {
        let error = Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::TestCommand,
            &value,
        )
        .expect_err("a command that names no test file runs none");
        assert!(
            matches!(
                error,
                SettingsError::ValueRefused {
                    field: Field::TestCommand,
                    ..
                }
            ),
            "refuses {value}: {error}"
        );
        let said = error.to_string();
        assert!(said.contains("{test}"), "{said}");
        assert!(said.contains("{name}"), "{said}");
        assert_eq!(
            dir.text(LayerFile::Project("web")),
            r#"{"ci-holds-merge": false}"#,
            "{value} was written anyway"
        );
    }

    // Only a string is one
    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::TestCommand,
        &json!(["cargo test {name}"]),
    )
    .expect_err("a command is one string");

    // A file written by hand is refused the same way a write is
    dir.put(
        LayerFile::Project("web"),
        r#"{"test-command": "cargo test"}"#,
    );
    let error = Settings::load(&dir.config(), "web").expect_err("no placeholder, no command");
    assert!(
        matches!(error, SettingsError::FileRefused { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("test-command"), "{error}");
}

#[test]
fn the_reviewer_harness_model_and_effort_are_empty_by_default() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    for (name, from, empty) in [
        (
            "harness",
            settings.reviewer_harness.from,
            settings.reviewer_harness.value.is_none(),
        ),
        (
            "model",
            settings.reviewer_model.from,
            settings.reviewer_model.value.is_none(),
        ),
        (
            "effort",
            settings.reviewer_effort.from,
            settings.reviewer_effort.value.is_none(),
        ),
    ] {
        assert!(empty, "kelpie names no reviewer {name} (ADR 0006)");
        assert_eq!(from, Layer::BuiltIn, "reviewer {name}");
    }
    assert_eq!(BUILT_IN.reviewer_harness, None);
    assert_eq!(BUILT_IN.reviewer_model, None);
    assert_eq!(BUILT_IN.reviewer_effort, None);

    // The same types as the worker's three rows
    for (field, value) in [
        (Field::ReviewerHarness, json!("goose")),
        (Field::ReviewerModel, json!("small")),
        (Field::ReviewerEffort, json!("high")),
    ] {
        Settings::set(&dir.config(), LayerFile::Project("web"), field, &value).unwrap();
    }
    let set = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        set.reviewer_harness,
        Layered {
            value: Some(Harness::Goose),
            from: Layer::Project,
        }
    );
    assert_eq!(set.reviewer_model.value.as_deref(), Some("small"));
    assert_eq!(set.reviewer_effort.value.as_deref(), Some("high"));
    assert_eq!(
        set.worker_harness.value, None,
        "the worker's rows are its own"
    );

    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::ReviewerHarness,
        &json!("not-a-harness"),
    )
    .expect_err("a reviewer runs on a harness kelpie has");
}

#[test]
fn the_five_rows_are_project_settings_only() {
    let dir = Dir::new();

    for (field, key, value) in [
        (Field::Reviewers, "reviewers", json!(["kelpie:reviewer"])),
        (
            Field::TestCommand,
            "test-command",
            json!("cargo test {name}"),
        ),
        (Field::ReviewerHarness, "reviewer-harness", json!("goose")),
        (Field::ReviewerModel, "reviewer-model", json!("small")),
        (Field::ReviewerEffort, "reviewer-effort", json!("high")),
    ] {
        assert_eq!(field.key(), key);
        assert_eq!(field.layers(), &[Layer::Project], "{key}");
        assert!(Field::ALL.contains(&field), "{key} is on the form");

        let error = Settings::set(&dir.config(), LayerFile::Global, field, &value)
            .expect_err("which agents review a repo is the project's");
        let said = format!("{key} cannot be set at the global layer");
        assert!(error.to_string().contains(&said), "{key}: {error}");
        assert!(
            !dir.config().file(LayerFile::Global).unwrap().exists(),
            "{key} wrote the global file"
        );

        // A global file that names one anyway is refused when it is read
        dir.put(LayerFile::Global, &format!(r#"{{"{key}": {value}}}"#));
        let read = Settings::load(&dir.config(), "web").expect_err("the global layer holds none");
        assert!(read.to_string().contains(key), "{key}: {read}");
        fs::remove_file(dir.config().file(LayerFile::Global).unwrap()).unwrap();
    }
}
