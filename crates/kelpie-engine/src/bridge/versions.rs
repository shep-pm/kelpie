//! Which versions of one bridge kelpie uses, has ready, and has turned away
//!
//! [`Versions`] is `<data_dir>/bridges/<id>/versions.json`. It holds the version
//! in use, the one before it, the one that passed the probe and waits for its
//! swap, and the versions held back or marked bad, each with the reason
//! (`docs/design/bridge-updates.md`, Flow). With no file, the version in use is
//! kelpie's pin.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};

use super::{BRIDGES_FOLDER, Pin, is_name, token};

/// The file one bridge's versions are kept in, beside its installed versions
const FILE: &str = "versions.json";

/// A version kelpie did not use, and why
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejected {
    /// The version
    pub version: Version,

    /// What went wrong with it, as the person reads it
    pub reason: String,
}

/// The versions of one bridge
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Versions {
    /// The version sessions start
    pub in_use: Version,

    /// The version in use before this one, which a failed swap goes back to
    #[serde(default)]
    pub previous: Option<Version>,

    /// A newer version that is installed and passed the probe, and no session has swapped to
    #[serde(default)]
    pub ready: Option<Version>,

    /// Versions that failed the probe, kept to show why
    #[serde(default)]
    pub held_back: Vec<Rejected>,

    /// Versions that failed a session, which kelpie does not try again
    #[serde(default)]
    pub bad: Vec<Rejected>,
}

/// Why a bridge's versions could not be read or written
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum VersionsError {
    /// The bridge's id or version is not one folder or file name
    Name {
        /// The text that names a path rather than a name
        name: String,
    },

    /// A version is not a semantic version
    NotAVersion {
        /// The text kelpie could not read as one
        text: String,
    },

    /// The file exists but is not what [`Versions`] writes
    Malformed {
        /// The file
        file: PathBuf,
        /// What the parser said
        message: String,
    },

    /// The OS refused a read of the file
    Read {
        /// The file
        file: PathBuf,
        /// What the OS said
        source: io::Error,
    },

    /// The OS refused a write while saving the file
    Write {
        /// The file, or the temporary file beside it
        file: PathBuf,
        /// What the OS said
        source: io::Error,
    },
}

impl fmt::Display for VersionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name { name } => write!(f, "{name:?} is not one folder or file name"),
            Self::NotAVersion { text } => write!(f, "{text:?} is not a semantic version"),
            Self::Malformed { file, message } => {
                write!(f, "{} is not a versions file: {message}", file.display())
            }
            Self::Read { file, source } => write!(f, "cannot read {}: {source}", file.display()),
            Self::Write { file, source } => write!(f, "cannot write {}: {source}", file.display()),
        }
    }
}

impl core::error::Error for VersionsError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Write { source, .. } => Some(source),
            Self::Name { .. } | Self::NotAVersion { .. } | Self::Malformed { .. } => None,
        }
    }
}

/// Read `text` as a version
///
/// # Errors
///
/// [`VersionsError::NotAVersion`] when it is not a semantic version.
pub(crate) fn parse(text: &str) -> Result<Version, VersionsError> {
    Version::parse(text).map_err(|_| VersionsError::NotAVersion {
        text: text.to_owned(),
    })
}

impl Versions {
    /// The versions of `pin`'s bridge, or kelpie's pin in use when none are written yet
    ///
    /// # Errors
    ///
    /// [`VersionsError`] when a name in the pin is not one folder name, the pin's
    /// version is not a semantic version, or the file cannot be read or is not
    /// a versions file.
    pub fn load(data_dir: &Path, pin: &Pin<'_>) -> Result<Self, VersionsError> {
        let file = file(data_dir, pin.id)?;
        match fs::read_to_string(&file) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| VersionsError::Malformed {
                file,
                message: error.to_string(),
            }),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(Self {
                in_use: parse(pin.version)?,
                previous: None,
                ready: None,
                held_back: Vec::new(),
                bad: Vec::new(),
            }),
            Err(source) => Err(VersionsError::Read { file, source }),
        }
    }

    /// Write the versions of the bridge `id` through a temporary file and a rename
    ///
    /// A reader sees the file as it was or as it is now, never half of it.
    ///
    /// # Errors
    ///
    /// [`VersionsError::Name`] when `id` is not one folder name, and
    /// [`VersionsError::Write`] when the OS refuses the folder, the temporary
    /// file or the rename.
    pub fn save(&self, data_dir: &Path, id: &str) -> Result<(), VersionsError> {
        let target = file(data_dir, id)?;
        let write = |file: &Path, source: io::Error| VersionsError::Write {
            file: file.to_path_buf(),
            source,
        };
        let dir = target.parent().unwrap_or(data_dir);
        fs::create_dir_all(dir).map_err(|source| write(dir, source))?;
        let temp = dir.join(format!(
            "{FILE}.{}.tmp",
            token().map_err(|source| write(&target, source))?
        ));
        let mut text = serde_json::to_string_pretty(self)
            .map_err(|error| write(&temp, io::Error::other(error)))?;
        text.push('\n');
        let written = File::create(&temp).and_then(|mut file| {
            file.write_all(text.as_bytes())?;
            file.sync_all()
        });
        if let Err(source) = written {
            let _ = fs::remove_file(&temp);
            return Err(write(&temp, source));
        }
        fs::rename(&temp, &target).map_err(|source| {
            let _ = fs::remove_file(&temp);
            write(&target, source)
        })
    }

    /// Whether `version` failed a session and is not tried again
    #[must_use]
    pub fn is_bad(&self, version: &Version) -> bool {
        self.bad.iter().any(|rejected| &rejected.version == version)
    }
}

/// Where the bridge `id` keeps its versions
fn file(data_dir: &Path, id: &str) -> Result<PathBuf, VersionsError> {
    if !is_name(id) {
        return Err(VersionsError::Name {
            name: id.to_owned(),
        });
    }
    Ok(data_dir.join(BRIDGES_FOLDER).join(id).join(FILE))
}
