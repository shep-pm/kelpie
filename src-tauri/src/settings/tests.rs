use std::fs;

use kelpie_engine::login::Store;
use kelpie_engine::run::RunId;
use kelpie_engine::settings::ConfigDir;

use super::{FieldView, Target, settings_get, settings_reset, settings_schema, settings_set};
use crate::state::AppState;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;
use tauri::{Manager as _, WebviewUrl, WebviewWindowBuilder};

/// The login every call sends, which no response may repeat
const SECRET: &str = "sk-ant-oat01-the-secret";

/// What one command answered, as the text the webview would receive
fn invoke(
    webview: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    body: serde_json::Value,
) -> Result<String, String> {
    let request = InvokeRequest {
        cmd: cmd.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: "tauri://localhost".parse().unwrap(),
        body: InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    };
    match get_ipc_response(webview, request) {
        Ok(body) => Ok(body.deserialize::<serde_json::Value>().unwrap().to_string()),
        Err(error) => Err(error.to_string()),
    }
}

#[test]
fn no_command_returns_a_login() {
    let app = crate::builder_with(mock_builder(), Store::memory(), None)
        .build(mock_context(noop_assets()))
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .unwrap();
    let harness = serde_json::json!({ "harness": "claude" });
    let mut said = Vec::new();

    said.push(invoke(&webview, "login_status", harness.clone()));
    said.push(invoke(
        &webview,
        "login_set",
        serde_json::json!({ "harness": "claude", "value": SECRET }),
    ));
    said.push(invoke(&webview, "login_status", harness.clone()));
    said.push(invoke(
        &webview,
        "login_set",
        serde_json::json!({ "harness": "claude", "value": "  " }),
    ));
    said.push(invoke(&webview, "login_clear", harness.clone()));
    said.push(invoke(&webview, "login_status", harness));

    let said: Vec<String> = said
        .into_iter()
        .map(|answer| answer.unwrap_or_else(|error| error))
        .collect();
    assert_eq!(said[0], "false");
    assert_eq!(said[1], "null");
    assert_eq!(said[2], "true", "status says only that a login is set");
    assert!(said[3].contains("blank"), "{}", said[3]);
    assert_eq!(said[4], "null");
    assert_eq!(said[5], "false");
    for answer in &said {
        assert!(!answer.contains(SECRET), "{answer}");
    }
}

#[test]
fn there_is_no_command_that_reads_a_login_back() {
    let app = crate::builder_with(mock_builder(), Store::memory(), None)
        .build(mock_context(noop_assets()))
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .unwrap();

    for cmd in ["login_get", "login_read", "login_show"] {
        let answer = invoke(&webview, cmd, serde_json::json!({ "harness": "claude" }));
        assert!(answer.is_err(), "{cmd} answered {answer:?}");
    }
}

#[test]
fn a_harness_kelpie_cannot_ask_is_refused_before_anything_starts() {
    let dir = std::env::temp_dir().join(format!("kelpie-settings-{}", RunId::new().unwrap()));
    let state = AppState::open(
        dir.clone(),
        ConfigDir::at(dir.join("config")),
        Store::memory(),
        Box::new(|_| Err("never built".to_owned())),
        Box::new(|| Ok(None)),
        None,
    )
    .unwrap();
    let app = crate::builder_with(mock_builder(), Store::memory(), None)
        .manage(state)
        .build(mock_context(noop_assets()))
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .unwrap();

    let answer = invoke(
        &webview,
        "harness_options",
        serde_json::json!({ "harness": "goose", "refresh": false }),
    );

    let error = answer.expect_err("kelpie has no goose harness to ask");
    assert!(error.contains("goose"), "{error}");
    assert!(
        !dir.join("asks").exists(),
        "the refusal named a harness kelpie never started"
    );
    fs::remove_dir_all(dir).unwrap();
}

/// What kelpie keeps for one harness, with the time it was asked
const KEPT: &str = concat!(
    r#"{"asked_at":1,"models":[{"id":"small","name":"Small","#,
    r#""description":null}],"efforts":[]}"#
);

/// The same answer, as the command sends it to the webview
const OFFERED: &str = concat!(
    r#"{"models":[{"id":"small","name":"Small","#,
    r#""description":null}],"efforts":[]}"#
);

#[test]
fn a_kept_answer_is_read_without_installing_or_starting_anything() {
    let dir = std::env::temp_dir().join(format!("kelpie-settings-{}", RunId::new().unwrap()));
    fs::create_dir_all(dir.join("options")).unwrap();
    fs::write(dir.join("options").join("claude.json"), KEPT).unwrap();
    let state = AppState::open(
        dir.clone(),
        ConfigDir::at(dir.join("config")),
        Store::memory(),
        Box::new(|_| Err("never built".to_owned())),
        Box::new(|| Ok(None)),
        None,
    )
    .unwrap();
    let app = crate::builder_with(mock_builder(), Store::memory(), None)
        .manage(state)
        .build(mock_context(noop_assets()))
        .unwrap();
    let webview = WebviewWindowBuilder::new(&app, "main", WebviewUrl::default())
        .build()
        .unwrap();

    let answer = invoke(
        &webview,
        "harness_options",
        serde_json::json!({ "harness": "claude", "refresh": false }),
    )
    .unwrap();

    assert_eq!(answer, OFFERED);
    assert!(
        !dir.join("bridges").exists(),
        "the kept answer installed no bridge: {}",
        dir.display()
    );
    fs::remove_dir_all(dir).unwrap();
}

/// An app on the mock runtime with a project picked, its settings in `dir/config`
fn app_with_project(dir: &std::path::Path) -> tauri::App<tauri::test::MockRuntime> {
    let state = AppState::open(
        dir.to_path_buf(),
        ConfigDir::at(dir.join("config")),
        Store::memory(),
        Box::new(|_| Err("never built".to_owned())),
        Box::new(|| Ok(None)),
        None,
    )
    .unwrap();
    state.set_project(dir.to_path_buf()).unwrap();
    mock_builder()
        .manage(state)
        .build(mock_context(noop_assets()))
        .unwrap()
}

fn field<'a>(fields: &'a [FieldView], key: &str) -> &'a FieldView {
    fields.iter().find(|field| field.key == key).unwrap()
}

fn fresh_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kelpie-settings-{}", RunId::new().unwrap()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A write through `app`, which is where its handle and its state are
fn set(
    app: &tauri::App<tauri::test::MockRuntime>,
    layer: Target,
    path: String,
    value: serde_json::Value,
) -> Result<(), String> {
    settings_set(
        app.handle().clone(),
        app.state::<AppState>(),
        layer,
        path,
        value,
    )
}

fn reset(
    app: &tauri::App<tauri::test::MockRuntime>,
    layer: Target,
    path: String,
) -> Result<(), String> {
    settings_reset(app.handle().clone(), app.state::<AppState>(), layer, path)
}

#[test]
fn a_field_says_which_layer_its_value_came_from() {
    let dir = fresh_dir();
    let app = app_with_project(&dir);
    let state = app.state::<AppState>();
    let project = state.project_id(None).unwrap();
    let on = || Target::Project {
        id: project.clone(),
    };

    set(&app, Target::Global, "host".into(), "http://one".into()).unwrap();
    set(&app, on(), "reply-style".into(), "harness-default".into()).unwrap();
    let seen = settings_get(state.clone(), None).unwrap();

    let project_fields = seen.project.unwrap().fields;
    assert_eq!(field(&project_fields, "host").value, "http://one");
    assert_eq!(field(&project_fields, "host").from, "global");
    assert_eq!(field(&project_fields, "reply-style").from, "project");
    assert_eq!(field(&project_fields, "read-anywhere").from, "kelpie");
    // The global tab holds only what the global layer can hold, and never a project's value
    assert_eq!(field(&seen.global, "reply-style").from, "kelpie");
    assert!(seen.global.iter().all(|f| f.key != "network-allowlist"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_worker_fields_on_the_project_tab_are_what_a_worker_runs_as() {
    let dir = fresh_dir();
    let app = app_with_project(&dir);
    let state = app.state::<AppState>();
    let project = state.project_id(None).unwrap();
    let agents = dir.join("config").join("agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join("worker.md"),
        "---\nharness: goose\nmodel: global-model\neffort: low\n---\n",
    )
    .unwrap();

    // The global `worker.md`'s keys show as the global layer's
    let seen = settings_get(state.clone(), None).unwrap();
    let fields = seen.project.unwrap().fields;
    assert_eq!(field(&fields, "worker-model").value, "global-model");
    assert_eq!(field(&fields, "worker-model").from, "global");
    assert_eq!(field(&fields, "worker-harness").value, "goose");
    assert_eq!(field(&fields, "worker-harness").from, "global");
    assert_eq!(field(&fields, "worker-effort").value, "low");
    assert_eq!(field(&fields, "worker-effort").from, "global");

    // The project's own setting wins, and says so
    set(
        &app,
        Target::Project { id: project },
        "worker-model".into(),
        "project-model".into(),
    )
    .unwrap();
    let seen = settings_get(state.clone(), None).unwrap();
    let fields = seen.project.unwrap().fields;
    assert_eq!(field(&fields, "worker-model").value, "project-model");
    assert_eq!(field(&fields, "worker-model").from, "project");
    assert_eq!(field(&fields, "worker-effort").from, "global");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_pm_fields_on_the_global_tab_are_the_global_pm_agents_keys() {
    let dir = fresh_dir();
    let app = app_with_project(&dir);
    let state = app.state::<AppState>();

    let before = settings_get(state.clone(), None).unwrap();
    assert_eq!(field(&before.global, "pm-model").value, "opus");
    assert_eq!(field(&before.global, "pm-model").from, "kelpie");
    assert_eq!(field(&before.global, "pm-harness").value, "claude-code");

    set(&app, Target::Global, "pm-model".into(), "sonnet".into()).unwrap();
    let after = settings_get(state.clone(), None).unwrap();

    assert_eq!(field(&after.global, "pm-model").value, "sonnet");
    assert_eq!(field(&after.global, "pm-model").from, "global");
    let project = after.project.unwrap().fields;
    assert_eq!(field(&project, "pm-model").value, "sonnet");
    assert_eq!(field(&project, "pm-model").from, "global");
    let text = fs::read_to_string(dir.join("config/agents/pm.md")).unwrap();
    assert!(text.contains("model: sonnet"), "{text}");
    assert!(!dir.join("config/settings.json").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reset_falls_back_to_the_layer_below() {
    let dir = fresh_dir();
    let app = app_with_project(&dir);
    let state = app.state::<AppState>();
    let project = state.project_id(None).unwrap();
    let on = || Target::Project {
        id: project.clone(),
    };
    set(
        &app,
        Target::Global,
        "pm-model".into(),
        "global-model".into(),
    )
    .unwrap();
    set(&app, on(), "pm-model".into(), "project-model".into()).unwrap();

    reset(&app, on(), "pm-model".into()).unwrap();
    let seen = settings_get(state.clone(), None)
        .unwrap()
        .project
        .unwrap()
        .fields;
    assert_eq!(field(&seen, "pm-model").value, "global-model");
    assert_eq!(field(&seen, "pm-model").from, "global");

    reset(&app, Target::Global, "pm-model".into()).unwrap();
    let seen = settings_get(state.clone(), None)
        .unwrap()
        .project
        .unwrap()
        .fields;
    assert_eq!(field(&seen, "pm-model").value, "opus");
    assert_eq!(field(&seen, "pm-model").from, "kelpie");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_blank_value_clears_the_field_and_a_bad_one_is_refused() {
    let dir = fresh_dir();
    let app = app_with_project(&dir);
    let state = app.state::<AppState>();
    set(&app, Target::Global, "host".into(), "http://one".into()).unwrap();
    set(&app, Target::Global, "host".into(), "  ".into()).unwrap();
    let seen = settings_get(state.clone(), None).unwrap().global;
    assert_eq!(field(&seen, "host").from, "kelpie");

    let error = set(&app, Target::Global, "reply-style".into(), "loud".into()).unwrap_err();
    assert!(error.contains("reply-style"), "{error}");
    let error = set(&app, Target::Global, "nope".into(), 1.into()).unwrap_err();
    assert!(error.contains("nope"), "{error}");
    let error = set(
        &app,
        Target::Global,
        "network-allowlist".into(),
        serde_json::json!(["a.example"]),
    )
    .unwrap_err();
    assert!(error.contains("network-allowlist"), "{error}");
    let error = set(
        &app,
        Target::Project { id: "../x".into() },
        "host".into(),
        "h".into(),
    )
    .unwrap_err();
    assert!(error.contains("no project"), "{error}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn before_a_project_is_picked_the_global_tab_still_reads() {
    let dir = fresh_dir();
    let state = AppState::open(
        dir.clone(),
        ConfigDir::at(dir.join("config")),
        Store::memory(),
        Box::new(|_| Err("never built".to_owned())),
        Box::new(|| Ok(None)),
        None,
    )
    .unwrap();
    let app = mock_builder()
        .manage(state)
        .build(mock_context(noop_assets()))
        .unwrap();

    let seen = settings_get(app.state::<AppState>(), None).unwrap();

    assert!(seen.project.is_none());
    assert!(!seen.global.is_empty());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn each_bridge_has_its_own_entry_in_the_update_setting() {
    let dir = fresh_dir();
    let app = app_with_project(&dir);
    let state = app.state::<AppState>();
    let policy = |map| set(&app, Target::Global, "bridge-updates".into(), map);

    // The bridge row writes one entry, and the field says where it came from
    policy(serde_json::json!({"claude-acp": "registry-ask-first"})).unwrap();
    let seen = settings_get(state.clone(), None).unwrap().global;
    let held = field(&seen, "bridge-updates");
    assert_eq!(held.value["claude-acp"], "registry-ask-first");
    assert_eq!(held.from, "global");

    // A value kelpie does not offer is refused, and names the field
    let error = policy(serde_json::json!({"claude-acp": "whenever"})).unwrap_err();
    assert!(error.contains("bridge-updates"), "{error}");

    // The row's last entry dropped: the field falls back to kelpie
    policy(serde_json::Value::Null).unwrap();
    let seen = settings_get(state.clone(), None).unwrap().global;
    assert_eq!(field(&seen, "bridge-updates").from, "kelpie");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_schema_the_form_is_drawn_from_names_every_field() {
    let schema = settings_schema();
    for key in ["pm-model", "reply-style", "network-allowlist"] {
        assert!(schema["properties"][key].is_object(), "{key}");
    }
}
