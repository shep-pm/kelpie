//! The error every settings read and write returns
//!
//! Each variant names the file or the field that broke. The three carrying an
//! [`io::Error`] leave the enum without `PartialEq`, so a caller matches the
//! variant and reads what it names.

use std::fmt;
use std::io;
use std::path::PathBuf;

use super::{CONFIG_FOLDER, Field, Layer, PROJECTS_FOLDER};

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
