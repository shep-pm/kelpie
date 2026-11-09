//! The error every bridge install path returns
//!
//! Each variant names what broke, and the constructors below are where `super`
//! builds them. The three carrying an [`io::Error`] leave the enum without
//! `PartialEq`, so a caller matches the variant and reads what it names.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use super::Download;

/// Why a bridge did not install
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum BridgeError {
    /// The pin has no archive for the platform this process runs on
    NoDownload {
        /// The bridge kelpie wanted
        id: String,
        /// The version it wanted
        version: String,
        /// The platform it is on, as the OS names it
        platform: String,
    },

    /// The archive could not be fetched, or the server answered an error status
    Download {
        /// The url kelpie fetched
        url: String,
        /// What the client said
        source: io::Error,
    },

    /// The downloaded archive hashes to `actual`, not the pin's `expected`
    ChecksumMismatch {
        /// The url it came from
        url: String,
        /// The sha256 the pin holds, as hex
        expected: String,
        /// The sha256 the download hashed to, as hex
        actual: String,
    },

    /// The url names an archive format kelpie does not unpack, which is only `.tar.gz` and `.zip`
    ArchiveFormat {
        /// The url kelpie could not read as an archive
        url: String,
    },

    /// The archive holds no regular file named `cmd`
    MissingCommand {
        /// The url it came from
        url: String,
        /// The command the pin names
        cmd: String,
    },

    /// `name` is not one folder or file name, so the pin cannot name a path under `<data_dir>/bridges`
    PinNotUsable {
        /// The id, version or cmd that is not usable
        name: String,
    },

    /// The file could not be read
    Read {
        /// The file or folder
        path: PathBuf,
        /// What the OS said
        source: io::Error,
    },

    /// The file, or the folder above it, could not be written
    Write {
        /// The file or folder
        path: PathBuf,
        /// What the OS said
        source: io::Error,
    },
}

/// The read side of [`BridgeError`]
pub(super) fn could_not_read(path: &Path, source: io::Error) -> BridgeError {
    BridgeError::Read {
        path: path.to_path_buf(),
        source,
    }
}

/// The write side of [`BridgeError`]
pub(super) fn could_not_write(path: &Path, source: io::Error) -> BridgeError {
    BridgeError::Write {
        path: path.to_path_buf(),
        source,
    }
}

/// The download side of [`BridgeError`]
pub(super) fn could_not_download(url: &str, source: io::Error) -> BridgeError {
    BridgeError::Download {
        url: url.to_string(),
        source,
    }
}

/// The missing-command side of [`BridgeError`]
pub(super) fn missing_command(download: &Download) -> BridgeError {
    BridgeError::MissingCommand {
        url: download.url.clone(),
        cmd: download.cmd.clone(),
    }
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoDownload {
                id,
                version,
                platform,
            } => {
                write!(f, "the bridge {id} {version} has no archive for {platform}")
            }
            Self::Download { url, source } => write!(f, "cannot download {url}: {source}"),
            Self::ChecksumMismatch {
                url,
                expected,
                actual,
            } => write!(
                f,
                "{url} is not the archive the pin names: expected sha256 {expected}, got {actual}"
            ),
            Self::ArchiveFormat { url } => {
                write!(
                    f,
                    "{url} is not an archive kelpie unpacks, which is .tar.gz or .zip"
                )
            }
            Self::MissingCommand { url, cmd } => {
                write!(f, "{url} holds no {cmd} to install")
            }
            Self::PinNotUsable { name } => {
                write!(
                    f,
                    "{name:?} is not one folder or file name for a bridge path"
                )
            }
            Self::Read { path, source } => write!(f, "cannot read {path:?}: {source}"),
            Self::Write { path, source } => write!(f, "cannot write {path:?}: {source}"),
        }
    }
}

impl core::error::Error for BridgeError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Download { source, .. }
            | Self::Read { source, .. }
            | Self::Write { source, .. } => Some(source),
            _ => None,
        }
    }
}
