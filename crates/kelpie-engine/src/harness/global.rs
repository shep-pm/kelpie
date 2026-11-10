//! The person's own global folder, linked into a session's config folder
//!
//! Each harness keeps a global folder of its own (`~/.claude`, goose's config
//! root), and each session gets a config folder of kelpie's own (#23, #24).
//! [`link`] is how the first reaches the second once the project turns on
//! *Harness global folder* (ADR 0006): every entry the harness names is linked
//! into the session's folder, and every link into the person's folder that is no
//! longer wanted is removed, so turning the setting off takes the links back.
//!
//! A link and not a copy: the person edits one file and the next session reads
//! it. Writes stay the session's state folder (ADR 0005), so a harness cannot
//! change a linked file through it. A login or a secret is never an entry, and
//! the sandbox keeps those closed whatever the setting says.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One file or folder of the person's global folder that a harness reads
///
/// A name inside the folder, which may sit in a folder of its own, as goose's
/// `config/config.yaml` does. A login or a secret is never one.
pub type Entry = &'static str;

/// Why a session's config folder could not be linked to the person's global folder
///
/// No variant holds a path outside the person's own folders, and none holds a
/// file's contents.
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GlobalError {
    /// A folder the link needed could not be made
    Folder {
        /// The folder the OS refused
        path: PathBuf,

        /// What the OS said, without any file's contents
        reason: String,
    },

    /// A link could not be made, read or removed
    Link {
        /// The link the OS refused
        path: PathBuf,

        /// What the OS said, without any file's contents
        reason: String,
    },
}

impl fmt::Display for GlobalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Folder { path, reason } => write!(
                f,
                "the session's config folder {} could not be made: {reason}",
                path.display()
            ),
            Self::Link { path, reason } => write!(
                f,
                "the link {} could not be made or removed: {reason}",
                path.display()
            ),
        }
    }
}

impl core::error::Error for GlobalError {}

/// Link each entry of `global` that is there into the session's config folder `session`
///
/// Each link is named for its entry, so the harness finds the person's own
/// `skills` where it looks for its own. An entry the person does not have is
/// skipped, and a name `session` already holds as something other than a link
/// stands: kelpie's own settings for the session come first. Every link under
/// `session` that points into `global` and is not one of `entries` is removed,
/// which is how turning the setting off takes back what a session linked with it
/// on.
///
/// The paths it linked, which are the ones the sandbox has to open for the
/// harness to read them.
///
/// # Errors
///
/// [`GlobalError`] when a folder cannot be made, or a link cannot be made, read
/// or removed.
pub fn link(global: &Path, session: &Path, entries: &[Entry]) -> Result<Vec<PathBuf>, GlobalError> {
    let wanted: Vec<Entry> = entries
        .iter()
        .copied()
        .filter(|name| global.join(name).exists())
        .collect();
    unhook(global, session, &wanted)?;
    let mut linked = Vec::new();
    for name in wanted {
        hook(global, session, name)?;
        linked.push(global.join(name));
    }
    Ok(linked)
}

/// Remove every link under `session` that points into `global` and is not wanted
///
/// A folder that is not there holds no links, and reading one that is there is
/// left to the harness: only the links kelpie made are touched.
fn unhook(global: &Path, session: &Path, wanted: &[Entry]) -> Result<(), GlobalError> {
    let mut folders = vec![session.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let read = match fs::read_dir(&folder) {
            Ok(read) => read,
            Err(source) if source.kind() == io::ErrorKind::NotFound => continue,
            Err(source) => return Err(refused(&folder, &source)),
        };
        for entry in read {
            let entry = entry.map_err(|source| refused(&folder, &source))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|source| refused(&path, &source))?;
            if kind.is_dir() {
                folders.push(path);
                continue;
            }
            if !kind.is_symlink() || entry_of(session, &path, wanted).is_some() {
                continue;
            }
            if points_at(&path, global) {
                fs::remove_file(&path).map_err(|source| refused(&path, &source))?;
            }
        }
    }
    Ok(())
}

/// Make the link for one entry, refreshing one that is already a link
fn hook(global: &Path, session: &Path, name: Entry) -> Result<(), GlobalError> {
    let (from, to) = (global.join(name), session.join(name));
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|source| refused_folder(parent, &source))?;
    }
    match fs::symlink_metadata(&to) {
        Ok(held) if held.is_symlink() => {
            fs::remove_file(&to).map_err(|source| refused(&to, &source))?
        }
        Ok(_) => return Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(refused(&to, &source)),
    }
    link_to(&from, &to, from.is_dir()).map_err(|source| refused(&to, &source))
}

/// What the OS said about `path`, as a link refusal
fn refused(path: &Path, source: &io::Error) -> GlobalError {
    GlobalError::Link {
        path: path.to_path_buf(),
        reason: source.to_string(),
    }
}

/// What the OS said about `path`, as a folder refusal
fn refused_folder(path: &Path, source: &io::Error) -> GlobalError {
    GlobalError::Folder {
        path: path.to_path_buf(),
        reason: source.to_string(),
    }
}

/// The entry `link` was made for, when it sits where that entry's link sits
fn entry_of(session: &Path, link: &Path, wanted: &[Entry]) -> Option<Entry> {
    let name = link.strip_prefix(session).ok()?;
    let name = name.to_string_lossy();
    wanted.iter().copied().find(|entry| *entry == name)
}

/// Whether `link` points at `global` or at something under it
fn points_at(link: &Path, global: &Path) -> bool {
    let Ok(target) = fs::read_link(link) else {
        return false;
    };
    let target = match link.parent() {
        Some(parent) if !target.is_absolute() => parent.join(target),
        _ => target,
    };
    under(&target, global)
}

/// Whether `path` is `folder` or beneath it, once each is read as an absolute path
fn under(path: &Path, folder: &Path) -> bool {
    let absolute = |path: &Path| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let (path, folder) = (absolute(path), absolute(folder));
    path.starts_with(&folder)
}

/// A link at `to` standing for `from`, which is a folder when `folder` says so
#[cfg(unix)]
fn link_to(from: &Path, to: &Path, folder: bool) -> io::Result<()> {
    let _ = folder;
    std::os::unix::fs::symlink(from, to)
}

/// Windows needs to be told which kind of link to make, and to hold the privilege
#[cfg(windows)]
fn link_to(from: &Path, to: &Path, folder: bool) -> io::Result<()> {
    if folder {
        std::os::windows::fs::symlink_dir(from, to)
    } else {
        std::os::windows::fs::symlink_file(from, to)
    }
}

/// The person's home folder, as the OS names it
pub(super) fn home() -> Option<PathBuf> {
    let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(home)
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}
