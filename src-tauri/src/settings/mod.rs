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
use std::sync::{Mutex, PoisonError};

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
use tauri::{AppHandle, Runtime, State};

use crate::pm::{self, Defaulted};
use crate::state::AppState;
use crate::theme;

/// The folder under the data folder a harness is asked in
const ASKS_FOLDER: &str = "asks";

/// One theme write at a time, across its save and the window change it makes
///
/// [`settings_set`] runs off the main thread, so two quick theme picks land on two
/// threads, and each saves the file, reads it back and themes the window. Held
/// across all three, this makes the write that lands last the one that themes the
/// window last, so the window always ends up on the theme the file holds. The
/// engine's own write lock covers a layer file's read-modify-write and is let go
/// before the window change, so it cannot reach that change.
///
/// `pub(crate)` so a test can hold it and show a theme write waits on it.
pub(crate) static THEME_WRITES: Mutex<()> = Mutex::new(());

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
    // The ask is kelpie's own, not a project's session: the person's global
    // folder stays out of it (`harness-global-folder` is a project setting)
    let harness =
        claude::harness(bridge, &asked, &logins, false).map_err(|error| error.to_string())?;
    // The ask sends no prompt, but kelpie starts no harness outside a sandbox
    // (ADR 0005): it reads and writes its own `asked` folder, reaches no host,
    // and the sandbox drops when the ask returns. One that cannot start fails
    // the ask, and nothing runs in its place.
    //
    // The ask belongs to no project, so it reads as narrowly as a session can:
    // `Reads::Project`, with every session's state folder closed to it.
    let sessions_root = state.sessions_root();
    let sandbox = crate::sandbox_setup::proven(
        Sandbox::start(
            &harness,
            &sandbox::Project {
                folder: &asked,
                network_allowlist: &[],
                reads: Reads::Project,
                sessions_root: &sessions_root,
            },
            &asked,
        ),
        |sandbox| sandbox.ready(&state.tree()),
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
            Field::WorkerTurnTimeout => layered(
                field,
                &resolved.worker_turn_timeout.value,
                resolved.worker_turn_timeout.from,
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
            Field::Theme => layered(field, &resolved.theme.value, resolved.theme.from),
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
            Field::RunWithoutSandbox => layered(
                field,
                &resolved.run_without_sandbox.value,
                resolved.run_without_sandbox.from,
            ),
            Field::BridgeUpdates => layered(
                field,
                &resolved.bridge_updates.value,
                resolved.bridge_updates.from,
            ),
            Field::PickUp => layered(field, &resolved.pick_up.value, resolved.pick_up.from),
            Field::ReadyLabel => layered(
                field,
                &resolved.ready_label.value,
                resolved.ready_label.from,
            ),
            Field::DoneWindow => layered(
                field,
                &resolved.done_window.value,
                resolved.done_window.from,
            ),
            Field::CiHoldsMerge => layered(
                field,
                &resolved.ci_holds_merge.value,
                resolved.ci_holds_merge.from,
            ),
            Field::FixAttempts => layered(
                field,
                &resolved.fix_attempts.value,
                resolved.fix_attempts.from,
            ),
            Field::CiWait => layered(field, &resolved.ci_wait.value, resolved.ci_wait.from),
            Field::CatchUp => layered(field, &resolved.catch_up.value, resolved.catch_up.from),
            Field::Reviewers => layered(field, &resolved.reviewers.value, resolved.reviewers.from),
            Field::TestCommand => layered(
                field,
                &resolved.test_command.value,
                resolved.test_command.from,
            ),
            Field::ReviewerHarness => layered(
                field,
                &resolved.reviewer_harness.value,
                resolved.reviewer_harness.from,
            ),
            Field::ReviewerModel => layered(
                field,
                &resolved.reviewer_model.value,
                resolved.reviewer_model.from,
            ),
            Field::ReviewerEffort => layered(
                field,
                &resolved.reviewer_effort.value,
                resolved.reviewer_effort.from,
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
/// Project tab shows the fields its layer may set, over what the layers below it
/// say for each. `project` is a project's id, and `None` is the first project,
/// since Phase 1 has one.
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
                    |field| field.layers().contains(&Layer::Project) || is_pm(field),
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
/// next message uses the result, and a theme shows on the window at once.
///
/// # Errors
///
/// The text of why the value was refused, the file could not be written, or the
/// window could not be themed.
#[tauri::command(async)]
#[specta::specta]
pub fn settings_set<R: Runtime>(
    app: AppHandle<R>,
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
    write(
        &app,
        state.config(),
        field,
        project.as_deref(),
        Some(&value),
    )
}

/// Clear one field of one layer, so its value falls back to the layer below
///
/// # Errors
///
/// The text of why the file could not be written.
#[tauri::command(async)]
#[specta::specta]
pub fn settings_reset<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    layer: Target,
    path: String,
) -> Result<(), String> {
    let field = field_named(&path)?;
    let project = layer_file(&state, &layer)?;
    write(&app, state.config(), field, project.as_deref(), None)
}

/// Set, or with `None` reset, `field` in the layer file of `project`, or the global one
///
/// A theme the write lands is read back from the layers and set on the window, so
/// the change shows without a restart (`docs/design/settings.md`, When a change
/// applies). A theme write holds [`THEME_WRITES`] over the save, the read-back and
/// the window change, so no other theme write slips between them.
fn write<R: Runtime>(
    app: &AppHandle<R>,
    config: &ConfigDir,
    field: Field,
    project: Option<&str>,
    value: Option<&Value>,
) -> Result<(), String> {
    let at = match project {
        Some(id) => LayerFile::Project(id),
        None => LayerFile::Global,
    };
    let _theme_write = (field == Field::Theme)
        .then(|| THEME_WRITES.lock().unwrap_or_else(PoisonError::into_inner));
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
    .map_err(|error| error.to_string())?;
    if field == Field::Theme {
        let theme = Settings::load_global(config)
            .map(|settings| settings.theme.value)
            .map_err(|error| error.to_string())?;
        theme::apply_to_main(app, theme)?;
    }
    Ok(())
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
mod tests;
