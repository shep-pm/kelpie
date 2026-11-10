//! The settings page's commands
//!
//! A login goes in and never comes out: [`login_set`] keeps it, [`login_status`]
//! says only whether one is set, and [`login_clear`] forgets it. No command
//! returns a login's value, and no error text holds one (`docs/design/settings.md`).
//! Each runs off the main thread, since a locked keychain can wait on the person.
//!
//! [`harness_options`] lists what a harness offers, from the harness itself.
//!
//! [`settings_get`], [`settings_set`], [`settings_reset`] and [`settings_schema`]
//! are the settings form's way in: every field it shows is written to the layer
//! file it sits in, never held here. The PM's harness, model and effort are one
//! value kept in two places by layer: the global `pm` agent's keys on the Global
//! tab, and the project's own `pm-*` settings on the Project tab.
//!
//! The bridge row beside a harness's login is `crate::bridges`, which reads what
//! the bridge checks wrote rather than the settings file.

use std::fs;

use kelpie_engine::agents::{Agents, Binding, Origin, PM, PmKey, WORKER};
use kelpie_engine::bridge;
use kelpie_engine::harness::claude;
use kelpie_engine::login::Store;
use kelpie_engine::sandbox::{self, Reads, Sandbox};
use kelpie_engine::session::options;
use kelpie_engine::settings::{ConfigDir, Field, Layer, LayerFile, Resolved, Settings};
use kelpie_protocol::Options;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::pm::{self, Defaulted};
use crate::state::AppState;

/// The folder under the data folder a harness is asked in
const ASKS_FOLDER: &str = "asks";

/// Keep `value` as `harness`'s login, replacing any it had
///
/// # Errors
///
/// The text of why the keychain refused it, or that the value was blank.
#[tauri::command(async)]
#[specta::specta]
pub fn login_set(logins: State<'_, Store>, harness: String, value: String) -> Result<(), String> {
    logins
        .set(&harness, &value)
        .map_err(|error| error.to_string())
}

/// Whether `harness` has a login
///
/// # Errors
///
/// The text of why the keychain could not be read.
#[tauri::command(async)]
#[specta::specta]
pub fn login_status(logins: State<'_, Store>, harness: String) -> Result<bool, String> {
    logins.status(&harness).map_err(|error| error.to_string())
}

/// Forget `harness`'s login
///
/// # Errors
///
/// The text of why the keychain refused it.
#[tauri::command(async)]
#[specta::specta]
pub fn login_clear(logins: State<'_, Store>, harness: String) -> Result<(), String> {
    logins.clear(&harness).map_err(|error| error.to_string())
}

/// What `harness` offers: the models and efforts it lists for itself
///
/// Kelpie asks a harness once and keeps its answer in the data folder, so
/// opening the settings page starts nothing. `refresh` asks it again. The list
/// is the harness's own, so kelpie names no model outside the default
/// (`docs/design/settings.md`).
///
/// # Errors
///
/// The text of why kelpie could not ask: the harness has no harness in kelpie
/// yet, its bridge could not be installed, it has no login, it could not be
/// started, or it did not answer.
#[tauri::command(async)]
#[specta::specta]
pub fn harness_options(
    harness: String,
    refresh: bool,
    state: State<'_, AppState>,
    logins: State<'_, Store>,
) -> Result<Options, String> {
    // Only Claude Code has a harness slice in kelpie (#12)
    if harness != claude::LOGIN {
        return Err(format!("kelpie has no {harness} harness to ask"));
    }
    // A kept answer answers without installing a bridge or starting a harness
    if !refresh && let Some(kept) = options::kept(state.data_dir(), &harness) {
        return Ok(kept);
    }
    let data = state.data_dir();
    let bridge = bridge::ensure(data, claude::pin()).map_err(|error| error.to_string())?;
    let asked = data.join(ASKS_FOLDER).join(&harness);
    fs::create_dir_all(&asked).map_err(|error| error.to_string())?;
    let harness = claude::harness(bridge, &asked, &logins).map_err(|error| error.to_string())?;
    // The ask sends no prompt, but kelpie starts no harness outside a sandbox
    // (ADR 0005): it reads and writes its own `asked` folder, reaches no host,
    // and the sandbox drops when the ask returns. One that cannot start fails
    // the ask, and nothing runs in its place.
    //
    // The ask belongs to no project, so it reads as narrowly as a session can:
    // `Reads::Project`, with every session's state folder closed to it.
    let sessions_root = state.sessions_root();
    let sandbox = Sandbox::start(
        &harness,
        &sandbox::Project {
            folder: &asked,
            network_allowlist: &[],
            reads: Reads::Project,
            sessions_root: &sessions_root,
        },
        &asked,
    )
    .map_err(|error| {
        format!(
            "could not start the sandbox to ask {}: {error}",
            claude::LOGIN
        )
    })?;
    options::offerings(
        &state.tree(),
        &harness,
        claude::LOGIN,
        sandbox,
        data,
        &asked,
        refresh,
    )
    .map_err(|error| error.to_string())
}

/// One field of the form: its value, and the layer the value came from
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct FieldView {
    /// The field's key in a settings file, such as `pm-model`
    pub key: String,

    /// The value as the file holds it, `null` where no layer sets one
    pub value: Value,

    /// Where it came from, as a person reads it: `kelpie`, `global`, `project`, or an agent such as `kelpie:pm`
    pub from: String,
}

/// The project tab: one project's fields over the global and built-in layers
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct ProjectView {
    /// The project's id, which a write to its layer names
    pub id: String,

    /// The folder the maintainer picked
    pub path: String,

    /// Every field, in the order the form shows them
    pub fields: Vec<FieldView>,
}

/// What the settings page shows: each tab's fields with the layer of each value
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct ResolvedView {
    /// The fields the global layer can hold, resolved from it alone
    pub global: Vec<FieldView>,

    /// The project's fields, `None` before a project is picked
    pub project: Option<ProjectView>,
}

/// The layer a write goes to
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// `settings.json`, and the global `pm` agent for the PM's harness, model and effort
    Global,

    /// One project's own settings
    Project {
        /// The project's id, from [`ProjectView`]
        id: String,
    },
}

/// The layer a value's origin is shown as: the global `pm` and `worker` agents are the global layer
fn from(origin: &Origin) -> String {
    match origin {
        Origin::File(name) if name == PM || name == WORKER => Layer::Global.name().to_owned(),
        other => other.to_string(),
    }
}

fn layered<T: Serialize>(field: Field, value: &T, layer: Layer) -> FieldView {
    FieldView {
        key: field.key().to_owned(),
        value: serde_json::to_value(value).unwrap_or(Value::Null),
        from: layer.name().to_owned(),
    }
}

/// The form's fields in order, from the settings and the PM's and the worker's resolved bindings
///
/// Without a worker binding, the worker's fields show what the settings alone say.
fn fields(
    resolved: &Resolved,
    pm: &Defaulted,
    worker: Option<&Binding>,
    only: impl Fn(Field) -> bool,
) -> Vec<FieldView> {
    let sourced = |field: Field, value: Value, origin: &Origin| FieldView {
        key: field.key().to_owned(),
        value,
        from: from(origin),
    };
    Field::ALL
        .into_iter()
        .filter(|field| only(*field))
        .map(|field| match field {
            Field::PmHarness => sourced(field, json_of(&Some(pm.harness.value)), &pm.harness.from),
            Field::PmModel => sourced(field, json_of(&pm.model.value), &pm.model.from),
            Field::PmEffort => sourced(field, json_of(&pm.effort.value), &pm.effort.from),
            Field::WorkerHarness => match worker {
                Some(w) => sourced(field, json_of(&w.harness.value), &w.harness.from),
                None => layered(
                    field,
                    &resolved.worker_harness.value,
                    resolved.worker_harness.from,
                ),
            },
            Field::WorkerModel => match worker {
                Some(w) => sourced(field, json_of(&w.model.value), &w.model.from),
                None => layered(
                    field,
                    &resolved.worker_model.value,
                    resolved.worker_model.from,
                ),
            },
            Field::WorkerEffort => match worker {
                Some(w) => sourced(field, json_of(&w.effort.value), &w.effort.from),
                None => layered(
                    field,
                    &resolved.worker_effort.value,
                    resolved.worker_effort.from,
                ),
            },
            Field::WorkerSlots => layered(
                field,
                &resolved.worker_slots.value,
                resolved.worker_slots.from,
            ),
            Field::LeaseCapacity => layered(
                field,
                &resolved.lease_capacity.value,
                resolved.lease_capacity.from,
            ),
            Field::Provider => layered(field, &resolved.provider.value, resolved.provider.from),
            Field::Host => layered(field, &resolved.host.value, resolved.host.from),
            Field::ReplyStyle => layered(
                field,
                &resolved.reply_style.value,
                resolved.reply_style.from,
            ),
            Field::NetworkAllowlist => layered(
                field,
                &resolved.network_allowlist.value,
                resolved.network_allowlist.from,
            ),
            Field::ReadAnywhere => layered(
                field,
                &resolved.read_anywhere.value,
                resolved.read_anywhere.from,
            ),
            Field::HarnessGlobalFolder => layered(
                field,
                &resolved.harness_global_folder.value,
                resolved.harness_global_folder.from,
            ),
            Field::BridgeUpdates => layered(
                field,
                &resolved.bridge_updates.value,
                resolved.bridge_updates.from,
            ),
            // `Field` grows with each slice's settings, and a form shows only those it has
            _ => FieldView {
                key: field.key().to_owned(),
                value: Value::Null,
                from: Layer::BuiltIn.name().to_owned(),
            },
        })
        .collect()
}

fn json_of<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// Read every field of both tabs, each with the layer it came from
///
/// The Global tab shows what the global layer and the global `pm` agent say. The
/// Project tab shows what the project sees: its own file over those. `project`
/// is a project's id, and `None` is the first project, since Phase 1 has one.
///
/// # Errors
///
/// The text of which settings or agent file cannot be used, and why.
#[tauri::command(async)]
#[specta::specta]
pub fn settings_get(
    state: State<'_, AppState>,
    project: Option<String>,
) -> Result<ResolvedView, String> {
    let config = state.config();
    let agents = Agents::load(config).map_err(|error| error.to_string())?;
    let global_pm = agents.pm().map_err(|error| error.to_string())?;
    let global_settings = Settings::load_global(config).map_err(|error| error.to_string())?;
    let global_binding = Binding {
        harness: global_pm.harness,
        model: global_pm.model,
        effort: global_pm.effort,
        body: global_pm.body,
    };
    let global = fields(
        &global_settings,
        &pm::with_defaults(&global_binding),
        None,
        |field| field.layers().contains(&Layer::Global) || is_pm(field),
    );
    let project = match state.project_id(project.as_deref()) {
        Ok(id) => {
            let settings = Settings::load(config, &id).map_err(|error| error.to_string())?;
            let binding = agents
                .pm_binding(&settings)
                .map_err(|error| error.to_string())?;
            let worker = agents
                .worker_binding(&settings)
                .map_err(|error| error.to_string())?;
            let path = state
                .project_path_of(&id)
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            Some(ProjectView {
                id,
                path,
                fields: fields(
                    &settings,
                    &pm::with_defaults(&binding),
                    Some(&worker),
                    |_| true,
                ),
            })
        }
        // No project yet: the Project tab has nothing to show
        Err(_) if state.project_path().is_none() => None,
        Err(error) => return Err(error),
    };
    Ok(ResolvedView { global, project })
}

/// Whether `field` is one of the PM's three, which the global layer keeps in the `pm` agent
fn is_pm(field: Field) -> bool {
    pm_key(field).is_some()
}

fn pm_key(field: Field) -> Option<PmKey> {
    match field {
        Field::PmHarness => Some(PmKey::Harness),
        Field::PmModel => Some(PmKey::Model),
        Field::PmEffort => Some(PmKey::Effort),
        _ => None,
    }
}

fn field_named(path: &str) -> Result<Field, String> {
    Field::ALL
        .into_iter()
        .find(|field| field.key() == path)
        .ok_or_else(|| format!("kelpie has no setting named {path}"))
}

/// The layer file `target` names, checked against the projects kelpie holds
fn layer_file(state: &AppState, target: &Target) -> Result<Option<String>, String> {
    match target {
        Target::Global => Ok(None),
        Target::Project { id } => state.project_id(Some(id)).map(Some),
    }
}

/// Write one field of one layer
///
/// The value is checked against the settings schema before anything is written.
/// `null` and a blank string clear the field, which is what an empty form field
/// means. On the Global tab the PM's harness, model and effort are written to the
/// global `pm` agent's file, and everything else to `settings.json`. The PM's
/// next message uses the result.
///
/// # Errors
///
/// The text of why the value was refused, or the file could not be written.
#[tauri::command(async)]
#[specta::specta]
pub fn settings_set(
    state: State<'_, AppState>,
    layer: Target,
    path: String,
    value: Value,
) -> Result<(), String> {
    let field = field_named(&path)?;
    let project = layer_file(&state, &layer)?;
    let value = match value {
        Value::String(text) if text.trim().is_empty() => Value::Null,
        other => other,
    };
    write(state.config(), field, project.as_deref(), Some(&value))
}

/// Clear one field of one layer, so its value falls back to the layer below
///
/// # Errors
///
/// The text of why the file could not be written.
#[tauri::command(async)]
#[specta::specta]
pub fn settings_reset(
    state: State<'_, AppState>,
    layer: Target,
    path: String,
) -> Result<(), String> {
    let field = field_named(&path)?;
    let project = layer_file(&state, &layer)?;
    write(state.config(), field, project.as_deref(), None)
}

/// Set, or with `None` reset, `field` in the layer file of `project`, or the global one
fn write(
    config: &ConfigDir,
    field: Field,
    project: Option<&str>,
    value: Option<&Value>,
) -> Result<(), String> {
    let at = match project {
        Some(id) => LayerFile::Project(id),
        None => LayerFile::Global,
    };
    if project.is_none()
        && let Some(key) = pm_key(field)
    {
        let text = match value {
            None | Some(Value::Null) => None,
            Some(Value::String(text)) => Some(text.as_str()),
            Some(other) => return Err(format!("{} takes text, not {other}", field.key())),
        };
        return Agents::set_pm_key(config, key, text).map_err(|error| error.to_string());
    }
    match value {
        Some(value) => Settings::set(config, at, field, value),
        None => Settings::reset(config, at, field),
    }
    .map_err(|error| error.to_string())
}

/// The JSON Schema every settings file satisfies, from the Rust types
///
/// The form is rendered from it, and a field's description is its help text.
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn settings_schema() -> Value {
    kelpie_engine::settings::schema()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use kelpie_engine::login::Store;
    use kelpie_engine::run::RunId;
    use kelpie_engine::settings::ConfigDir;

    use super::{FieldView, Target, settings_get, settings_reset, settings_schema, settings_set};
    use crate::state::AppState;
    use tauri::ipc::{CallbackFn, InvokeBody};
    use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
    use tauri::webview::InvokeRequest;
    use tauri::{WebviewUrl, WebviewWindowBuilder};

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

    #[test]
    fn a_field_says_which_layer_its_value_came_from() {
        use tauri::Manager as _;
        let dir = fresh_dir();
        let app = app_with_project(&dir);
        let state = app.state::<AppState>();
        let project = state.project_id(None).unwrap();
        let on = || Target::Project {
            id: project.clone(),
        };

        settings_set(
            state.clone(),
            Target::Global,
            "host".into(),
            "http://one".into(),
        )
        .unwrap();
        settings_set(
            state.clone(),
            on(),
            "reply-style".into(),
            "harness-default".into(),
        )
        .unwrap();
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
        use tauri::Manager as _;
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
        settings_set(
            state.clone(),
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
        use tauri::Manager as _;
        let dir = fresh_dir();
        let app = app_with_project(&dir);
        let state = app.state::<AppState>();

        let before = settings_get(state.clone(), None).unwrap();
        assert_eq!(field(&before.global, "pm-model").value, "opus");
        assert_eq!(field(&before.global, "pm-model").from, "kelpie");
        assert_eq!(field(&before.global, "pm-harness").value, "claude-code");

        settings_set(
            state.clone(),
            Target::Global,
            "pm-model".into(),
            "sonnet".into(),
        )
        .unwrap();
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
        use tauri::Manager as _;
        let dir = fresh_dir();
        let app = app_with_project(&dir);
        let state = app.state::<AppState>();
        let project = state.project_id(None).unwrap();
        let on = || Target::Project {
            id: project.clone(),
        };
        settings_set(
            state.clone(),
            Target::Global,
            "pm-model".into(),
            "global-model".into(),
        )
        .unwrap();
        settings_set(
            state.clone(),
            on(),
            "pm-model".into(),
            "project-model".into(),
        )
        .unwrap();

        settings_reset(state.clone(), on(), "pm-model".into()).unwrap();
        let seen = settings_get(state.clone(), None)
            .unwrap()
            .project
            .unwrap()
            .fields;
        assert_eq!(field(&seen, "pm-model").value, "global-model");
        assert_eq!(field(&seen, "pm-model").from, "global");

        settings_reset(state.clone(), Target::Global, "pm-model".into()).unwrap();
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
        use tauri::Manager as _;
        let dir = fresh_dir();
        let app = app_with_project(&dir);
        let state = app.state::<AppState>();
        settings_set(
            state.clone(),
            Target::Global,
            "host".into(),
            "http://one".into(),
        )
        .unwrap();

        settings_set(state.clone(), Target::Global, "host".into(), "  ".into()).unwrap();
        let seen = settings_get(state.clone(), None).unwrap().global;
        assert_eq!(field(&seen, "host").from, "kelpie");

        let error = settings_set(
            state.clone(),
            Target::Global,
            "reply-style".into(),
            "loud".into(),
        )
        .unwrap_err();
        assert!(error.contains("reply-style"), "{error}");
        let error =
            settings_set(state.clone(), Target::Global, "nope".into(), 1.into()).unwrap_err();
        assert!(error.contains("nope"), "{error}");
        let error = settings_set(
            state.clone(),
            Target::Global,
            "network-allowlist".into(),
            serde_json::json!(["a.example"]),
        )
        .unwrap_err();
        assert!(error.contains("network-allowlist"), "{error}");
        let error = settings_set(
            state.clone(),
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
        use tauri::Manager as _;
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
        use tauri::Manager as _;
        let dir = fresh_dir();
        let app = app_with_project(&dir);
        let state = app.state::<AppState>();
        let set = |map: serde_json::Value| {
            settings_set(state.clone(), Target::Global, "bridge-updates".into(), map)
        };

        // The bridge row writes one entry, and the field says where it came from
        set(serde_json::json!({"claude-acp": "registry-ask-first"})).unwrap();
        let seen = settings_get(state.clone(), None).unwrap().global;
        let held = field(&seen, "bridge-updates");
        assert_eq!(held.value["claude-acp"], "registry-ask-first");
        assert_eq!(held.from, "global");

        // A value kelpie does not offer is refused, and names the field
        let error = set(serde_json::json!({"claude-acp": "whenever"})).unwrap_err();
        assert!(error.contains("bridge-updates"), "{error}");

        // The row's last entry dropped: the field falls back to kelpie
        set(serde_json::Value::Null).unwrap();
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
}
