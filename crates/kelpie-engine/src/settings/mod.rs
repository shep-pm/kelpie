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

use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use schemars::JsonSchema;
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

    /// Where a harness's model runs, such as a local server
    Provider,

    /// The host a provider runs on
    Host,

    /// How kelpie asks a harness to phrase replies
    ReplyStyle,

    /// Domains a sandbox request saved with *Allow for this project*
    NetworkAllowlist,

    /// Whether an agent may read anywhere in the folder it works in
    ReadAnywhere,

    /// Whether a harness may read its own global config folder
    HarnessGlobalFolder,
}

impl Field {
    /// Every field, in the order a form shows them
    pub const ALL: [Field; 9] = [
        Field::PmHarness,
        Field::PmModel,
        Field::PmEffort,
        Field::Provider,
        Field::Host,
        Field::ReplyStyle,
        Field::NetworkAllowlist,
        Field::ReadAnywhere,
        Field::HarnessGlobalFolder,
    ];

    /// The key this field's value sits under in a settings file
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::PmHarness => "pm-harness",
            Self::PmModel => "pm-model",
            Self::PmEffort => "pm-effort",
            Self::Provider => "provider",
            Self::Host => "host",
            Self::ReplyStyle => "reply-style",
            Self::NetworkAllowlist => "network-allowlist",
            Self::ReadAnywhere => "read-anywhere",
            Self::HarnessGlobalFolder => "harness-global-folder",
        }
    }
}

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

    /// Where a harness's model runs, for a harness that reaches a model through a server
    pub provider: Option<String>,

    /// The host a provider runs on
    pub host: Option<String>,

    /// How kelpie asks a harness to phrase its replies
    pub reply_style: Option<ReplyStyle>,

    /// Domains a sandbox request may reach, one saved with *Allow for this project*
    pub network_allowlist: Option<Vec<String>>,

    /// Whether an agent may read anywhere by default, instead of only what its work touches
    pub read_anywhere: Option<bool>,

    /// Whether a harness may read the global config folder it keeps for itself
    pub harness_global_folder: Option<bool>,
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
}

/// kelpie's built-in layer, in code rather than a shipped file
///
/// Every field works empty, so kelpie ships defaults and never requirements
/// (ADR 0006).
pub const BUILT_IN: Defaults = Defaults {
    pm_harness: None,
    pm_model: None,
    pm_effort: None,
    provider: None,
    host: None,
    reply_style: ReplyStyle::ConcisePlus,
    network_allowlist: &[],
    read_anywhere: false,
    harness_global_folder: false,
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
}

/// The JSON Schema a settings file must satisfy, from the Rust types
#[must_use]
pub fn schema() -> Value {
    schema::document().clone()
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
    let text = serde_json::to_string_pretty(&Value::Object(fields))
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
