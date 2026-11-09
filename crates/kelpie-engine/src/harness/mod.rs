//! Harnesses: what each one needs before a session can run on it
//!
//! A [`crate::session::Harness`] says what to run, and each harness's slice builds one
//! from what that harness asks for (ADR 0007): its bridge from the ACP registry,
//! the folder the session keeps its state in, and the person's own login, read
//! from the [`login`](crate::login) store.
//! Each also says what a sandbox needs to know of it (ADR 0005): the hosts its
//! model answers on, the folders its install is read from, and the mode that
//! turns its own permission prompts off.
//! Nothing here starts a process: [`session`](crate::session) does that through
//! the process tree (ADR 0001).
//!
//! Kelpie ships no harness and no login of its own (ADR 0006). Each slice points
//! the harness at the person's own install and gives the session a config folder
//! of kelpie's own, which keeps the person's global folder out until they turn
//! it on. A harness that speaks ACP itself, such as goose, is run directly; one
//! that does not is run through its bridge from the registry.
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::Path;
//!
//! use kelpie_engine::harness::{claude, goose};
//! use kelpie_engine::login::Store;
//!
//! let harness = claude::harness("claude-agent-acp".into(), Path::new("pm"), &Store::keychain())?;
//! println!("run {}", harness.program.display());
//!
//! let ollama = goose::Provider {
//!     name: "ollama".to_owned(),
//!     host: None,
//! };
//! let local = goose::harness(Path::new("pm"), ollama)?;
//! let acp = local.args.first().expect("goose is run with its subcommand");
//! println!("or run {} {}", local.program.display(), acp.to_string_lossy());
//! # Ok::<(), kelpie_engine::harness::HarnessError>(())
//! ```

use std::env::{self, consts::EXE_SUFFIX};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::login::LoginError;

pub mod claude;
pub mod goose;

/// Why a harness could not be built
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HarnessError {
    /// The harness's own command is not on `PATH`, so the person has not installed it
    NotInstalled {
        /// The harness kelpie cannot run, as it names it to the person
        harness: String,

        /// The command kelpie looked for on `PATH`
        cmd: String,
    },

    /// The login store holds no login for this harness
    NoLogin {
        /// The harness that has no login
        harness: String,

        /// The command that makes a token
        setup: String,
    },

    /// The login store could not be read, so whether the harness has a login is unknown
    Login(LoginError),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInstalled { harness, cmd } => write!(
                f,
                "{harness} is not installed: kelpie found no {cmd} on PATH. \
                 Install {harness} and start kelpie again."
            ),
            Self::NoLogin { harness, setup } => write!(
                f,
                "{harness} has no login: each session keeps a config folder of its own, \
                 which holds none. Run `{setup}`, then paste the token it prints into \
                 {harness}'s login on kelpie's settings page."
            ),
            Self::Login(error) => write!(f, "kelpie could not read the login: {error}"),
        }
    }
}

impl From<LoginError> for HarnessError {
    fn from(error: LoginError) -> Self {
        Self::Login(error)
    }
}

impl core::error::Error for HarnessError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Login(error) => Some(error),
            Self::NotInstalled { .. } | Self::NoLogin { .. } => None,
        }
    }
}

/// The first `cmd` on the `PATH` text `path`, as an absolute path
///
/// Each harness runs the person's own install, found here rather than trusted to
/// whatever `PATH` the harness would search itself.
pub(super) fn on_path(cmd: &str, path: &OsStr) -> Option<PathBuf> {
    let name = format!("{cmd}{EXE_SUFFIX}");
    env::split_paths(path).find_map(|dir| {
        let found = dir.join(&name);
        is_program(&found).then(|| std::path::absolute(&found).unwrap_or(found))
    })
}

/// The folders a program is read from: the one it sits in, and the one its link
/// leads to when it is a link
///
/// A sandbox that closes the home folder has to leave these open for the
/// harness to start.
pub(super) fn install_folders(program: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = Vec::new();
    let real = fs::canonicalize(program).ok();
    for path in [Some(program), real.as_deref()].into_iter().flatten() {
        if let Some(folder) = path.parent()
            && !folders.iter().any(|known| known == folder)
        {
            folders.push(folder.to_owned());
        }
    }
    folders
}

/// Whether `path` is a file this OS can run
fn is_program(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && runnable(&metadata)
}

/// Whether the OS says a file may be run
#[cfg(unix)]
fn runnable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o111 != 0
}

/// Windows runs a file by its extension, which `name` already carries
#[cfg(not(unix))]
fn runnable(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests;
