//! Kelpie's GitHub App, one per repo owner, and the tokens it mints
//!
//! A worker pushes with a token for its one repo, never the maintainer's login
//! (`docs/design/workers.md`, Pushing and the pull request). [`Apps`] keeps each
//! App kelpie registered, [`AppTokens`] mints its tokens, and [`curl`] runs the
//! calls with the system's `curl` and signs with the system's `openssl`, both
//! through the process tree (ADR 0001).
//!
//! Registering an App is the GitHub slice's own work: until it lands, kelpie
//! reads what is on disk and a worker with no App is told it cannot push.
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::Path;
//! use std::sync::Arc;
//!
//! use kelpie_engine::github::{GithubApp, forge_of, system_tokens};
//! use kelpie_engine::run::RunId;
//! use kelpie_engine::tree::ProcessTree;
//!
//! let tree = Arc::new(ProcessTree::new(RunId::new()?, Path::new("run.json").to_path_buf())?);
//! let tokens = system_tokens(&tree, Path::new("/data/kelpie"));
//! let repo = forge_of("git@github.com:shep-pm/kelpie.git").expect("a GitHub remote");
//! let worker = tokens.worker(&repo)?;
//! assert!(!worker.workflows, "the installation says whether a worker may change a workflow file");
//! # Ok::<(), Box<dyn core::error::Error>>(())
//! ```

use std::fmt;
use std::fs::{self, Metadata};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::tree::ProcessTree;

pub mod api;
pub mod curl;
pub mod slug;
pub mod tokens;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

pub use api::{
    ApiError, Cooldown, GithubApi, Grant, Installation, IssuedToken, Pem, Signer, token_request,
};
pub use curl::{CurlGithub, Openssl};
pub use slug::{ForgeSlug, forge_of};
pub use tokens::{AppTokens, GithubApp, InstallationToken, Jwt, TokenError, WorkerToken};

/// The folder in kelpie's data folder the Apps are kept in
pub const FOLDER: &str = "github";

const RECORD: &str = "app.json";
const KEY: &str = "key.pem";

/// A moment, as whole seconds since the Unix epoch
///
/// GitHub's answers carry whole seconds, and a token's life is measured in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Timestamp(u64);

impl Timestamp {
    /// The moment as whole seconds since the Unix epoch
    #[must_use]
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs)
    }

    /// The seconds themselves
    #[must_use]
    pub const fn secs(self) -> u64 {
        self.0
    }
}

/// Where a mint reads the present moment from
pub trait Clock: Send + Sync {
    /// The present moment
    fn now(&self) -> Timestamp;
}

/// The OS clock
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Timestamp::from_secs(since.as_secs())
    }
}

/// kelpie's App tokens, asked of GitHub with `curl` and signed with `openssl`,
/// both through `tree`, for the Apps in `kelpie_home`
#[must_use]
pub fn system_tokens(tree: &Arc<ProcessTree>, kelpie_home: &Path) -> AppTokens {
    AppTokens::new(
        Apps::under(kelpie_home),
        Box::new(CurlGithub::new(tree)),
        Box::new(Openssl::new(tree)),
        Box::new(SystemClock),
    )
}

/// A repo owner on GitHub, a user or an organization, in lower case
///
/// GitHub reads a login in any case, so an owner's folder is named in one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Owner(String);

impl Owner {
    /// The owner of `repo`
    ///
    /// # Errors
    ///
    /// Why the owner is not a GitHub login, as [`Owner::try_from`] says.
    pub fn of(repo: &ForgeSlug) -> Result<Self, String> {
        Self::try_from(repo.owner())
    }

    /// The login, in lower case
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for Owner {
    type Error = String;

    fn try_from(login: &str) -> Result<Self, Self::Error> {
        let allowed = |c: char| c.is_ascii_alphanumeric() || c == '-';
        if login.is_empty()
            || login.len() > 39
            || login.starts_with('-')
            || !login.chars().all(allowed)
        {
            return Err(format!("{login:?} is not a GitHub login"));
        }
        Ok(Self(login.to_ascii_lowercase()))
    }
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An App kelpie registered, as [`Apps`] keeps it
// wire format: changing this is a breaking change to `app.json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct App {
    /// The App's id
    pub id: u64,
    /// The App's slug, which its pages and its bot's login are named for
    pub slug: String,
    /// The App's client id, which its tokens are asked for as
    pub client_id: String,
    /// The account that owns the App, the only one it installs on
    pub owner: String,
}

impl App {
    /// The page that installs the App on an owner's repos
    #[must_use]
    pub fn install_url(&self) -> String {
        format!("https://github.com/apps/{}/installations/new", self.slug)
    }
}

/// Why an App's files could not be read or written, naming the file and never
/// the key in it
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// The key or a folder above it is a link, someone else's, or open to
    /// others, so it may have been read or replaced
    Exposed(PathBuf),
    /// A file could not be read or written, or is not what kelpie wrote, with why
    Unusable(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exposed(path) => write!(
                f,
                "{} may be used by others: it must be yours, not a link, and {}; register the \
                 App again if someone else may have read it",
                path.display(),
                narrow(path)
            ),
            Self::Unusable(why) => f.write_str(why),
        }
    }
}

impl core::error::Error for StoreError {}

// What the message tells the maintainer to run to keep a folder or the key to its owner
#[cfg(unix)]
fn narrow(_path: &Path) -> String {
    "`chmod 700` for a folder or `chmod 600` for the key".to_owned()
}

#[cfg(windows)]
fn narrow(path: &Path) -> String {
    let path = path.display();
    format!(
        "`icacls {path} /inheritance:r /grant:r %USERNAME%:(OI)(CI)F` for a folder or \
         `icacls {path} /inheritance:r /grant:r %USERNAME%:F` for the key"
    )
}

fn unusable(path: &Path, e: &io::Error) -> StoreError {
    StoreError::Unusable(format!("cannot use {}: {}", path.display(), e.kind()))
}

// An App file that is not a regular file, which a read could wait on forever.
fn not_a_file(path: &Path) -> StoreError {
    StoreError::Unusable(format!(
        "{} is not a regular file: register the App again to write it",
        path.display()
    ))
}

/// kelpie's Apps, one folder per owner under kelpie's data folder
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apps {
    folder: PathBuf,
}

impl Apps {
    /// The Apps kept under `kelpie_home`
    #[must_use]
    pub fn under(kelpie_home: &Path) -> Self {
        Self {
            folder: kelpie_home.join(FOLDER),
        }
    }

    /// The App for `owner`, or `None` when kelpie has none
    ///
    /// # Errors
    ///
    /// [`StoreError`] when its files cannot be read, or the key or a folder
    /// above it is exposed.
    pub fn get(&self, owner: &Owner) -> Result<Option<App>, StoreError> {
        let record = self.record(owner);
        match fs::symlink_metadata(&record) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(unusable(&record, &e)),
            Ok(meta) if !meta.is_file() => return Err(not_a_file(&record)),
            Ok(_) => {}
        }
        for folder in [&self.folder, &self.owned(owner)] {
            private(folder)?;
        }
        let key = self.key(owner);
        private(&key)?;
        if !fs::symlink_metadata(&key).is_ok_and(|meta| meta.is_file()) {
            return Err(not_a_file(&key));
        }
        let text = fs::read_to_string(&record).map_err(|e| unusable(&record, &e))?;
        serde_json::from_str(&text).map(Some).map_err(|_| {
            StoreError::Unusable(format!(
                "{} is not an App kelpie wrote: register the App again to write it",
                record.display()
            ))
        })
    }

    /// Keeps `app` and its `pem`, in place of any App its owner had
    ///
    /// The folders are made the maintainer's alone first, and the key goes in
    /// before the record, so an App on record always has its key.
    ///
    /// # Errors
    ///
    /// [`StoreError`] when the owner is not a login, a folder is someone else's
    /// or a link, or a file cannot be written.
    pub fn save(&self, app: &App, pem: &Pem) -> Result<(), StoreError> {
        let owner = Owner::try_from(app.owner.as_str()).map_err(StoreError::Unusable)?;
        let owned = self.owned(&owner);
        fs::create_dir_all(&owned).map_err(|e| unusable(&owned, &e))?;
        for folder in [&self.folder, &owned] {
            make_private(folder)?;
        }
        let record = serde_json::to_string_pretty(app).expect("an App serializes");
        put(&self.key(&owner), pem.expose().as_bytes())?;
        put(&self.record(&owner), record.as_bytes())?;
        Ok(())
    }

    /// The private key's file for `owner`, which only a signer reads
    #[must_use]
    pub fn key(&self, owner: &Owner) -> PathBuf {
        self.owned(owner).join(KEY)
    }

    fn owned(&self, owner: &Owner) -> PathBuf {
        self.folder.join(owner.as_str())
    }

    fn record(&self, owner: &Owner) -> PathBuf {
        self.owned(owner).join(RECORD)
    }
}

// Makes `folder` the maintainer's alone, refusing a link or someone else's.
fn make_private(folder: &Path) -> Result<(), StoreError> {
    let meta = fs::symlink_metadata(folder).map_err(|e| unusable(folder, &e))?;
    if meta.file_type().is_symlink() || !owned_by_maintainer(&meta) {
        return Err(StoreError::Exposed(folder.to_owned()));
    }
    restrict(folder).map_err(|e| unusable(folder, &e))?;
    Ok(())
}

// Refuses `path` when it is a link, someone else's, or open to others.
fn private(path: &Path) -> Result<(), StoreError> {
    let meta = fs::symlink_metadata(path).map_err(|e| unusable(path, &e))?;
    if meta.file_type().is_symlink() || !alone(&meta) {
        return Err(StoreError::Exposed(path.to_owned()));
    }
    Ok(())
}

// Writes `bytes` beside `path`, readable by its owner alone, syncs it and
// renames it over `path`.
fn put(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let fresh = path.with_extension(format!("new.{}", std::process::id()));
    let _ = fs::remove_file(&fresh);
    let written = create_private(&fresh)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()))
        .and_then(|()| fs::rename(&fresh, path));
    let _ = fs::remove_file(&fresh);
    written.map_err(|e| unusable(path, &e))
}

fn create_private(path: &Path) -> io::Result<fs::File> {
    let file = fs::File::create(path)?;
    restrict_mode(&file)?;
    Ok(file)
}

#[cfg(unix)]
fn restrict_mode(file: &fs::File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_mode(_file: &fs::File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict(folder: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(folder, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict(_folder: &Path) -> io::Result<()> {
    Ok(())
}

// Whether `meta`'s mode gives the file to no one but its owner. Windows has no
// such mode, so a file there is judged by its owner alone, as `worker_folder`
// judges it.
#[cfg(unix)]
fn alone(meta: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    owned_by_maintainer(meta) && meta.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn alone(meta: &Metadata) -> bool {
    owned_by_maintainer(meta)
}

#[cfg(unix)]
fn owned_by_maintainer(meta: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.uid() == rustix::process::geteuid().as_raw()
}

#[cfg(not(unix))]
fn owned_by_maintainer(_meta: &Metadata) -> bool {
    true
}
