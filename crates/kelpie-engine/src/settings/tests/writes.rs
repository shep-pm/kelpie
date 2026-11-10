//! Writing one field to one layer, and what a refused or racing write leaves
//!
//! Each test sets or updates one field, then reads the layer file back as text:
//! what landed in it, what the schema kept out, and what a second writer must
//! not lose.

use std::sync::Barrier;

use serde_json::{Map, Value, json};

use super::*;

/// How many races one write test runs
///
/// A lost update needs both threads to read before either writes, which the OS
/// is free not to arrange. Each round writes values the last round did not, so
/// any round that loses one shows up, and the lock means none can.
const ROUNDS: usize = 50;

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
fn a_null_in_a_file_is_not_counted_as_that_layer_setting_it() {
    let dir = Dir::new();
    dir.put(
        LayerFile::Global,
        r#"{"read-anywhere": null, "pm-model": null}"#,
    );

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(settings.read_anywhere.from, Layer::BuiltIn);
    assert_eq!(
        settings.pm_model,
        Layered {
            value: None,
            from: Layer::BuiltIn
        }
    );

    Settings::reset(&dir.config(), LayerFile::Global, Field::ReadAnywhere).unwrap();
    Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::Host,
        &json!("localhost:11434"),
    )
    .unwrap();
    let kept = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(kept.host.value.as_deref(), Some("localhost:11434"));
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
fn two_writes_to_one_layer_keep_both_fields() {
    let dir = Dir::new();
    let config = dir.config();
    for round in 0..ROUNDS {
        let host = format!("host-{round}");
        let provider = format!("provider-{round}");
        let barrier = Barrier::new(2);
        let writes = std::thread::scope(|scope| {
            let started =
                [(Field::Host, &host), (Field::Provider, &provider)].map(|(field, value)| {
                    let config = &config;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        Settings::set(config, LayerFile::Global, field, &json!(value))
                    })
                });
            started
                .into_iter()
                .map(|write| write.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(
            writes.iter().all(Result::is_ok),
            "round {round} failed a write"
        );
        let settings = Settings::load(&config, "web").unwrap();
        assert_eq!(
            settings.host.value.as_deref(),
            Some(host.as_str()),
            "round {round} lost the host"
        );
        assert_eq!(
            settings.provider.value.as_deref(),
            Some(provider.as_str()),
            "round {round} lost the provider"
        );
    }
}

#[test]
fn an_update_adds_to_what_the_layers_already_resolve_to() {
    let dir = Dir::new();
    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::NetworkAllowlist,
        &json!(["docs.rs"]),
    )
    .unwrap();

    Settings::update(
        &dir.config(),
        LayerFile::Project("web"),
        Field::NetworkAllowlist,
        |settings| {
            let mut hosts = settings.network_allowlist.value.clone();
            hosts.push("crates.io".to_owned());
            Value::Array(hosts.into_iter().map(Value::String).collect())
        },
    )
    .unwrap();

    let hosts = Settings::load(&dir.config(), "web")
        .unwrap()
        .network_allowlist;

    assert_eq!(hosts.value, ["docs.rs", "crates.io"]);
    assert_eq!(hosts.from, Layer::Project, "the project's own file set it");
}

#[test]
fn two_updates_of_one_list_keep_both_entries() {
    let dir = Dir::new();
    let config = dir.config();
    for round in 0..ROUNDS {
        let (first, second) = (format!("first-{round}.dev"), format!("second-{round}.dev"));
        let barrier = Barrier::new(2);
        let added = std::thread::scope(|scope| {
            [first.as_str(), second.as_str()]
                .map(|host| {
                    let (config, barrier) = (&config, &barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        Settings::update(
                            config,
                            LayerFile::Project("web"),
                            Field::NetworkAllowlist,
                            |settings| {
                                let mut hosts = settings.network_allowlist.value.clone();
                                hosts.push(host.to_owned());
                                Value::Array(hosts.into_iter().map(Value::String).collect())
                            },
                        )
                    })
                })
                .map(|write| write.join().unwrap())
        });
        assert!(
            added.iter().all(Result::is_ok),
            "round {round} failed a write"
        );
        let hosts = Settings::load(&config, "web")
            .unwrap()
            .network_allowlist
            .value;
        assert!(
            hosts.contains(&first) && hosts.contains(&second),
            "round {round} lost one: {hosts:?}"
        );
    }
}

#[test]
fn a_project_id_cannot_name_a_folder_outside_projects() {
    let dir = Dir::new();
    for id in [
        "..",
        "../..",
        "web/../secret",
        "",
        "C:",
        "D:x",
        "web:secret",
        "web\x00",
    ] {
        let error = Settings::load(&dir.config(), id).unwrap_err();
        match error {
            SettingsError::ProjectIdNotUsable { id: got } => assert_eq!(got, id.to_string()),
            other => panic!("not an unusable project id: {other}"),
        }
    }
    assert!(!dir.0.join("secret").exists());
}

#[test]
fn a_layer_file_lists_its_keys_in_sorted_order_whatever_was_set_first() {
    let dir = Dir::new();
    for (field, value) in [
        (Field::ReadAnywhere, json!(true)),
        (Field::NetworkAllowlist, json!(["docs.rs"])),
        (Field::HarnessGlobalFolder, json!(false)),
    ] {
        Settings::set(&dir.config(), LayerFile::Project("web"), field, &value).unwrap();
    }

    let text = dir.text(LayerFile::Project("web"));

    let keys: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("  \""))
        .filter_map(|line| line.split('"').next())
        .collect();
    assert_eq!(
        keys,
        [
            "$schema",
            "harness-global-folder",
            "network-allowlist",
            "read-anywhere"
        ],
        "{text}"
    );
}
