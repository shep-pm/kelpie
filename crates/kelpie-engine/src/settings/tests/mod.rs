use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::run::RunId;

// Writing one field to one layer, in its own file to stay under the 1000-line cap
mod writes;

// The board's three fields, in their own file beside the types they read
mod tickets;

// The gate's three fields, in their own file beside the types they read
mod ci;

// The catch-up choice, in its own file beside the type it reads
mod git;

// The review pass's five fields, in their own file beside the types they read
mod review;

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
            worker_harness: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            worker_model: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            worker_effort: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            worker_slots: Layered {
                value: TWO_WORKERS,
                from: Layer::BuiltIn,
            },
            worker_turn_timeout: Layered {
                value: SIXTY_MINUTES,
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
            theme: Layered {
                value: Theme::System,
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
            run_without_sandbox: Layered {
                value: false,
                from: Layer::BuiltIn,
            },
            bridge_updates: Layered {
                value: BTreeMap::new(),
                from: Layer::BuiltIn,
            },
            lease_capacity: Layered {
                value: BTreeMap::from([(CARGO_TEST.to_string(), CARGO_TEST_CAPACITY)]),
                from: Layer::BuiltIn,
            },
            pick_up: Layered {
                value: PickUp::Me,
                from: Layer::BuiltIn,
            },
            ready_label: Layered {
                value: "ready-for-agent".to_string(),
                from: Layer::BuiltIn,
            },
            done_window: Layered {
                value: BUILT_IN.done_window,
                from: Layer::BuiltIn,
            },
            ci_holds_merge: Layered {
                value: true,
                from: Layer::BuiltIn,
            },
            fix_attempts: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            ci_wait: Layered {
                value: BUILT_IN.ci_wait,
                from: Layer::BuiltIn,
            },
            catch_up: Layered {
                value: CatchUp::Always,
                from: Layer::BuiltIn,
            },
            reviewers: Layered {
                value: Reviewers::built_in(),
                from: Layer::BuiltIn,
            },
            test_command: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            reviewer_harness: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            reviewer_model: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
            reviewer_effort: Layered {
                value: None,
                from: Layer::BuiltIn,
            },
        }
    );
    assert_eq!(BUILT_IN.reply_style, ReplyStyle::ConcisePlus);
    assert_eq!(BUILT_IN.network_allowlist, &[] as &[&str]);
}

#[test]
fn worker_turn_timeout_defaults_to_sixty_minutes() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.worker_turn_timeout.value.duration(),
        Duration::from_secs(60 * 60)
    );
    assert_eq!(settings.worker_turn_timeout.from, Layer::BuiltIn);

    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::WorkerTurnTimeout,
        &json!(90),
    )
    .unwrap();
    let longer = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        longer.worker_turn_timeout.value.duration(),
        Duration::from_secs(90 * 60)
    );
    assert_eq!(longer.worker_turn_timeout.from, Layer::Project);

    let other = Settings::load(&dir.config(), "paper").unwrap();
    assert_eq!(
        other.worker_turn_timeout.value.duration(),
        Duration::from_secs(60 * 60),
        "the ceiling is one project's own"
    );

    for value in [
        json!(0),
        json!("60"),
        json!(-1),
        json!(MAX_TURN_MINUTES + 1),
    ] {
        let error = Settings::set(
            &dir.config(),
            LayerFile::Project("web"),
            Field::WorkerTurnTimeout,
            &value,
        )
        .expect_err("a ceiling of no minutes, or a typo, is refused");
        assert!(
            matches!(error, SettingsError::ValueRefused { .. }),
            "refuses {value}"
        );
    }
}

#[test]
fn worker_slots_default_to_two_and_cargo_test_capacity_to_three() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.worker_slots,
        Layered {
            value: Slots(2),
            from: Layer::BuiltIn
        }
    );
    assert_eq!(
        settings.lease_capacity_for(CARGO_TEST),
        Capacity(nonzero(3))
    );
    assert_eq!(
        settings.lease_capacity_for("e2e"),
        Capacity(nonzero(1)),
        "a lease kelpie names no capacity for holds one command at a time"
    );

    Settings::set(
        &dir.config(),
        LayerFile::Project("web"),
        Field::WorkerSlots,
        &json!(5),
    )
    .unwrap();
    Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::LeaseCapacity,
        &json!({ "cargo-test": 1, "e2e": 2 }),
    )
    .unwrap();

    let raised = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        raised.worker_slots,
        Layered {
            value: Slots(5),
            from: Layer::Project
        }
    );
    assert_eq!(raised.lease_capacity_for(CARGO_TEST), Capacity(nonzero(1)));
    assert_eq!(raised.lease_capacity_for("e2e"), Capacity(nonzero(2)));

    let other = Settings::load(&dir.config(), "paper").unwrap();
    assert_eq!(
        other.worker_slots,
        Layered {
            value: Slots(2),
            from: Layer::BuiltIn
        },
        "the cap is one project's own"
    );
    assert_eq!(other.lease_capacity_for(CARGO_TEST), Capacity(nonzero(1)));

    for (field, layer, value) in [
        (Field::WorkerSlots, LayerFile::Project("web"), json!("3")),
        (Field::WorkerSlots, LayerFile::Project("web"), json!(-1)),
        (
            Field::WorkerSlots,
            LayerFile::Project("web"),
            json!(MAX_COUNT + 1),
        ),
        (
            Field::LeaseCapacity,
            LayerFile::Global,
            json!({ "cargo-test": "3" }),
        ),
        (
            Field::LeaseCapacity,
            LayerFile::Global,
            json!({ "cargo-test": 0 }),
        ),
        (
            Field::LeaseCapacity,
            LayerFile::Global,
            json!({ "e2e": MAX_COUNT + 1 }),
        ),
    ] {
        let error = Settings::set(&dir.config(), layer, field, &value)
            .expect_err("the schema refuses this");
        assert!(
            matches!(error, SettingsError::ValueRefused { .. }),
            "{} refuses {value}",
            field.key()
        );
    }
    let kept = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(kept.worker_slots.value, Slots(5));
    assert_eq!(kept.lease_capacity_for(CARGO_TEST), Capacity(nonzero(1)));

    // A project may hold every worker out of its slots; a lease may not be emptied.
    Settings::set(
        &dir.config(),
        LayerFile::Project("paper"),
        Field::WorkerSlots,
        &json!(0),
    )
    .unwrap();
    let parked = Settings::load(&dir.config(), "paper").unwrap();
    assert_eq!(
        parked.worker_slots.value,
        Slots(0),
        "a project with no slots starts no worker"
    );
    assert_eq!(
        parked.lease_capacity_for(CARGO_TEST),
        Capacity(nonzero(1)),
        "the project's cap never empties a lease"
    );

    // A file written by hand is refused the same way a write is
    dir.put(
        LayerFile::Global,
        r#"{"lease-capacity": {"cargo-test": 0}}"#,
    );
    let error = Settings::load(&dir.config(), "web").expect_err("a lease with no place is refused");
    assert!(
        matches!(error, SettingsError::FileRefused { .. }),
        "{error}"
    );
}

#[test]
fn bridge_updates_default_to_registry_automatic() {
    let dir = Dir::new();

    let settings = Settings::load(&dir.config(), "web").unwrap();

    assert_eq!(
        settings.bridge_updates_for("claude-acp"),
        BridgeUpdates::RegistryAutomatic
    );
    assert_eq!(settings.bridge_updates.from, Layer::BuiltIn);

    Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::BridgeUpdates,
        &json!({ "claude-acp": "kelpies-pin" }),
    )
    .unwrap();
    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.bridge_updates_for("claude-acp"),
        BridgeUpdates::KelpiesPin
    );
    assert_eq!(
        settings.bridge_updates_for("codex-acp"),
        BridgeUpdates::RegistryAutomatic,
        "a bridge the map does not name keeps the default"
    );
}

#[test]
fn a_bridge_update_policy_the_schema_does_not_know_is_refused() {
    let dir = Dir::new();

    let error = Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::BridgeUpdates,
        &json!({ "claude-acp": "whenever" }),
    )
    .unwrap_err();

    assert!(
        matches!(error, SettingsError::ValueRefused { .. }),
        "{error}"
    );
    assert!(!dir.config().file(LayerFile::Global).unwrap().exists());
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

#[test]
fn the_global_view_ignores_every_project_layer() {
    let dir = Dir::new();
    dir.put(LayerFile::Global, r#"{"reply-style": "harness-default"}"#);
    dir.put(
        LayerFile::Project("web"),
        r#"{"reply-style": "concise-plus", "host": "http://localhost:11434"}"#,
    );

    let global = Settings::load_global(&dir.config()).unwrap();

    assert_eq!(
        global.reply_style,
        Layered {
            value: ReplyStyle::HarnessDefault,
            from: Layer::Global,
        }
    );
    assert_eq!(global.host.from, Layer::BuiltIn);
}

#[test]
fn a_reply_style_says_what_it_adds_to_the_instructions() {
    let concise = ReplyStyle::ConcisePlus.instructions().unwrap();
    assert!(concise.starts_with("## Replies"), "{concise}");
    assert!(concise.contains("table"), "{concise}");
    assert_eq!(ReplyStyle::HarnessDefault.instructions(), None);
}

#[test]
fn worker_harness_model_and_effort_resolve_project_over_global() {
    let dir = Dir::new();

    // Nothing set: kelpie names no worker harness, model or effort (ADR 0006)
    let settings = Settings::load(&dir.config(), "web").unwrap();
    for value in [&settings.worker_model, &settings.worker_effort] {
        assert_eq!(
            *value,
            Layered {
                value: None,
                from: Layer::BuiltIn
            }
        );
    }
    assert_eq!(settings.worker_harness.value, None);
    assert_eq!(settings.worker_harness.from, Layer::BuiltIn);

    // The global value is the global `worker` agent's own keys, so the global
    // settings file does not take one
    let refused = Settings::set(
        &dir.config(),
        LayerFile::Global,
        Field::WorkerModel,
        &json!("global-model"),
    );
    assert!(refused.is_err());
    let agents = dir.0.join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join("worker.md"),
        "---\nharness: goose\nmodel: global-model\neffort: low\n---\n",
    )
    .unwrap();
    dir.put(
        LayerFile::Project("web"),
        r#"{"worker-harness": "claude-code", "worker-model": "project-model", "worker-effort": "high"}"#,
    );

    let settings = Settings::load(&dir.config(), "web").unwrap();
    assert_eq!(
        settings.worker_harness,
        Layered {
            value: Some(Harness::ClaudeCode),
            from: Layer::Project
        }
    );
    assert_eq!(
        settings.worker_model,
        Layered {
            value: Some("project-model".to_owned()),
            from: Layer::Project
        }
    );
    assert_eq!(
        settings.worker_effort,
        Layered {
            value: Some("high".to_owned()),
            from: Layer::Project
        }
    );

    // The worker's binding takes the project's three over the global file's
    let agents = crate::agents::Agents::load(&dir.config()).unwrap();
    let binding = agents.worker_binding(&settings).unwrap();
    assert_eq!(binding.harness.value, Some(Harness::ClaudeCode));
    assert_eq!(binding.model.value.as_deref(), Some("project-model"));
    assert_eq!(binding.effort.value.as_deref(), Some("high"));

    // Another project that sets none sees the global file's
    let other = Settings::load(&dir.config(), "paper").unwrap();
    let binding = agents.worker_binding(&other).unwrap();
    assert_eq!(binding.harness.value, Some(Harness::Goose));
    assert_eq!(binding.model.value.as_deref(), Some("global-model"));
    assert_eq!(binding.effort.value.as_deref(), Some("low"));
}
