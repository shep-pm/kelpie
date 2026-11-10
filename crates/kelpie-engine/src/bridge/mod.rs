//! Bridge: the installer every bridge shares
//!
//! A harness that does not speak ACP itself runs through its bridge from the ACP
//! registry (ADR 0007). [`ensure`] is the installer every bridge shares: it
//! downloads the registry's own prebuilt archive over HTTPS, checks its sha256
//! before unpacking anything, and unpacks only the command kelpie runs. Kelpie
//! needs neither Node nor Bun, and no downloader enters the process tree,
//! because the fetch is an HTTP client in the engine (ADR 0001).
//!
//! A [`Pin`] is one bridge at one version, copied from the bridge's `agent.json`
//! in `agentclientprotocol/registry`. Pins are kelpie's own code. [`check`] finds a
//! newer version in the registry ([`registry`]), installs it with this same
//! [`ensure`], probes it ([`probe()`]) and records it in [`Versions`]
//! (`docs/design/bridge-updates.md`). A session on a [`crate::session::Bridge`]
//! takes a `ready` version at its next idle point.
//!
//! The install lands at `<data_dir>/bridges/<id>/<version>/<cmd>`, with the
//! command's own sha256 recorded beside it in `<cmd>.sha256`. A later call
//! rehashes the file and reuses it when the sum matches, so each pinned version
//! is downloaded once.
//!
//! # Quick start
//!
//! ```no_run
//! use kelpie_engine::bridge::{Download, Pin, Platform, ensure};
//! use std::path::Path;
//!
//! let pin = Pin {
//!     id: "claude-acp",
//!     registry_id: "claude-acp",
//!     version: "0.101.1",
//!     platforms: &[(
//!         Platform::MacOsArm64,
//!         Download {
//!             url: "https://github.com/agentclientprotocol/claude-agent-acp/releases/download/v0.101.1/claude-code-acp-darwin-arm64.zip".into(),
//!             sha256: "0000000000000000000000000000000000000000000000000000000000000000".into(),
//!             cmd: "claude-code-acp".into(),
//!         },
//!     )],
//! };
//! let cmd = ensure(Path::new(".kelpie-data"), &pin)?;
//! println!("run {}", cmd.display());
//! # Ok::<(), kelpie_engine::bridge::BridgeError>(())
//! ```

use std::env::consts::{ARCH, OS};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use sha2::{Digest as _, Sha256};

mod check;
mod error;
mod probe;
pub mod registry;
pub mod versions;

pub use check::{CheckOutcome, check};
pub use probe::{Capability, ProbeError, probe};
pub use versions::{Rejected, Versions, VersionsError};

pub use error::BridgeError;
use error::{could_not_download, could_not_read, could_not_write, missing_command};

/// The folder under the data dir that holds every installed bridge
const BRIDGES_FOLDER: &str = "bridges";

/// The file inside one version's folder that holds the downloaded archive
const ARCHIVE_NAME: &str = "archive";

/// The suffix of the file beside an installed bridge that records its sha256
const RECORD_SUFFIX: &str = ".sha256";

/// The suffix of the file that serialises installs of one version
const LOCK_SUFFIX: &str = ".lock";

/// The prefix of the folder an in-progress install is unpacked into
const TEMP_PREFIX: &str = ".install-";

/// How long one bridge download may take end to end
///
/// A bridge archive is one prebuilt executable, not a package tree, so it lands
/// in seconds on a home line. 300 s leaves room for a slow one.
const TIMEOUT: Duration = Duration::from_secs(300);

/// The most bytes kelpie reads from one bridge archive download
///
/// 512 MiB, well above one prebuilt executable, so a server that answers wrong
/// cannot fill the disk.
const LIMIT: u64 = 512 * 1024 * 1024;

/// The platform one bridge archive is built for
///
/// Kelpie's own names. The registry's platform strings are mapped onto these
/// where a pin is written out, not inside this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Linux on x86_64
    LinuxX64,

    /// Linux on arm64
    LinuxArm64,

    /// macOS on x86_64
    MacOsX64,

    /// macOS on arm64
    MacOsArm64,

    /// Windows on x86_64
    WindowsX64,
}

impl Platform {
    /// The platform this process runs on, or `None` when kelpie has no name for it
    #[must_use]
    pub fn current() -> Option<Platform> {
        match platform_text().as_str() {
            "linux-x86_64" => Some(Self::LinuxX64),
            "linux-aarch64" => Some(Self::LinuxArm64),
            "macos-x86_64" => Some(Self::MacOsX64),
            "macos-aarch64" => Some(Self::MacOsArm64),
            "windows-x86_64" => Some(Self::WindowsX64),
            _ => None,
        }
    }
}

/// This process's platform, as the OS names it: `<os>-<arch>`
fn platform_text() -> String {
    format!("{OS}-{ARCH}")
}

/// One bridge archive kelpie downloads for one platform
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    /// Where the archive is. Its extension says which format kelpie unpacks
    pub url: String,

    /// The sha256 the registry publishes for the archive, as 64 hex digits
    pub sha256: String,

    /// The file inside the archive kelpie installs, which is the one it runs
    pub cmd: String,
}

/// One bridge kelpie installs: a registry id, one pinned version, and the
/// archive for each platform kelpie ships on
///
/// Written out in kelpie's code from the bridge's `agent.json` in
/// `agentclientprotocol/registry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pin<'a> {
    /// The registry id, such as `claude-acp`
    pub id: &'a str,

    /// The id the registry's index lists this bridge under, which [`registry::Index::bridge`] looks up
    pub registry_id: &'a str,

    /// The version pinned, such as `0.101.1`
    pub version: &'a str,

    /// One entry per platform kelpie ships on, each with its archive
    pub platforms: &'a [(Platform, Download)],
}

impl Pin<'_> {
    /// The archive for the platform this process runs on
    fn download(&self) -> Result<&Download, BridgeError> {
        let current = Platform::current();
        self.platforms
            .iter()
            .find_map(|(platform, download)| (Some(*platform) == current).then_some(download))
            .ok_or_else(|| BridgeError::NoDownload {
                id: self.id.to_string(),
                version: self.version.to_string(),
                platform: platform_text(),
            })
    }

    /// Whether every name in the pin is one folder or file name, so the pin
    /// cannot name a path outside `<data_dir>/bridges`
    fn names_usable(&self) -> bool {
        is_name(self.id)
            && is_name(self.version)
            && self
                .platforms
                .iter()
                .all(|(_, download)| is_name(&download.cmd))
    }
}

/// Whether a text names one folder or file, and not a path through them
pub(crate) fn is_name(text: &str) -> bool {
    !text.is_empty() && text != "." && text != ".." && !text.contains('/') && !text.contains('\\')
}

/// Install `pin`'s bridge when it is not installed, and return its command path
///
/// The command lands at `<data_dir>/bridges/<id>/<version>/<cmd>`, and is reused
/// when hashing it gives back the sha256 recorded beside it at install.
/// Otherwise kelpie downloads the archive, checks its sha256 against the pin
/// before unpacking, and unpacks only `cmd` into a folder beside the target,
/// which it renames into place, so a half-written install is never used. The
/// rest of the archive is left out, which drops the Claude Code binary the
/// registry's archive carries (ADR 0007).
///
/// # Errors
///
/// [`BridgeError`] says which thing broke: no archive for this platform, a failed
/// fetch, a checksum mismatch, an archive format kelpie does not unpack, no `cmd`
/// in the archive, an unusable name in the pin, or the OS refusing a read or write.
pub fn ensure(data_dir: &Path, pin: &Pin<'_>) -> Result<PathBuf, BridgeError> {
    if !pin.names_usable() {
        return Err(BridgeError::PinNotUsable {
            name: unusable_name(pin),
        });
    }
    let download = pin.download()?;
    let dir = data_dir.join(BRIDGES_FOLDER).join(pin.id).join(pin.version);
    let target = dir.join(&download.cmd);
    if is_installed(&target) {
        return Ok(target);
    }
    install(&dir, download)?;
    Ok(target)
}

/// The command kelpie runs for `id` at `version`, `None` when it is not installed or not intact
///
/// [`check`] installs a newer version before it probes it, so a session taking
/// that version into use finds it here instead of downloading it again.
#[must_use]
pub fn installed(data_dir: &Path, id: &str, version: &Version, cmd: &str) -> Option<PathBuf> {
    if !is_name(id) || !is_name(cmd) {
        return None;
    }
    let target = data_dir
        .join(BRIDGES_FOLDER)
        .join(id)
        .join(version.to_string())
        .join(cmd);
    is_installed(&target).then_some(target)
}

/// The first name in the pin that is not one folder or file name
fn unusable_name(pin: &Pin<'_>) -> String {
    if !is_name(pin.id) {
        return pin.id.to_string();
    }
    if !is_name(pin.version) {
        return pin.version.to_string();
    }
    pin.platforms
        .iter()
        .find_map(|(_, download)| (!is_name(&download.cmd)).then(|| download.cmd.clone()))
        .unwrap_or_default()
}

/// Whether `target` is this bridge, installed and intact
///
/// Anything wrong here, a missing record or a file that no longer hashes to it,
/// means the bridge is installed again rather than run in the state it is in.
fn is_installed(target: &Path) -> bool {
    let Ok(record) = fs::read_to_string(record_path(target)) else {
        return false;
    };
    let Ok(sum) = hash_file(target) else {
        return false;
    };
    sum == record.trim()
}

/// Download the archive, check it, and move what it holds into place
fn install(dir: &Path, download: &Download) -> Result<(), BridgeError> {
    let format = ArchiveFormat::from_url(&download.url)?;
    fs::create_dir_all(dir).map_err(|error| could_not_write(dir, error))?;

    // One install per version at a time: the sweep below removes every staging
    // folder it finds, which is only safe when no other install is running. A
    // second call waits here, then finds the bridge installed and downloads
    // nothing. The OS drops the lock when its holder exits, so a run that died
    // never blocks the next one.
    let _guard = lock_install(dir)?;
    if is_installed(&dir.join(&download.cmd)) {
        return Ok(());
    }
    clear_stale(dir)?;
    let token = token().map_err(|error| could_not_write(dir, error))?;
    let temp = dir.join(format!("{TEMP_PREFIX}{token}"));
    fs::create_dir_all(&temp).map_err(|error| could_not_write(&temp, error))?;

    let outcome = stage(&temp, download, format).and_then(|()| land(dir, &temp, download));
    let _ = fs::remove_dir_all(&temp);
    outcome
}

/// Take the exclusive lock that serialises installs of one version, waiting for it
///
/// The lock file sits beside the version's folder, not in it, so the folder
/// holds only the command and its record. The lock is released when the
/// returned file is dropped.
fn lock_install(dir: &Path) -> Result<File, BridgeError> {
    let name = dir.file_name().unwrap_or_default().to_string_lossy();
    let path = dir.with_file_name(format!(".{name}{LOCK_SUFFIX}"));
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| could_not_write(&path, error))?;
    file.lock().map_err(|error| could_not_write(&path, error))?;
    Ok(file)
}

/// Fetch the archive into `temp` and unpack only its command out of it
fn stage(temp: &Path, download: &Download, format: ArchiveFormat) -> Result<(), BridgeError> {
    let archive = temp.join(ARCHIVE_NAME);
    let sum = fetch(&download.url, &archive)?;
    if !sum.eq_ignore_ascii_case(&download.sha256) {
        return Err(BridgeError::ChecksumMismatch {
            url: download.url.clone(),
            expected: download.sha256.clone(),
            actual: sum,
        });
    }
    let cmd = temp.join(&download.cmd);
    match format {
        ArchiveFormat::TarGz => untar(&archive, download, &cmd)?,
        ArchiveFormat::Zip => unzip(&archive, download, &cmd)?,
    }
    Ok(())
}

/// Move the staged command to the target, with its sha256 recorded beside it
fn land(dir: &Path, temp: &Path, download: &Download) -> Result<(), BridgeError> {
    let staged = temp.join(&download.cmd);
    let target = dir.join(&download.cmd);
    let record = record_path(&target);
    let sum = hash_file(&staged).map_err(|error| could_not_read(&staged, error))?;
    make_runnable(&staged).map_err(|error| could_not_write(&staged, error))?;
    if target
        .try_exists()
        .map_err(|error| could_not_read(&target, error))?
    {
        fs::remove_file(&target).map_err(|error| could_not_write(&target, error))?;
    }
    fs::rename(&staged, &target).map_err(|error| could_not_write(&target, error))?;
    fs::write(&record, format!("{sum}\n")).map_err(|error| could_not_write(&record, error))?;
    Ok(())
}

/// Remove what an install that died mid-way left behind
///
/// Only called holding the install lock, so none of it is in use.
fn clear_stale(dir: &Path) -> Result<(), BridgeError> {
    let entries = fs::read_dir(dir).map_err(|error| could_not_read(dir, error))?;
    for entry in entries {
        let entry = entry.map_err(|error| could_not_read(dir, error))?;
        let path = entry.path();
        if !entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            continue;
        }
        let is_dir = entry
            .file_type()
            .map_err(|error| could_not_read(&path, error))?
            .is_dir();
        let removed = if is_dir {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed.map_err(|error| could_not_write(&path, error))?;
    }
    Ok(())
}

/// Download `url` into `archive`, returning the hex sha256 of what arrived
///
/// The sum is taken while the bytes are written, so the archive is never used
/// before it has been checked.
fn fetch(url: &str, archive: &Path) -> Result<String, BridgeError> {
    let file = File::create(archive).map_err(|error| could_not_write(archive, error))?;
    let mut hashing = HashingWriter::new(file);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    let response = agent
        .get(url)
        .call()
        .map_err(|error| could_not_download(url, io::Error::other(error)))?;
    let status = response.status();
    if !status.is_success() {
        let reason = io::Error::other(format!("the server answered {status}"));
        return Err(could_not_download(url, reason));
    }
    let mut body = response
        .into_body()
        .into_with_config()
        .limit(LIMIT)
        .reader();
    io::copy(&mut body, &mut hashing).map_err(|error| could_not_download(url, error))?;
    Ok(hashing.sum())
}

/// Copy out the one member named `cmd`, leaving every other in the archive
fn untar(archive: &Path, download: &Download, out: &Path) -> Result<(), BridgeError> {
    let file = File::open(archive).map_err(|error| could_not_read(archive, error))?;
    let mut gzip = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let entries = gzip
        .entries()
        .map_err(|error| could_not_read(archive, error))?;
    for entry in entries {
        let mut entry = entry.map_err(|error| could_not_read(archive, error))?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let path = entry
            .path()
            .map_err(|error| could_not_read(archive, error))?;
        if !is_command(&path, &download.cmd) {
            continue;
        }
        return entry
            .unpack(out)
            .map(|_| ())
            .map_err(|error| could_not_write(out, error));
    }
    Err(missing_command(download))
}

/// Copy out the one entry named `cmd`, leaving every other in the archive
fn unzip(archive: &Path, download: &Download, out: &Path) -> Result<(), BridgeError> {
    let file = File::open(archive).map_err(|error| could_not_read(archive, error))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|error| could_not_read(archive, io::Error::other(error)))?;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| could_not_read(archive, io::Error::other(error)))?;
        if !entry.is_file() {
            continue;
        }
        if entry.name().rsplit('/').next() != Some(download.cmd.as_str()) {
            continue;
        }
        let mut handle = File::create(out).map_err(|error| could_not_write(out, error))?;
        return io::copy(&mut entry, &mut handle)
            .map(|_| ())
            .map_err(|error| could_not_write(out, error));
    }
    Err(missing_command(download))
}

/// Whether an archive member is the command kelpie installs, at any depth
fn is_command(path: &Path, cmd: &str) -> bool {
    path.file_name().and_then(OsStr::to_str) == Some(cmd)
}

/// The archive formats kelpie unpacks, told apart by the url's extension
enum ArchiveFormat {
    /// A gzipped tar, which the registry ships on Linux
    TarGz,

    /// A zip, which the registry ships on macOS (ADR 0007)
    Zip,
}

impl ArchiveFormat {
    /// Which format `url` names
    fn from_url(url: &str) -> Result<Self, BridgeError> {
        let name = url.to_ascii_lowercase();
        if name.ends_with(".tar.gz") {
            Ok(Self::TarGz)
        } else if name.ends_with(".zip") {
            Ok(Self::Zip)
        } else {
            Err(BridgeError::ArchiveFormat {
                url: url.to_string(),
            })
        }
    }
}

/// The file beside an installed bridge that records its sha256
fn record_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(OsString::from).unwrap_or_default();
    name.push(RECORD_SUFFIX);
    target.with_file_name(name)
}

/// The sha256 of a whole file, as hex
fn hash_file(path: &Path) -> io::Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hashing = HashingWriter::new(io::sink());
    io::copy(&mut reader, &mut hashing)?;
    Ok(hashing.sum())
}

/// A folder name no other in-progress install shares
fn token() -> io::Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes)?;
    Ok(hex(bytes))
}

/// Bytes as lowercase hex
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A writer that keeps the sha256 of everything written through it
struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha256,
}

impl<W: Write> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    /// The hex sha256 of everything written through it
    fn sum(self) -> String {
        hex(self.hasher.finalize())
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.hasher.update(buf);
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Give the installed command the run bits, which an archive may not carry
#[cfg(unix)]
fn make_runnable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))
}

/// Nothing to do: Windows runs a file by its extension, not its mode bits
#[cfg(not(unix))]
fn make_runnable(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
