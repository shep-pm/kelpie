//! The run file: which groups a run started, for the next launch to stop

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write as _};
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
    /// The sibling cannot be written, synced or renamed.
    pub fn write(&self, path: &Path) -> io::Result<()> {
        let mut sibling = OsString::from(path);
        sibling.push(".tmp");
        let mut file = fs::File::create(&sibling)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        // Without these a power cut can leave `path` empty after the rename
        file.sync_all()?;
        fs::rename(&sibling, path)?;
        sync_folder(path)
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

/// Make a rename in `path`'s folder durable; a folder cannot be synced on Windows
fn sync_folder(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    if let Some(folder) = path.parent() {
        let folder = if folder.as_os_str().is_empty() {
            Path::new(".")
        } else {
            folder
        };
        fs::File::open(folder)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
