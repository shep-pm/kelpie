//! Settings: one schema, read in layers from `~/.config/kelpie`
//!
//! The types here are the one place a setting is described. [`schema`] writes
//! their JSON Schema, and the form, the validation and the checked-in
//! `docs/settings.schema.json` all come from that. A layer file holds only the
//! fields it sets, and every read and write checks it against the schema first.
//! [`layers`] resolves a field to the nearest layer that sets it and names the
//! layer the value came from.
//!
//! The built-in layer is [`BUILT_IN`] in code, not a shipped file. A harness's
//! login is not a setting here at all: it lives in the OS keychain.
//!
//! # Quick start
//!
//! ```no_run
//! use kelpie_engine::settings::{ConfigDir, Field, Layer, LayerFile, Settings};
//! use serde_json::json;
//!
//! let dir = ConfigDir::from_env()?;
//! Settings::set(&dir, LayerFile::Project("web"), Field::ReadAnywhere, &json!(true))?;
//! let settings = Settings::load(&dir, "web")?;
//! assert!(settings.read_anywhere.value);
//! assert_eq!(settings.read_anywhere.from, Layer::Project);
//! # Ok::<(), kelpie_engine::settings::SettingsError>(())
//! ```

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub mod layers;
mod schema;

#[cfg(test)]
mod tests;

pub use layers::{Layer, LayerFile, Layered};

/// The schema every settings file points at with `$schema`
pub const SCHEMA_REF: &str =
    "https://raw.githubusercontent.com/shep-pm/kelpie/main/docs/settings.schema.json";

/// The key a settings file names its schema under
const SCHEMA_KEY: &str = "$schema";

/// The folder settings, agent files and kelpie-global instructions live in
const CONFIG_FOLDER: &str = ".config/kelpie";

/// The file one layer's settings live in
const SETTINGS_FILE: &str = "settings.json";

/// The folder a project's own settings live under, inside the config folder
const PROJECTS_FOLDER: &str = "projects";

/// One settings writer at a time in this process
///
/// A layer file is read, changed in memory and written back as one step, so two
/// writes to the same file cannot each lose the other's field. kelpie has one
/// run owning its config folder (ADR 0001), so no lock across processes is held.
static WRITES: Mutex<()> = Mutex::new(());

// ADR 0008: the config folder is `~/.config/kelpie` on Windows too, so the home
// folder is the one the OS calls home, not a platform-specific config path.
#[cfg(windows)]
const HOME_VAR: &str = "USERPROFILE";

#[cfg(not(windows))]
const HOME_VAR: &str = "HOME";

/// kelpie's config folder: `~/.config/kelpie` on every OS (ADR 0008)
///
/// Never Tauri's config path, so the folder can sit in a dotfiles repo or a
/// synced folder. It is created when a setting is first written, never on a read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDir(PathBuf);

impl ConfigDir {
    /// The config folder under `home`
    #[must_use]
    pub fn from_home(home: &Path) -> Self {
        Self(home.join(CONFIG_FOLDER))
    }

    /// The config folder under this process's home folder
    ///
    /// # Errors
    ///
    /// The OS gives no home folder to read.
    pub fn from_env() -> Result<Self, SettingsError> {
        let home = std::env::var_os(HOME_VAR)
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .ok_or(SettingsError::NoHome)?;
        Ok(Self::from_home(&home))
    }

    /// A config folder of your own, for a test or a person who moved it
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// The folder itself
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The file one layer's settings live in
    ///
    /// # Errors
    ///
    /// The project id is not one folder name.
    pub fn file(&self, at: LayerFile<'_>) -> Result<PathBuf, SettingsError> {
        match at {
            LayerFile::Global => Ok(self.0.join(SETTINGS_FILE)),
            LayerFile::Project(id) if usable(id) => {
                Ok(self.0.join(PROJECTS_FOLDER).join(id).join(SETTINGS_FILE))
            }
            LayerFile::Project(id) => Err(SettingsError::ProjectIdNotUsable { id: id.to_string() }),
        }
    }
}

/// A project id names one folder under `projects`, and nothing else
///
/// `:` is refused beside the separators: on Windows `PathBuf::join` with an id
/// like `D:x` replaces the whole path it is joined onto.
fn usable(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// A harness kelpie can run a session on
// wire format: a settings file holds these strings
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Harness {
    /// Claude Code
    ClaudeCode,

    /// goose
    Goose,
}

/// How kelpie asks a harness to phrase its replies
// wire format: a settings file holds these strings
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ReplyStyle {
    /// Kelpie's own style: short prose with lists, tables and before and after comparisons
    ConcisePlus,

    /// Whatever the harness does when kelpie says nothing about its output
    HarnessDefault,
}

impl ReplyStyle {
    /// The instructions this style adds to an agent's own, `None` where it adds none
    ///
    /// The style is part of the instructions, so it is the same on every harness
    /// (ADR 0006).
    #[must_use]
    pub fn instructions(self) -> Option<&'static str> {
        match self {
            Self::ConcisePlus => Some(CONCISE_PLUS),
            Self::HarnessDefault => None,
        }
    }
}

/// Kelpie's own reply style, as instructions
const CONCISE_PLUS: &str = "## Replies

- Lead with the answer or the decision needed, then the reasons.
- Keep to short prose. Use a list for steps or parallel items, a table for a comparison across several attributes, and a before and after for a change.
- Say what you know and what you do not. Mark a guess as one.
- Ask one question at a time, with your recommendation, when the maintainer has to choose.";

/// Where a bridge's new versions come from (`docs/design/bridge-updates.md`, The setting)
// wire format: a settings file holds these strings
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BridgeUpdates {
    /// The default. A version that passes the probe is used at each session's next idle point
    #[default]
    RegistryAutomatic,

    /// A quiet notice with an Update button. It is not in the needs-you count, since nothing is blocked
    RegistryAskFirst,

    /// Only the version in kelpie's code, which changes with kelpie's releases
    KelpiesPin,
}

/// One settings field
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Field {
    /// Which harness the PM runs on
    PmHarness,

    /// Which model the PM runs on
    PmModel,

    /// How much effort the PM puts in
    PmEffort,

    /// Which harness a worker runs on
    WorkerHarness,

    /// Which model a worker runs on
    WorkerModel,

    /// How much effort a worker puts in
    WorkerEffort,

    /// Where a harness's model runs, such as a local server
    Provider,

    /// The host a provider runs on
    Host,

    /// How kelpie asks a harness to phrase replies
    ReplyStyle,

    /// Domains a sandbox request saved with *Allow for this project*
    NetworkAllowlist,

    /// Whether an agent reads anywhere, not only what its work touches
    ReadAnywhere,

    /// Whether a harness may read its own global config folder
    HarnessGlobalFolder,

    /// Where each bridge's new versions come from
    BridgeUpdates,

    /// How many of a project's workers may run at once
    WorkerSlots,

    /// How long a worker's turn may run before kelpie stops it
    WorkerTurnTimeout,

    /// How many commands may hold each lease at once
    LeaseCapacity,
}

impl Field {
    /// Every field, in the order a form shows them
    pub const ALL: [Field; 16] = [
        Field::PmHarness,
        Field::PmModel,
        Field::PmEffort,
        Field::WorkerHarness,
        Field::WorkerModel,
        Field::WorkerEffort,
        Field::WorkerSlots,
        Field::WorkerTurnTimeout,
        Field::Provider,
        Field::Host,
        Field::ReplyStyle,
        Field::NetworkAllowlist,
        Field::ReadAnywhere,
        Field::HarnessGlobalFolder,
        Field::BridgeUpdates,
        Field::LeaseCapacity,
    ];

    /// The key this field's value sits under in a settings file
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::PmHarness => "pm-harness",
            Self::PmModel => "pm-model",
            Self::PmEffort => "pm-effort",
            Self::WorkerHarness => "worker-harness",
            Self::WorkerModel => "worker-model",
            Self::WorkerEffort => "worker-effort",
            Self::WorkerSlots => "worker-slots",
            Self::WorkerTurnTimeout => "worker-turn-timeout",
            Self::Provider => "provider",
            Self::Host => "host",
            Self::ReplyStyle => "reply-style",
            Self::NetworkAllowlist => "network-allowlist",
            Self::ReadAnywhere => "read-anywhere",
            Self::HarnessGlobalFolder => "harness-global-folder",
            Self::BridgeUpdates => "bridge-updates",
            Self::LeaseCapacity => "lease-capacity",
        }
    }
}

/// How many of a project's workers run at once
///
/// 0 is a project holding every worker out of its slots: no worker starts a turn.
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Slots(u32);

impl Slots {
    /// How many places there are, as a count kelpie can index with
    #[must_use]
    pub fn places(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

/// How long a worker's turn may run, in whole minutes
///
/// Never zero: a ceiling of no time would stop every turn before it began. It
/// counts from when the worker's slot was granted ([`crate::worker::ceiling`]).
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnTimeout(NonZeroU32);

impl TurnTimeout {
    /// The ceiling as a length of time
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_secs(u64::from(self.0.get()) * SECONDS_PER_MINUTE)
    }
}

/// How many commands hold a lease at once
///
/// Never zero: a lease with no place can never be taken, and every command
/// queued behind it then waits forever.
// wire format: a settings file holds it
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Capacity(NonZeroU32);

impl Capacity {
    /// How many places there are, as a count kelpie can index with
    #[must_use]
    pub fn places(self) -> usize {
        usize::try_from(self.0.get()).unwrap_or(usize::MAX)
    }

    /// The count itself, for the lease that holds this many commands at once
    #[must_use]
    pub(crate) const fn count(self) -> NonZeroU32 {
        self.0
    }
}

/// The most a settings file may name, past which it is a typo
///
/// A count this large names more agents or commands than a machine can run at
/// once. Naming it in the schema is what keeps a write from landing a file kelpie
/// then cannot read back.
const MAX_COUNT: u32 = 64;

/// The longest turn ceiling a settings file may name, one day in minutes
///
/// Past it a stuck turn holds its slot for longer than a person looks back at it.
const MAX_TURN_MINUTES: u32 = 24 * 60;

const SECONDS_PER_MINUTE: u64 = 60;

/// A count the schema bounds, as the nonzero one [`Capacity`] holds
const fn nonzero(count: u32) -> NonZeroU32 {
    match NonZeroU32::new(count) {
        Some(count) => count,
        None => panic!("a lease always has a place"),
    }
}

impl JsonSchema for Slots {
    fn schema_name() -> Cow<'static, str> {
        "Slots".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 0,
            "maximum": MAX_COUNT,
        })
    }
}

impl JsonSchema for TurnTimeout {
    fn schema_name() -> Cow<'static, str> {
        "TurnTimeout".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_TURN_MINUTES,
        })
    }
}

impl JsonSchema for Capacity {
    fn schema_name() -> Cow<'static, str> {
        "Capacity".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "minimum": 1,
            "maximum": MAX_COUNT,
        })
    }
}

/// The lease every worker's `cargo test` run queues behind
pub const CARGO_TEST: &str = "cargo-test";

/// How many commands hold [`CARGO_TEST`] at once when no layer names it
pub const CARGO_TEST_CAPACITY: Capacity = Capacity(nonzero(3));

/// How many commands hold a lease no layer names a capacity for
const ONE_AT_A_TIME: Capacity = Capacity(nonzero(1));

/// How long a worker's turn may run when no layer names a ceiling, as SK's
/// `worker.turn_timeout` has it
const SIXTY_MINUTES: TurnTimeout = TurnTimeout(nonzero(60));

/// How many of a project's workers run at once when no layer names a cap
const TWO_WORKERS: Slots = Slots(2);

/// The settings of one layer file, holding only the fields that file sets
///
/// `settings.json` is the global layer's file and `projects/<id>/settings.json`
/// one project's. These types are what the schema is written from, so a file
/// kelpie reads can hold nothing they do not describe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Settings {
    /// The schema this file is written against
    #[serde(rename = "$schema")]
    pub schema_ref: Option<String>,

    /// Which harness the PM's next message runs on. Where it is empty, the `pm` agent's own harness decides.
    pub pm_harness: Option<Harness>,

    /// Which model the PM runs on, named as the harness names it. Empty leaves the model to the harness.
    pub pm_model: Option<String>,

    /// How much effort the PM puts in, named as the harness names it. Empty leaves the effort to the harness.
    pub pm_effort: Option<String>,

    /// Which harness a worker runs on. Where it is empty, the `worker` agent's own harness decides.
    pub worker_harness: Option<Harness>,

    /// Which model a worker runs on, named as the harness names it. Empty leaves the model to the harness.
    pub worker_model: Option<String>,

    /// How much effort a worker puts in, named as the harness names it. Empty leaves the effort to the harness.
    pub worker_effort: Option<String>,

    /// How many of this project's workers run at once. A worker that is Paused or waiting on the maintainer holds no slot. 0 holds every worker out of the project's slots.
    pub worker_slots: Option<Slots>,

    /// How many minutes a worker's turn may run before kelpie cancels it and pauses the worker. The count starts when the worker's slot is granted, not when its process launches.
    pub worker_turn_timeout: Option<TurnTimeout>,

    /// Where a harness's model runs, for a harness that reaches a model through a server
    pub provider: Option<String>,

    /// The host a provider runs on
    pub host: Option<String>,

    /// How kelpie asks a harness to phrase its replies
    pub reply_style: Option<ReplyStyle>,

    /// Domains a sandbox request may reach, one saved with *Allow for this project*
    pub network_allowlist: Option<Vec<String>>,

    /// Whether an agent reads anywhere, instead of only the project it works in and the folders beside it. Every harness's login, every secret and every other session's state folder stay denied. The change lands when the session next starts, on a relaunch or a harness switch.
    pub read_anywhere: Option<bool>,

    /// Whether a harness may read the global config folder it keeps for itself: Claude Code's `~/.claude`, or `CLAUDE_CONFIG_DIR` when the person set it, and goose's `~/.config/goose`, or the `config` folder under `GOOSE_PATH_ROOT` when the person set it. Each applies when the session next starts.
    pub harness_global_folder: Option<bool>,

    /// Where each bridge's new versions come from, keyed by the bridge's registry id such as `claude-acp`. A bridge the map does not name takes `registry-automatic`.
    pub bridge_updates: Option<BTreeMap<String, BridgeUpdates>>,

    /// How many commands hold each lease at once, keyed by the lease's name such as `cargo-test`. A lease the map does not name takes kelpie's own capacity for it, and no lease holds zero.
    pub lease_capacity: Option<BTreeMap<String, Capacity>>,
}

/// The built-in layer: what every field holds when no file sets it
///
/// [`Field::layers`] says which layers may set a field at all, and a field none
/// of them sets falls back to here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Defaults {
    /// The built-in PM harness
    pub pm_harness: Option<Harness>,

    /// The built-in PM model
    pub pm_model: Option<&'static str>,

    /// The built-in PM effort
    pub pm_effort: Option<&'static str>,

    /// The built-in worker harness
    pub worker_harness: Option<Harness>,

    /// The built-in worker model
    pub worker_model: Option<&'static str>,

    /// The built-in worker effort
    pub worker_effort: Option<&'static str>,

    /// How many of a project's workers run at once when no layer caps them
    pub worker_slots: Slots,

    /// How long a worker's turn may run when no layer sets a ceiling
    pub worker_turn_timeout: TurnTimeout,

    /// The built-in provider
    pub provider: Option<&'static str>,

    /// The built-in host
    pub host: Option<&'static str>,

    /// The built-in reply style
    pub reply_style: ReplyStyle,

    /// The built-in network allowlist
    pub network_allowlist: &'static [&'static str],

    /// The built-in read rule
    pub read_anywhere: bool,

    /// The built-in rule for a harness's own global folder
    pub harness_global_folder: bool,

    /// The capacity kelpie gives each lease it names, by name
    pub lease_capacity: &'static [(&'static str, Capacity)],
}

/// kelpie's built-in layer, in code rather than a shipped file
///
/// Every field works empty, so kelpie ships defaults and never requirements
/// (ADR 0006).
pub const BUILT_IN: Defaults = Defaults {
    pm_harness: None,
    pm_model: None,
    pm_effort: None,
    worker_harness: None,
    worker_model: None,
    worker_effort: None,
    worker_slots: TWO_WORKERS,
    worker_turn_timeout: SIXTY_MINUTES,
    provider: None,
    host: None,
    reply_style: ReplyStyle::ConcisePlus,
    network_allowlist: &[],
    read_anywhere: false,
    harness_global_folder: false,
    lease_capacity: &[(CARGO_TEST, CARGO_TEST_CAPACITY)],
};

/// Every setting's value, each with the layer kelpie took it from
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The PM's harness
    pub pm_harness: Layered<Option<Harness>>,

    /// The PM's model
    pub pm_model: Layered<Option<String>>,

    /// The PM's effort
    pub pm_effort: Layered<Option<String>>,

    /// A worker's harness
    pub worker_harness: Layered<Option<Harness>>,

    /// A worker's model
    pub worker_model: Layered<Option<String>>,

    /// A worker's effort
    pub worker_effort: Layered<Option<String>>,

    /// How many of the project's workers run at once
    pub worker_slots: Layered<Slots>,

    /// How long a worker's turn may run
    pub worker_turn_timeout: Layered<TurnTimeout>,

    /// Where a harness's model runs
    pub provider: Layered<Option<String>>,

    /// The host a provider runs on
    pub host: Layered<Option<String>>,

    /// How kelpie asks a harness to phrase replies
    pub reply_style: Layered<ReplyStyle>,

    /// Domains a sandbox request may reach
    pub network_allowlist: Layered<Vec<String>>,

    /// Whether an agent may read anywhere by default
    pub read_anywhere: Layered<bool>,

    /// Whether a harness may read its own global config folder
    pub harness_global_folder: Layered<bool>,

    /// Where each bridge's new versions come from, for the bridges a file names
    pub bridge_updates: Layered<BTreeMap<String, BridgeUpdates>>,

    /// How many commands hold each lease at once, for the leases a file names
    pub lease_capacity: Layered<BTreeMap<String, Capacity>>,
}

impl Resolved {
    /// Where `bridge`'s new versions come from: what the settings name for it, or the default
    #[must_use]
    pub fn bridge_updates_for(&self, bridge: &str) -> BridgeUpdates {
        self.bridge_updates
            .value
            .get(bridge)
            .copied()
            .unwrap_or_default()
    }

    /// How many commands hold `lease` at once: what the settings name for it,
    /// kelpie's own capacity for it, or one at a time
    #[must_use]
    pub fn lease_capacity_for(&self, lease: &str) -> Capacity {
        self.lease_capacity
            .value
            .get(lease)
            .copied()
            .or_else(|| {
                BUILT_IN
                    .lease_capacity
                    .iter()
                    .find(|(name, _)| *name == lease)
                    .map(|(_, capacity)| *capacity)
            })
            .unwrap_or(ONE_AT_A_TIME)
    }
}

/// The JSON Schema a settings file must satisfy, from the Rust types
///
/// Keys are sorted, so the text of `docs/settings.schema.json` does not depend
/// on which features the build turned on.
#[must_use]
pub fn schema() -> Value {
    let mut document = schema::document().clone();
    document.sort_all_objects();
    document
}

impl Settings {
    /// Read the global and project layer files and resolve every field
    ///
    /// A file that is not there is not an error, and nothing is created.
    ///
    /// # Errors
    ///
    /// See [`SettingsError`]: a file that does not parse, one that holds a field
    /// its layer cannot hold, or one that cannot be read.
    pub fn load(dir: &ConfigDir, project_id: &str) -> Result<Resolved, SettingsError> {
        let global = schema::read(dir, LayerFile::Global)?;
        let project = schema::read(dir, LayerFile::Project(project_id))?;
        layers::resolve(&global, &project)
    }

    /// Read the global layer file and resolve every field from it alone
    ///
    /// What the form's Global tab shows: no project's file is read.
    ///
    /// # Errors
    ///
    /// As [`Settings::load`], for the global file.
    pub fn load_global(dir: &ConfigDir) -> Result<Resolved, SettingsError> {
        let global = schema::read(dir, LayerFile::Global)?;
        let project = layers::LayerDoc {
            file: PathBuf::new(),
            fields: Map::new(),
        };
        layers::resolve(&global, &project)
    }

    /// Write one field of one layer, leaving the rest of the file as it was
    ///
    /// The value is checked against the field's own place in the schema before
    /// anything is written, and the file is replaced by a rename of a temporary
    /// file beside it. A `null` clears the field instead, which is what an empty
    /// form field means. One write happens at a time in this process, so a write
    /// never drops a field another write just made.
    ///
    /// # Errors
    ///
    /// See [`SettingsError`]: the field belongs to another layer, the schema
    /// refuses the value, the file does not parse, or the write fails. A file
    /// that does not parse is left exactly as it was.
    pub fn set(
        dir: &ConfigDir,
        at: LayerFile<'_>,
        field: Field,
        value: &Value,
    ) -> Result<(), SettingsError> {
        let file = dir.file(at)?;
        let _held = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let mut doc = schema::read(dir, at)?;
        let rule = schema::field_schema(field, at.layer())?;
        if value.is_null() {
            doc.fields.remove(field.key());
        } else {
            if let Some((_, reason)) = schema::check(&rule, value, field.key()) {
                return Err(SettingsError::ValueRefused { field, reason });
            }
            doc.fields.insert(field.key().to_string(), value.clone());
        }
        write(&file, &doc.fields)
    }

    /// Clear one field of one layer, so its value falls back to the layer below
    ///
    /// A layer that does not set the field writes nothing at all.
    ///
    /// # Errors
    ///
    /// As [`Settings::set`], less the value.
    pub fn reset(dir: &ConfigDir, at: LayerFile<'_>, field: Field) -> Result<(), SettingsError> {
        let file = dir.file(at)?;
        let _held = WRITES.lock().unwrap_or_else(PoisonError::into_inner);
        let mut doc = schema::read(dir, at)?;
        if doc.fields.remove(field.key()).is_none() {
            return Ok(());
        }
        write(&file, &doc.fields)
    }
}

/// Replace one layer's file with `fields`, creating its folder when it is not there
///
/// The text is on disk in the temporary file before the rename points the layer
/// at it, so a write that dies partway leaves the old file or the whole new one.
/// The rename itself is not durable on its own: a folder sync is not something
/// `std` offers on every OS kelpie ships on.
fn write(file: &Path, fields: &Map<String, Value>) -> Result<(), SettingsError> {
    let failed = |source: io::Error| SettingsError::Write {
        file: file.to_path_buf(),
        source,
    };
    let mut fields = fields.clone();
    fields.insert(
        SCHEMA_KEY.to_string(),
        Value::String(SCHEMA_REF.to_string()),
    );
    // Another crate in the build can turn on `serde_json`'s `preserve_order`,
    // which would write keys in the order they were set
    let mut document = Value::Object(fields);
    document.sort_all_objects();
    let text = serde_json::to_string_pretty(&document)
        .map_err(|source| failed(io::Error::other(source)))?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }
    let temp = file.with_extension("json.tmp");
    // A write that got this far and then failed leaves no stray file in a folder
    // the design means to sit in a dotfiles repo or a synced folder.
    let dropped = |source: io::Error| {
        let _ = fs::remove_file(&temp);
        failed(source)
    };
    let mut handle = fs::File::create(&temp).map_err(failed)?;
    handle.write_all(text.as_bytes()).map_err(dropped)?;
    handle.sync_all().map_err(dropped)?;
    // Windows refuses to rename a file a handle still has open
    drop(handle);
    fs::rename(&temp, file).map_err(dropped)
}

/// Why kelpie could not read, use or write a setting
///
/// Two variants carry an [`io::Error`], so the enum has no `PartialEq`: match the
/// variant and read what it names.
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum SettingsError {
    /// The file's JSON does not parse: `message` says what broke at `line` and `column`
    Unparsed {
        /// The file kelpie could not read as JSON
        file: PathBuf,
        /// The line the JSON broke at
        line: usize,
        /// The column the JSON broke at
        column: usize,
        /// What the parser said
        message: String,
    },

    /// The file holds something its layer's schema refuses, such as a field only another layer may set
    FileRefused {
        /// The file that holds it
        file: PathBuf,
        /// Where in the file it is
        path: String,
        /// The rule it breaks
        reason: String,
    },

    /// A value `Settings::set` was given breaks the rule for its field
    ValueRefused {
        /// The field the value was for
        field: Field,
        /// The rule it breaks
        reason: String,
    },

    /// The field is set at a layer that cannot hold it
    FieldNotInLayer {
        /// The field
        field: Field,
        /// The layer it was set at
        layer: Layer,
    },

    /// The generated schema has no place for the field, so nothing could be checked against it
    FieldNotInSchema {
        /// The field
        field: Field,
    },

    /// The file could not be read
    Read {
        /// The file
        file: PathBuf,
        /// What the OS said
        source: io::Error,
    },

    /// The file, or the folder above it, could not be written
    Write {
        /// The file
        file: PathBuf,
        /// What the OS said
        source: io::Error,
    },

    /// The OS gives no home folder for `~/.config/kelpie` to hang under
    NoHome,

    /// The project id is not one folder name, so it cannot name `projects/<id>/settings.json`
    ProjectIdNotUsable {
        /// The id kelpie was given
        id: String,
    },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unparsed {
                file,
                line,
                column,
                message,
            } => {
                write!(f, "{file:?} is not JSON: {message} at {line}:{column}")
            }
            Self::FileRefused { file, path, reason } => {
                write!(
                    f,
                    "{file:?} is not settings this layer can hold: {path} {reason}"
                )
            }
            Self::ValueRefused { field, reason } => {
                write!(f, "{} {reason}", field.key())
            }
            Self::FieldNotInLayer { field, layer } => write!(
                f,
                "{} cannot be set at the {} layer",
                field.key(),
                layer.name()
            ),
            Self::FieldNotInSchema { field } => {
                write!(f, "the settings schema has no {} field", field.key())
            }
            Self::Read { file, source } => write!(f, "cannot read {file:?}: {source}"),
            Self::Write { file, source } => write!(f, "cannot write {file:?}: {source}"),
            Self::NoHome => write!(f, "no home folder to hold {CONFIG_FOLDER}"),
            Self::ProjectIdNotUsable { id } => {
                write!(f, "{id:?} is not one folder name for {PROJECTS_FOLDER}")
            }
        }
    }
}

impl core::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}
