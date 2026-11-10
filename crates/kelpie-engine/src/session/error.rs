//! Why kelpie could not learn what a harness offers

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

/// Why kelpie could not learn what a harness offers
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug)]
#[non_exhaustive]
pub enum SessionError {
    /// The harness process could not be started
    Start(io::Error),
    /// The harness ended the connection before it listed its options, with what it left behind
    Ended {
        /// The harness's own end, as the tree and its stderr can say it
        message: String,
    },
    /// The harness did not answer within the time kelpie waits, and was stopped
    TimedOut {
        /// How long kelpie waited
        after: Duration,
        /// How the harness ended once it was stopped, with what it last wrote
        message: String,
    },
    /// The harness answered with an error instead of its options
    Refused {
        /// The error the harness answered with
        message: String,
    },
    /// The harness id is not one file name, so no answer can be kept under it
    Name {
        /// The harness id kelpie was asked about
        name: String,
    },
    /// The answer could not be written to the data folder
    Cache {
        /// The file kelpie could not write
        path: PathBuf,
        /// What the OS said
        source: io::Error,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(error) => write!(f, "kelpie could not start the harness: {error}"),
            Self::Ended { message } => {
                write!(
                    f,
                    "the harness ended the connection before it answered: {message}"
                )
            }
            Self::TimedOut { after, message } => {
                write!(
                    f,
                    "the harness did not answer within {} s and was stopped",
                    after.as_secs_f32().ceil()
                )?;
                if message.is_empty() {
                    Ok(())
                } else {
                    write!(f, ": {message}")
                }
            }
            Self::Refused { message } => {
                write!(f, "the harness would not list its options: {message}")
            }
            Self::Name { name } => {
                write!(
                    f,
                    "{name} is not a name kelpie can keep a harness's answers under"
                )
            }
            Self::Cache { path, source } => {
                write!(f, "kelpie could not read or write {path:?}: {source}")
            }
        }
    }
}

impl core::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Start(error) | Self::Cache { source: error, .. } => Some(error),
            Self::Ended { .. }
            | Self::TimedOut { .. }
            | Self::Refused { .. }
            | Self::Name { .. } => None,
        }
    }
}
