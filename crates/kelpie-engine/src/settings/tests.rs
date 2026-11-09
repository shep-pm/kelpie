use std::fs;
use std::path::PathBuf;

use serde_json::{Map, Value, json};

use super::*;
use crate::run::RunId;

/// A config folder of its own for one test, never written over by another
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-settings-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// The config folder itself, which no test may assume exists beforehand
    fn config(&self) -> ConfigDir {
        ConfigDir::at(&self.0)
    }

    /// Write a layer file by hand, the way a person or a sync would
    fn put(&self, at: LayerFile<'_>, text: &str) {
        let file = self.config().file(at).unwrap();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, text).unwrap();
    }

    fn text(&self, at: LayerFile<'_>) -> String {
        fs::read_to_string(self.config().file(at).unwrap()).unwrap()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_config_dir_is_dot_config_kelpie_on_every_os() {
    for home in [
        "/home/maintainer",
        "/Users/maintainer",
        "C:\\Users\\maintainer",
    ] {
        let dir = ConfigDir::from_home(PathBuf::from(home).as_path());
        assert!(
            dir.path().ends_with(".config/kelpie"),
            "{} is not under {home}",
            dir.path().display()
        );
        assert!(
            dir.path().starts_with(home),
            "{} leaves {home}",
            dir.path().display()
        );
    }
}

#[test]
fn a_config_folder_is_never_created_on_a_read() {
    let dir = Dir::new();
    let missing = dir.0.join(".config").join("kelpie");
    let settings = Settings::load(&ConfigDir::at(&missing), "web").unwrap();
    assert_eq!(settings.reply_style.from, Layer::BuiltIn);
    assert!(!missing.exists());
}

#[test]
fn a_project_value_wins_over_global_and_says_so() {
    let dir = Dir::new();
    dir.put(LayerFile::Global, r#"{"reply-style": "harness-default"}"#);
    dir.put(
        LayerFile::Project("web"),
        r#"{"reply-style": "concise-plus"}"#,
    );

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.reply_style,
        Layered {
            value: ReplyStyle::ConcisePlus,
            from: Layer::Project,
        }
    );

    let other = Settings::load(&dir.config(), "paper").unwrap();
    assert_eq!(
        other.reply_style,
        Layered {
            value: ReplyStyle::HarnessDefault,
            from: Layer::Global,
        }
    );
}

#[test]
fn an_unset_field_comes_from_the_built_in_layer() {
    let dir = Dir::new();
    let settings = Settings::load(&dir.config(), "web").unwrap();

    assert_eq!(
        settings,
        Resolved {
            pm_harness: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            pm_model: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            pm_effort: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            provider: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            host: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            reply_style: Layered {
                value: ReplyStyle::ConcisePlus,
                from: Layer::BuiltIn,
            },
            network_allowlist: Layered {
                value: Vec::new(),
                from: Layer::BuiltIn,
            },
            read_anywhere: Layered {
                value: false,
                from: Layer::BuiltIn,
            },
            harness_global_folder: Layered {
                value: false,
                from: Layer::BuiltIn,
            },
        }
    );
    assert_eq!(BUILT_IN.reply_style, ReplyStyle::ConcisePlus);
    assert_eq!(BUILT_IN.network_allowlist, &[] as &[&str]);
}

#[test]
fn reset_falls_back_to_the_layer_below() {
    let dir = Dir::new();
    dir.put(LayerFile::Global, r#"{"reply-style": "harness-default"}"#);
    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::ReplyStyle,
        &json!("concise-plus"),
    )
    .unwrap();

    let raised = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(raised.reply_style.from, Layer::Project);

    Settings::reset(&dir.config(), LayerFile::Project("web"), Field::ReplyStyle).unwrap();
    let fallen = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        fallen.reply_style,
        Layered {
            value: ReplyStyle::HarnessDefault,
            from: Layer::Global,
        }
    );

    Settings::reset(&dir.config(), LayerFile::Global, Field::ReplyStyle).unwrap();
    let built_in = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(built_in.reply_style.from, Layer::BuiltIn);
}

#[test]
fn a_setting_written_once_lands_in_its_own_layer_file() {
    let dir = Dir::new();
    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::NetworkAllowlist,
        &json!(["docs.rs"]),
    )
    .unwrap();
    Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::Host,
        &json!("localhost:11434"),
    )
    .unwrap();

    let project: Map<String, Value> =
        serde_json::from_str(&dir.text(LayerFile::Project("web"))).unwrap();
    assert_eq!(project.get("network-allowlist"), Some(&json!(["docs.rs"])));
    assert_eq!(project.get("$schema"), Some(&json!(SCHEMA_REF)));
    assert_eq!(
        project.get("host"),
        None,
        "a project's file must not carry a field set globally"
    );
    assert!(
        dir.config()
            .file(LayerFile::Global)
            .unwrap()
            .ends_with("settings.json"),
        "the global file is the config folder's own"
    );

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.network_allowlist,
        Layered {
            value: vec!["docs.rs".to_string()],
            from: Layer::Project,
        }
    );
    assert_eq!(
        settings.host,
        Layered {
            value: Some("localhost:11434".to_string()),
            from: Layer::Global,
        }
    );
}

#[test]
fn a_value_the_schema_refuses_is_not_written() {
    let dir = Dir::new();
    dir.put(LayerFile::Project("web"), r#"{"read-anywhere": true}"#);

    for (field, value) in [
        (Field::ReplyStyle, json!("loud")),
        (Field::ReadAnywhere, json!("yes")),
        (Field::Host, json!(40)),
        (Field::NetworkAllowlist, json!(["docs.rs", 40])),
        (Field::PmHarness, json!("claude")),
    ] {
        let error = Settings::set(&dir.config(), LayerFile::Project("web"), field, &value)
            .expect_err("the schema refuses this");
        assert!(
            matches!(error, SettingsError::ValueRefused { .. }),
            "{error}"
        );
        assert_eq!(
            dir.text(LayerFile::Project("web")),
            r#"{"read-anywhere": true}"#,
            "{field:?} wrote {value}"
        );
    }

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert!(settings.read_anywhere.value);
}

#[test]
fn a_null_clears_a_field_instead_of_hiding_the_layer_below() {
    let dir = Dir::new();
    dir.put(LayerFile::Global, r#"{"host": "localhost:11434"}"#);
    dir.put(LayerFile::Project("web"), r#"{"host": null}"#);

    let written = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        written.host,
        Layered {
            value: Some("localhost:11434".to_string()),
            from: Layer::Global,
        }
    );

    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::Host,
        &json!(null),
    )
    .unwrap();
    assert!(
        !dir.text(LayerFile::Project("web")).contains("host"),
        "{}",
        dir.text(LayerFile::Project("web"))
    );
}

#[test]
fn a_field_only_another_layer_holds_is_refused() {
    let dir = Dir::new();
    let error = Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::ReadAnywhere,
        &json!(true),
    )
    .unwrap_err();
    match error {
        SettingsError::FieldNotInLayer { field, layer } => {
            assert_eq!(field, Field::ReadAnywhere);
            assert_eq!(layer, Layer::Global);
        }
        other => panic!("not a field out of its layer: {other}"),
    }
    assert!(!dir.config().file(LayerFile::Global).unwrap().exists());

    dir.put(LayerFile::Global, r#"{"read-anywhere": true}"#);
    let error = Settings::load(&dir.config(), "web").unwrap_err();
    assert!(
        matches!(error, SettingsError::FileRefused { .. }),
        "{error}"
    );
}

#[test]
fn a_broken_file_is_named_and_kept() {
    let dir = Dir::new();
    let broken = "{\n  \"read-anywhere\": true,\n  \"host\" \"localhost:11434\"\n}\n";
    dir.put(LayerFile::Project("web"), broken);

    let error = Settings::load(&dir.config(), "web").unwrap_err();
    match error {
        SettingsError::Unparsed { ref file, line, .. } => {
            assert!(file.ends_with("settings.json"), "{file:?}");
            assert_eq!(line, 3, "{error}");
        }
        other => panic!("not an unparsed file: {other}"),
    }

    let error = Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::Host,
        &json!("x"),
    )
    .unwrap_err();
    assert!(matches!(error, SettingsError::Unparsed { .. }), "{error}");
    assert_eq!(dir.text(LayerFile::Project("web")), broken);
}

#[test]
fn a_file_that_is_not_an_object_is_named() {
    let dir = Dir::new();
    dir.put(LayerFile::Global, "[\"read-anywhere\"]");
    let error = Settings::load(&dir.config(), "paper").unwrap_err();
    assert!(matches!(error, SettingsError::Unparsed { .. }), "{error}");
}

#[test]
fn a_key_kelpie_has_no_setting_for_is_named() {
    let dir = Dir::new();
    dir.put(LayerFile::Global, r#"{"read_anywhere": true}"#);
    let error = Settings::load(&dir.config(), "paper").unwrap_err();
    match error {
        SettingsError::FileRefused { path, reason, .. } => {
            assert_eq!(path, "/read_anywhere");
            assert_eq!(reason, "is not a setting kelpie has");
        }
        other => panic!("not a refused file: {other}"),
    }
}

#[test]
fn a_project_id_cannot_name_a_folder_outside_projects() {
    let dir = Dir::new();
    for id in ["..", "../..", "web/../secret", ""] {
        let error = Settings::load(&dir.config(), id).unwrap_err();
        match error {
            SettingsError::ProjectIdNotUsable { id: got } => assert_eq!(got, id.to_string()),
            other => panic!("not an unusable project id: {other}"),
        }
    }
    assert!(!dir.0.join("secret").exists());
}

#[test]
fn the_checked_in_schema_matches_the_types() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("docs")
        .join("settings.schema.json");
    let checked_in = fs::read_to_string(&path).unwrap_or_default();
    let current = serde_json::to_string_pretty(&schema()).unwrap() + "\n";
    if checked_in != current {
        fs::write(&path, &current).unwrap();
        panic!(
            "{} was stale and has been rewritten from the Rust types: commit it",
            path.display()
        );
    }
}

#[test]
fn every_field_has_a_place_in_the_schema() {
    let document = schema();
    let properties = document.get("properties").unwrap().as_object().unwrap();
    for field in Field::ALL {
        assert!(
            properties.contains_key(field.key()),
            "the schema has no {}",
            field.key()
        );
        assert!(
            properties
                .get(field.key())
                .unwrap()
                .get("description")
                .is_some_and(Value::is_string),
            "{} has no help text",
            field.key()
        );
    }
    assert_eq!(Field::ALL.len(), properties.len() - 1, "$schema aside");
}

#[test]
fn the_check_covers_every_keyword_the_schema_uses() {
    let mut used = Vec::new();
    schema::keywords(&schema(), &mut used);
    let unknown: Vec<&String> = used
        .iter()
        .filter(|key| !schema::KNOWN.contains(&key.as_str()))
        .collect();
    assert!(
        unknown.is_empty(),
        "keywords the check does not know: {unknown:?}"
    );
}

#[test]
fn a_field_helps_the_person_reading_the_form() {
    let document = schema();
    let help = |key: &str| {
        document
            .get("properties")
            .unwrap()
            .get(key)
            .unwrap()
            .get("description")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(help("pm-harness").contains("pm"), "{}", help("pm-harness"));
    assert!(
        help("network-allowlist").contains("Allow for this project"),
        "{}",
        help("network-allowlist")
    );
}

#[test]
fn a_layer_shows_what_a_form_needs_to_offer_a_reset() {
    assert_eq!(Layer::BuiltIn.name(), "kelpie");
    assert_eq!(Layer::Project.to_string(), "project");
    assert_eq!(Field::PmHarness.layers(), &[Layer::Project]);
    assert_eq!(Field::ReplyStyle.layers(), &[Layer::Global, Layer::Project]);
    assert!(Layer::BuiltIn < Layer::Global && Layer::Global < Layer::Project);
}
