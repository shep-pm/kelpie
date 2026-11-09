use std::fs;
use std::path::{Path, PathBuf};

use ureq::unversioned::transport::set_handler;

use super::*;
use crate::run::RunId;

/// The bridge every test installs
const ID: &str = "claude-acp";
const VERSION: &str = "0.101.1";

/// The command the fixtures hold, and the only one kelpie unpacks
const CMD: &str = "claude-code-acp";

/// What the command says, which is how a test tells a whole file from half one
const COMMAND_TEXT: &str = "#!/bin/sh\necho kelpie bridge fixture\n";

/// Every platform kelpie ships on
const PLATFORMS: [Platform; 5] = [
    Platform::LinuxX64,
    Platform::LinuxArm64,
    Platform::MacOsX64,
    Platform::MacOsArm64,
    Platform::WindowsX64,
];

/// One fixture, served at a fixed url
struct Fixture {
    /// Its file under `tests/fixtures`
    name: &'static str,

    /// The url kelpie fetches it from
    url: &'static str,

    /// The text the server answers a url containing it with
    pattern: &'static str,
}

const TAR: Fixture = Fixture {
    name: "bridge.tar.gz",
    url: "http://127.0.0.1:8099/bridge.tar.gz",
    pattern: "/bridge.tar.gz",
};

const ZIP: Fixture = Fixture {
    name: "bridge.zip",
    url: "http://127.0.0.1:8099/bridge.zip",
    pattern: "/bridge.zip",
};

impl Fixture {
    /// Answer every request for this fixture with the fixture's own bytes
    ///
    /// ureq's **_test** transport answers a request inside the process instead of
    /// opening a socket, which the sandbox kelpie workers run in refuses. The
    /// request and the response are still HTTP/1.1, written and parsed by ureq.
    fn serve(&self) -> &Fixture {
        let body = fs::read(fixture_path(self.name)).unwrap();
        let length = body.len().to_string();
        set_handler(
            self.pattern,
            200,
            &[
                ("content-type", "application/octet-stream"),
                ("content-length", length.as_str()),
            ],
            &body,
        );
        self
    }
}

/// A data dir of its own for one test, never written over by another
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-bridge-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn data(&self) -> &Path {
        &self.0
    }

    /// The folder one pinned version installs into
    fn version(&self) -> PathBuf {
        self.0.join("bridges").join(ID).join(VERSION)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A fixture's path in the crate
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// The archive one test serves, with the sha256 the fixture really has
fn download(fixture: &Fixture) -> Download {
    Download {
        url: fixture.url.to_string(),
        sha256: hash_file(&fixture_path(fixture.name)).unwrap(),
        cmd: CMD.to_string(),
    }
}

/// The one platform this test runs on, with `fixture` as its archive
fn platforms_for(fixture: &Fixture) -> [(Platform, Download); 1] {
    [(this_platform(), download(fixture))]
}

/// The platform these tests run on
fn this_platform() -> Platform {
    Platform::current().expect("a platform kelpie ships on")
}

/// A pin over the given platforms
fn pin(platforms: &[(Platform, Download)]) -> Pin<'_> {
    Pin {
        id: ID,
        version: VERSION,
        platforms,
    }
}

/// The names inside a folder, sorted
fn entries(dir: &Path) -> Vec<String> {
    let mut names = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<String>>();
    names.sort();
    names
}

#[test]
fn a_bridge_installs_once_and_is_reused() {
    let dir = Dir::new();
    let platforms = platforms_for(TAR.serve());

    let first = ensure(dir.data(), &pin(&platforms)).unwrap();
    let second = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(first, second);
    assert_eq!(first, dir.version().join(CMD));
    assert_eq!(fs::read_to_string(&first).unwrap(), COMMAND_TEXT);
    assert_eq!(
        fs::read_to_string(record_path(&first)).unwrap().trim(),
        hash_file(&first).unwrap()
    );

    // A pin whose archive would now hash wrong still reuses what is installed,
    // which is the same call answering without fetching anything
    let mut changed = download(&TAR);
    changed.sha256 = "0".repeat(64);
    let reused = ensure(dir.data(), &pin(&[(this_platform(), changed)])).unwrap();
    assert_eq!(reused, first);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&first).unwrap().permissions().mode();
        assert_ne!(mode & 0o111, 0, "{first:?} is not runnable");
    }
}

#[test]
fn a_zip_archive_installs_too() {
    let dir = Dir::new();
    let platforms = platforms_for(ZIP.serve());

    let installed = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(installed, dir.version().join(CMD));
    assert_eq!(fs::read_to_string(&installed).unwrap(), COMMAND_TEXT);
}

#[test]
fn a_checksum_mismatch_installs_nothing() {
    let dir = Dir::new();
    TAR.serve();
    let mut archive = download(&TAR);
    archive.sha256 = "0".repeat(64);

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(
        matches!(error, BridgeError::ChecksumMismatch { .. }),
        "{error}"
    );
    let text = error.to_string();
    assert!(text.contains(&"0".repeat(64)), "no expected sum in {text}");
    assert!(
        text.contains(&hash_file(&fixture_path(TAR.name)).unwrap()),
        "no actual sum in {text}"
    );
    assert_eq!(
        entries(&dir.version()),
        Vec::<String>::new(),
        "it installed anyway"
    );
}

#[test]
fn only_the_command_is_unpacked() {
    let dir = Dir::new();
    let platforms = platforms_for(TAR.serve());

    let installed = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(fs::read_to_string(&installed).unwrap(), COMMAND_TEXT);
    assert_eq!(
        entries(&dir.version()),
        [CMD.to_string(), format!("{CMD}.sha256")]
    );
}

#[test]
fn an_interrupted_install_is_not_used() {
    let dir = Dir::new();
    let version = dir.version();
    fs::create_dir_all(&version).unwrap();
    // What a run that died mid-install leaves behind: half a command, and the
    // folder it was being staged in
    fs::write(version.join(CMD), "#!/bin/sh\necho hal").unwrap();
    fs::create_dir_all(version.join(".install-stale")).unwrap();
    fs::write(version.join(".install-stale").join(CMD), "half an archive").unwrap();

    let platforms = platforms_for(TAR.serve());
    let installed = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(installed, version.join(CMD));
    assert_eq!(fs::read_to_string(&installed).unwrap(), COMMAND_TEXT);
    assert_eq!(
        entries(&version),
        [CMD.to_string(), format!("{CMD}.sha256")],
        "the staged folder was left beside it"
    );
}

#[test]
fn a_platform_with_no_download_says_so() {
    let dir = Dir::new();
    let other = PLATFORMS
        .into_iter()
        .find(|platform| *platform != this_platform())
        .unwrap();

    let error = ensure(dir.data(), &pin(&[(other, download(&TAR))])).unwrap_err();

    assert!(matches!(error, BridgeError::NoDownload { .. }), "{error}");
    assert!(
        error.to_string().contains(&format!("{ID} {VERSION}")),
        "{error}"
    );
    assert!(!dir.data().join("bridges").exists());
}

#[test]
fn an_archive_format_kelpie_does_not_unpack_says_so() {
    let dir = Dir::new();
    let mut archive = download(&TAR);
    archive.url = "http://127.0.0.1:8099/bridge.tar.xz".into();
    archive.sha256 = "0".repeat(64);

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(
        matches!(error, BridgeError::ArchiveFormat { .. }),
        "{error}"
    );
    assert!(error.to_string().contains(".tar.xz"), "{error}");
}

#[test]
fn an_archive_without_the_command_says_so() {
    let dir = Dir::new();
    TAR.serve();
    let mut archive = download(&TAR);
    archive.cmd = "not-in-the-archive".into();

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(
        matches!(error, BridgeError::MissingCommand { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("not-in-the-archive"), "{error}");
    assert_eq!(entries(&dir.version()), Vec::<String>::new());
}

#[test]
fn a_pin_that_names_a_path_outside_the_bridges_folder_is_refused() {
    let dir = Dir::new();
    let mut archive = download(&TAR);
    archive.cmd = "../escape".into();

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(matches!(error, BridgeError::PinNotUsable { .. }), "{error}");
    assert!(error.to_string().contains("../escape"), "{error}");
    assert!(!dir.data().join("bridges").exists());
}
