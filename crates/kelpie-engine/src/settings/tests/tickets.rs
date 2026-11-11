//! Whose issues a project picks up, what it calls Ready, and how far back Done
//! reaches
//!
//! The three the board reads (`docs/design/tickets.md`, Where tickets come from,
//! and The board): `pick-up` and `ready-label` are the project's own, and
//! `done-window` is the machine's or the project's.

use serde_json::{json, to_value};

use super::*;

#[test]
fn pick_up_defaults_to_me_and_the_ready_label_to_ready_for_agent() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.pick_up,
        Layered {
            value: PickUp::Me,
            from: Layer::BuiltIn,
        },
        "kelpie picks up the issues its own login opened"
    );
    assert_eq!(
        settings.ready_label,
        Layered {
            value: "ready-for-agent".to_string(),
            from: Layer::BuiltIn,
        }
    );
    assert_eq!(BUILT_IN.pick_up, PickUp::Me);
    assert_eq!(PickUp::default(), PickUp::Me);

    for (pick_up, name) in [(PickUp::Me, "me"), (PickUp::Everyone, "everyone")] {
        assert_eq!(to_value(pick_up).unwrap(), json!(name), "{pick_up:?}");
    }
}

#[test]
fn a_project_layer_sets_everyone_and_a_label_of_its_own() {
    let dir = Dir::new();
    dir.put(
        LayerFile::Project("web"),
        r#"{"pick-up": "everyone", "ready-label": "ship-it"}"#,
    );

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.pick_up,
        Layered {
            value: PickUp::Everyone,
            from: Layer::Project,
        }
    );
    assert_eq!(
        settings.ready_label,
        Layered {
            value: "ship-it".to_string(),
            from: Layer::Project,
        }
    );

    // Another project sees neither
    let other = Settings::load(&dir.config(), "paper").unwrap();
    assert_eq!(
        other.pick_up,
        Layered {
            value: PickUp::Me,
            from: Layer::BuiltIn,
        },
        "whose issues one project picks up is not every project's"
    );
    assert_eq!(other.ready_label.value, "ready-for-agent");

    // A write lands the same way a person's own file reads
    Settings::set(
        &dir.config(),
        LayerFile::Project("paper"),
        Field::PickUp,
        &json!("everyone"),
    )
    .unwrap();
    let written = dir.text(LayerFile::Project("paper"));
    assert!(written.contains(r#""pick-up": "everyone""#), "{written}");
    assert_eq!(
        Settings::load(&dir.config(), "paper")
            .unwrap()
            .pick_up
            .value,
        PickUp::Everyone
    );
}

#[test]
fn the_global_layer_refuses_pick_up_and_the_ready_label() {
    let dir = Dir::new();

    assert_eq!(Field::PickUp.layers(), &[Layer::Project]);
    assert_eq!(Field::ReadyLabel.layers(), &[Layer::Project]);

    for (field, value) in [
        (Field::PickUp, json!("everyone")),
        (Field::ReadyLabel, json!("ship-it")),
    ] {
        let error = Settings::set(&dir.config(), LayerFile::Global, field, &value)
            .expect_err("the ticket source is the project's");
        let said = format!("{} cannot be set at the global layer", field.key());
        assert!(error.to_string().contains(&said), "{field:?}: {error}");
        assert!(
            !dir.config().file(LayerFile::Global).unwrap().exists(),
            "{field:?} wrote the global file"
        );
    }

    // A global file that names one anyway is refused when it is read
    dir.put(LayerFile::Global, r#"{"pick-up": "me"}"#);
    let read = Settings::load(&dir.config(), "web").expect_err("the global layer holds no pick-up");
    assert!(read.to_string().contains("pick-up"), "{read}");
}

#[test]
fn the_done_window_defaults_to_14_and_a_project_overrides_the_global_layer() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.done_window,
        Layered {
            value: BUILT_IN.done_window,
            from: Layer::BuiltIn,
        }
    );
    assert_eq!(
        settings.done_window.value.days(),
        14,
        "two weeks of finished work on the board"
    );

    Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::DoneWindow,
        &json!(30),
    )
    .unwrap();
    let global = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(global.done_window.value.days(), 30);
    assert_eq!(global.done_window.from, Layer::Global);

    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::DoneWindow,
        &json!(7),
    )
    .unwrap();
    let project = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(project.done_window.value.days(), 7);
    assert_eq!(project.done_window.from, Layer::Project);

    // The global window stands for every other project
    let other = Settings::load(&dir.config(), "paper").unwrap();
    assert_eq!(other.done_window.value.days(), 30);

    // A window written as null is a window nobody set
    dir.put(LayerFile::Project("web"), r#"{"done-window": null}"#);
    let cleared = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(cleared.done_window.value.days(), 30);
    assert_eq!(cleared.done_window.from, Layer::Global);
}

#[test]
fn a_done_window_of_0_or_over_365_is_refused_and_nothing_is_written() {
    let dir = Dir::new();
    dir.put(LayerFile::Project("web"), r#"{"pick-up": "everyone"}"#);

    for value in [json!(0), json!(-1), json!(366), json!("14"), json!(14.5)] {
        let error = Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::DoneWindow,
            &value,
        )
        .expect_err("a window of no days, or a typo, is refused");
        assert!(
            matches!(
                error,
                SettingsError::ValueRefused {
                    field: Field::DoneWindow,
                    ..
                }
            ),
            "refuses {value}: {error}"
        );
        assert_eq!(
            dir.text(LayerFile::Project("web")),
            r#"{"pick-up": "everyone"}"#,
            "{value} was written anyway"
        );
    }

    // The edges kelpie means to take
    for days in [1, 365] {
        Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::DoneWindow,
            &json!(days),
        )
        .unwrap();
        assert_eq!(
            Settings::load(&dir.config(), "web")
                .unwrap()
                .done_window
                .value
                .days(),
            days
        );
    }

    // A file written by hand is refused the same way a write is
    dir.put(LayerFile::Global, r#"{"done-window": 0}"#);
    let error = Settings::load(&dir.config(), "web").expect_err("Done holds nothing at all");
    assert!(
        matches!(error, SettingsError::FileRefused { .. }),
        "{error}"
    );
}

#[test]
fn an_empty_label_and_one_with_a_comma_are_refused_and_nothing_is_written() {
    let dir = Dir::new();
    dir.put(LayerFile::Project("web"), r#"{"pick-up": "me"}"#);

    // `gh issue list --label` splits on commas, so a name holding one names two
    // labels and never the one meant
    for value in [
        json!(""),
        json!("ready,for-agent"),
        json!("ship-it,"),
        json!(40),
    ] {
        let error = Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::ReadyLabel,
            &value,
        )
        .expect_err("a label that names none, or more than one, is refused");
        assert!(
            matches!(
                error,
                SettingsError::ValueRefused {
                    field: Field::ReadyLabel,
                    ..
                }
            ),
            "refuses {value}: {error}"
        );
        assert_eq!(
            dir.text(LayerFile::Project("web")),
            r#"{"pick-up": "me"}"#,
            "{value} was written anyway"
        );
    }
    let kept = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        kept.ready_label,
        Layered {
            value: "ready-for-agent".to_string(),
            from: Layer::BuiltIn,
        },
        "the refusal left kelpie's own label in place"
    );

    // A label of its own, and one cleared back to kelpie's
    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::ReadyLabel,
        &json!("ship-it"),
    )
    .unwrap();
    assert_eq!(
        Settings::load(&dir.config(), "web")
            .unwrap()
            .ready_label
            .value,
        "ship-it"
    );
    Settings::reset(&dir.config(), LayerFile::Project("web"), Field::ReadyLabel).unwrap();
    let cleared = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(cleared.ready_label.value, "ready-for-agent");
    assert_eq!(cleared.ready_label.from, Layer::BuiltIn);

    // A file someone typed is refused the same way a write is
    dir.put(
        LayerFile::Project("paper"),
        r#"{"ready-label": "ready,for-agent"}"#,
    );
    let error = Settings::load(&dir.config(), "paper")
        .expect_err("a label that names two labels is refused where it is read");
    assert!(
        matches!(error, SettingsError::FileRefused { .. }),
        "{error}"
    );
}

#[test]
fn a_value_other_than_me_or_everyone_is_refused() {
    let dir = Dir::new();

    for value in [
        json!("anyone"),
        json!("me "),
        json!("Me"),
        json!(true),
        json!(0),
    ] {
        let error = Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::PickUp,
            &value,
        )
        .expect_err("kelpie has two ticket sources");
        assert!(
            matches!(
                error,
                SettingsError::ValueRefused {
                    field: Field::PickUp,
                    ..
                }
            ),
            "refuses {value}: {error}"
        );
    }
    assert!(
        !dir.config()
            .file(LayerFile::Project("web"))
            .unwrap()
            .exists(),
        "a refused pick-up wrote a project file"
    );
}

/// The form titles each choice by its description, so the schema carries both
#[test]
fn the_schema_names_each_pick_up_with_the_word_a_form_shows() {
    let document = schema();
    let variants = document
        .pointer("/$defs/PickUp/oneOf")
        .and_then(Value::as_array)
        .expect("the schema does not name the two ticket sources")
        .clone();
    let shown: Vec<(&str, &str)> = variants
        .iter()
        .map(|variant| {
            (
                variant["const"].as_str().unwrap(),
                variant["description"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("me", "Me: the issues the signed-in login opened"),
            ("everyone", "Everyone: every open issue"),
        ]
    );

    let window = &document["properties"]["done-window"];
    assert_eq!(window["type"], json!(["integer", "null"]));
    assert_eq!(window["minimum"], json!(1));
    assert_eq!(window["maximum"], json!(365));

    let label = &document["properties"]["ready-label"];
    assert_eq!(label["type"], json!(["string", "null"]));
}
