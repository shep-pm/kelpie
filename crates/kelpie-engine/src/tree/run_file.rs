//! The run file: which groups a run started, for the next launch to stop

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// What a run records about itself on disk
// wire format: the next launch reads what the last one wrote
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFile {
    /// The run's id, as `RunId` shows it
    pub run: String,
    /// The process groups that were alive at the last rewrite
    pub groups: Vec<Group>,
}

/// One process group, named by its leader
// wire format: the next launch reads what the last one wrote
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The leader's pid, which is the group's id
    pub pid: u32,
    /// When the leader started, as the OS reports it, so a reused pid differs
    pub started: u64,
}

impl RunFile {
    /// Replace the file at `path` with this one, atomically
    ///
    /// The new text goes to a sibling first and is renamed over `path`, so a
    /// reader sees the old file or the new one, never half of either.
    ///
    /// # Errors
    ///
    /// The sibling cannot be written or renamed.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let mut sibling = OsString::from(path);
        sibling.push(".tmp");
        fs::write(&sibling, serde_json::to_vec(self)?)?;
        fs::rename(&sibling, path)
    }

    /// Read the file at `path`
    ///
    /// # Errors
    ///
    /// The file cannot be read, or is not a run file.
    pub fn read(path: &Path) -> io::Result<RunFile> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }
}
