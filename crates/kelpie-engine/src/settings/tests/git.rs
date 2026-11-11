//! `catch-up`, which says when a pull request behind the base is caught up with it
//!
//! Lifted from SK's `[app.dogs.kelpie.git]` `catch_up` tests, ported to JSON layers.

use serde_json::json;

use super::*;

fn set(dir: &Dir, value: &Value) -> Result<(), SettingsError> {
    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::CatchUp,
        value,
    )
}

#[test]
fn a_missing_catch_up_is_always() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.catch_up,
        Layered {
            value: CatchUp::Always,
            from: Layer::BuiltIn,
        }
    );
    assert_eq!(BUILT_IN.catch_up, CatchUp::Always);
}

#[test]
fn catch_up_reads_always_and_conflicts() {
    let dir = Dir::new();

    for (word, expected) in [
        ("always", CatchUp::Always),
        ("conflicts", CatchUp::Conflicts),
    ] {
        set(&dir, &json!(word)).unwrap();
        let settings = Settings::load(&dir.config(), "web").unwrap();
        assert_eq!(settings.catch_up.value, expected, "{word}");
        assert_eq!(settings.catch_up.from, Layer::Project, "{word}");
    }

    // Another project is still caught up always
    assert_eq!(
        Settings::load(&dir.config(), "paper")
            .unwrap()
            .catch_up
            .value,
        CatchUp::Always
    );
}

#[test]
fn catch_up_refuses_any_other_word() {
    let dir = Dir::new();

    for word in ["never", "Always", ""] {
        let error = set(&dir, &json!(word)).expect_err("only two words are a catch-up");
        assert!(
            matches!(
                error,
                SettingsError::ValueRefused {
                    field: Field::CatchUp,
                    ..
                }
            ),
            "refuses {word:?}: {error}"
        );
        assert!(
            error.to_string().contains("always"),
            "names the choices: {error}"
        );
    }

    for value in [json!(true), json!(1), json!(["always"])] {
        set(&dir, &value).expect_err("a word is the only value that says when");
    }

    // A file that holds one is refused when it is read
    dir.put(LayerFile::Project("web"), r#"{"catch-up": "never"}"#);
    let read = Settings::load(&dir.config(), "web").expect_err("never is not a catch-up");
    assert!(read.to_string().contains("catch-up"), "{read}");
}

#[test]
fn catch_up_is_a_project_setting_only() {
    let dir = Dir::new();

    assert_eq!(Field::CatchUp.key(), "catch-up");
    assert_eq!(Field::CatchUp.layers(), &[Layer::Project]);
    assert!(Field::ALL.contains(&Field::CatchUp), "it is on the form");

    let error = Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::CatchUp,
        &json!("conflicts"),
    )
    .expect_err("whether a repo has a merge queue is the project's");
    assert!(
        error
            .to_string()
            .contains("catch-up cannot be set at the global layer"),
        "{error}"
    );
    assert!(
        !dir.config().file(LayerFile::Global).unwrap().exists(),
        "the global file was written"
    );

    // A global file that names it anyway is refused when it is read
    dir.put(LayerFile::Global, r#"{"catch-up": "conflicts"}"#);
    let read = Settings::load(&dir.config(), "web").expect_err("the global layer holds none");
    assert!(read.to_string().contains("catch-up"), "{read}");
}
