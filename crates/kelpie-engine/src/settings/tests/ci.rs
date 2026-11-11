//! Whether CI holds a project's merge, how many fix attempts red CI gets, and
//! how long CI is waited on
//!
//! The three the gate reads (`docs/design/gate-and-merge.md`, Settings), each
//! the project's own.

use serde_json::json;

use super::*;

#[test]
fn ci_holds_the_merge_by_default() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.ci_holds_merge,
        Layered {
            value: true,
            from: Layer::BuiltIn,
        }
    );

    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::CiHoldsMerge,
        &json!(false),
    )
    .unwrap();
    let off = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        off.ci_holds_merge,
        Layered {
            value: false,
            from: Layer::Project,
        }
    );

    // Another project still has CI hold its merge
    assert!(
        Settings::load(&dir.config(), "paper")
            .unwrap()
            .ci_holds_merge
            .value
    );

    // Only a bool is one
    for value in [json!("off"), json!(0), json!([false])] {
        Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::CiHoldsMerge,
            &value,
        )
        .expect_err("a bool is the only value that says whether CI holds the merge");
    }
}

#[test]
fn fix_attempts_has_no_cap_by_default_and_takes_zero() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.fix_attempts,
        Layered {
            value: None,
            from: Layer::BuiltIn,
        },
        "no cap until a project names one"
    );

    for count in [0, 3, u32::MAX] {
        Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::FixAttempts,
            &json!(count),
        )
        .unwrap();
        let set = Settings::load(&dir.config(), "web").unwrap();
        assert_eq!(set.fix_attempts.from, Layer::Project);
        assert_eq!(
            set.fix_attempts.value.map(FixAttempts::count),
            Some(count),
            "{count} fix attempts"
        );
    }

    // Clearing the field is no cap again
    Settings::reset(&dir.config(), LayerFile::Project("web"), Field::FixAttempts).unwrap();
    let cleared = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(cleared.fix_attempts.value, None);
    assert_eq!(cleared.fix_attempts.from, Layer::BuiltIn);
}

#[test]
fn a_negative_fix_attempts_is_refused() {
    let dir = Dir::new();
    dir.put(LayerFile::Project("web"), r#"{"ci-holds-merge": false}"#);

    // SK spelled no cap as -1. Here it is the field being absent
    for value in [json!(-1), json!(-5), json!(1.5), json!("3")] {
        let error = Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::FixAttempts,
            &value,
        )
        .expect_err("a count below 0, or no count, is refused");
        assert!(
            matches!(
                error,
                SettingsError::ValueRefused {
                    field: Field::FixAttempts,
                    ..
                }
            ),
            "refuses {value}: {error}"
        );
        assert_eq!(
            dir.text(LayerFile::Project("web")),
            r#"{"ci-holds-merge": false}"#,
            "{value} was written anyway"
        );
    }

    // A file written by hand is refused the same way a write is
    dir.put(LayerFile::Project("web"), r#"{"fix-attempts": -1}"#);
    let error = Settings::load(&dir.config(), "web").expect_err("-1 is no count");
    assert!(
        matches!(error, SettingsError::FileRefused { .. }),
        "{error}"
    );
}

#[test]
fn ci_wait_is_sixty_minutes_by_default() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(settings.ci_wait.from, Layer::BuiltIn);
    assert_eq!(
        settings.ci_wait.value.duration(),
        Duration::from_secs(60 * 60)
    );

    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::CiWait,
        &json!(90),
    )
    .unwrap();
    let set = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(set.ci_wait.value.duration(), Duration::from_secs(90 * 60));
    assert_eq!(set.ci_wait.from, Layer::Project);
}

#[test]
fn a_ci_wait_of_zero_or_past_a_day_is_refused() {
    let dir = Dir::new();
    dir.put(LayerFile::Project("web"), r#"{"ci-holds-merge": false}"#);

    for value in [json!(0), json!(-1), json!(1441), json!("60"), json!(60.5)] {
        let error = Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::CiWait,
            &value,
        )
        .expect_err("a wait of no time, or past a day, is refused");
        assert!(
            matches!(
                error,
                SettingsError::ValueRefused {
                    field: Field::CiWait,
                    ..
                }
            ),
            "refuses {value}: {error}"
        );
        assert_eq!(
            dir.text(LayerFile::Project("web")),
            r#"{"ci-holds-merge": false}"#,
            "{value} was written anyway"
        );
    }

    // The edges kelpie means to take
    for minutes in [1_u64, 1440] {
        Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::CiWait,
            &json!(minutes),
        )
        .unwrap();
        assert_eq!(
            Settings::load(&dir.config(), "web")
                .unwrap()
                .ci_wait
                .value
                .duration(),
            Duration::from_secs(minutes * 60)
        );
    }

    // A file written by hand is refused the same way a write is
    dir.put(LayerFile::Project("web"), r#"{"ci-wait": 0}"#);
    let error = Settings::load(&dir.config(), "web").expect_err("no wait waits for nothing");
    assert!(
        matches!(error, SettingsError::FileRefused { .. }),
        "{error}"
    );
}

#[test]
fn the_three_rows_are_project_settings_only() {
    let dir = Dir::new();

    for (field, key, value) in [
        (Field::CiHoldsMerge, "ci-holds-merge", json!(false)),
        (Field::FixAttempts, "fix-attempts", json!(2)),
        (Field::CiWait, "ci-wait", json!(30)),
    ] {
        assert_eq!(field.key(), key);
        assert_eq!(field.layers(), &[Layer::Project], "{key}");
        assert!(Field::ALL.contains(&field), "{key} is on the form");

        let error = Settings::set(&dir.config(), LayerFile::Global, field, &value)
            .expect_err("whether CI runs is the project's");
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
