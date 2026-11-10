//! Installing a bridge from a pin: what lands on disk, what is refused, and
//! what a second call reuses
//!
//! `ensure` in `bridge/mod.rs`

use std::fs;
use std::thread;
use std::time::Duration;

use super::*;
#[test]

fn a_bridge_installs_once_and_is_reused() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);

    let first = ensure(dir.data(), &pin(&platforms)).unwrap();
    let second = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(first, second);
    assert_eq!(server.served(), 1, "the second call fetched again");
    assert_eq!(first, dir.version().join(CMD));
    assert_eq!(fs::read_to_string(&first).unwrap(), COMMAND_TEXT);
    assert_eq!(
        fs::read_to_string(record_path(&first)).unwrap().trim(),
        hash_file(&first).unwrap()
    );

    // A pin whose archive would now hash wrong still reuses what is installed,
    // which is the same call answering without fetching anything
    let mut changed = download(&server, TAR);
    changed.sha256 = "0".repeat(64);
    let reused = ensure(dir.data(), &pin(&[(this_platform(), changed)])).unwrap();
    assert_eq!(reused, first);
    assert_eq!(server.served(), 1);

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
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, ZIP);

    let installed = ensure(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(installed, dir.version().join(CMD));
    assert_eq!(fs::read_to_string(&installed).unwrap(), COMMAND_TEXT);
}

#[test]

fn a_checksum_mismatch_installs_nothing() {
    let dir = Dir::new();
    let server = Server::start(Duration::ZERO);
    let mut archive = download(&server, TAR);
    archive.sha256 = "0".repeat(64);

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(
        matches!(error, BridgeError::ChecksumMismatch { .. }),
        "{error}"
    );
    let text = error.to_string();
    assert!(text.contains(&"0".repeat(64)), "no expected sum in {text}");
    assert!(
        text.contains(&hash_file(&fixture_path(TAR)).unwrap()),
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
    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);

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

    let server = Server::start(Duration::ZERO);
    let platforms = platforms_for(&server, TAR);
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

fn installs_that_overlap_fetch_once_and_both_succeed() {
    let dir = Dir::new();
    // Slow enough that the second call starts while the first is downloading
    let server = Server::start(Duration::from_millis(300));
    let platforms = platforms_for(&server, TAR);

    let results = thread::scope(|scope| {
        let first = scope.spawn(|| ensure(dir.data(), &pin(&platforms)));
        thread::sleep(Duration::from_millis(100));
        let second = scope.spawn(|| ensure(dir.data(), &pin(&platforms)));
        [first.join().unwrap(), second.join().unwrap()]
    });

    for result in results {
        let installed = result.unwrap();
        assert_eq!(fs::read_to_string(installed).unwrap(), COMMAND_TEXT);
    }
    assert_eq!(server.served(), 1, "the overlapping call fetched again");
    assert_eq!(
        entries(&dir.version()),
        [CMD.to_string(), format!("{CMD}.sha256")]
    );
}

#[test]

fn a_platform_with_no_download_says_so() {
    let dir = Dir::new();
    let other = PLATFORMS
        .into_iter()
        .find(|platform| *platform != this_platform())
        .unwrap();

    let error = ensure(dir.data(), &pin(&[(other, unfetched(TAR))])).unwrap_err();

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
    let mut archive = unfetched(TAR);
    archive.url = "http://127.0.0.1:1/bridge.tar.xz".into();
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
    let server = Server::start(Duration::ZERO);
    let mut archive = download(&server, TAR);
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
    let mut archive = unfetched(TAR);
    archive.cmd = "../escape".into();

    let error = ensure(dir.data(), &pin(&[(this_platform(), archive)])).unwrap_err();

    assert!(matches!(error, BridgeError::PinNotUsable { .. }), "{error}");
    assert!(error.to_string().contains("../escape"), "{error}");
    assert!(!dir.data().join("bridges").exists());
}
