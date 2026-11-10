//! `versions.json`: which version a session starts on, which one it goes back to,
//! and which folders stay on disk
//!
//! `bridge/versions.rs`

use std::fs;
use std::path::PathBuf;

use super::*;
use crate::settings::BridgeUpdates;
/// The versions file of the bridge every test installs
fn versions_file(dir: &Dir) -> PathBuf {
    dir.data().join("bridges").join(ID).join("versions.json")
}
#[test]

fn with_no_file_the_pin_is_in_use_and_a_save_reads_back() {
    let dir = Dir::new();
    let platforms = [(this_platform(), unfetched(TAR))];

    let mut versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    assert_eq!(versions.in_use, version(VERSION));
    assert_eq!(versions.previous, None);
    assert_eq!(versions.ready, None);
    assert!(versions.held_back.is_empty() && versions.bad.is_empty());

    versions.ready = Some(version("0.102.0"));
    versions.bad.push(Rejected {
        version: version("0.100.0"),
        reason: "failed to resume".to_string(),
    });
    versions.save(dir.data(), ID).unwrap();

    assert_eq!(
        Versions::load(dir.data(), &pin(&platforms)).unwrap(),
        versions
    );
    assert_eq!(
        entries(versions_file(&dir).parent().unwrap()),
        ["versions.json"],
        "the temporary file was left behind"
    );
}

#[test]

fn the_versions_file_is_the_json_people_and_later_builds_read() {
    let dir = Dir::new();
    let platforms = [(this_platform(), unfetched(TAR))];
    fs::create_dir_all(versions_file(&dir).parent().unwrap()).unwrap();
    fs::write(
        versions_file(&dir),
        r#"{
  "in_use": "0.101.1",
  "previous": "0.101.0",
  "ready": "0.102.0",
  "held_back": [{"version": "0.103.0", "reason": "no session/resume"}],
  "bad": []
}"#,
    )
    .unwrap();

    let versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(versions.in_use, version("0.101.1"));
    assert_eq!(versions.previous, Some(version("0.101.0")));
    assert_eq!(versions.ready, Some(version("0.102.0")));
    assert_eq!(
        versions.held_back,
        [Rejected {
            version: version("0.103.0"),
            reason: "no session/resume".to_string()
        }]
    );
}

#[test]

fn a_file_that_is_not_a_versions_file_is_an_error_not_a_reset() {
    let dir = Dir::new();
    let platforms = [(this_platform(), unfetched(TAR))];
    fs::create_dir_all(versions_file(&dir).parent().unwrap()).unwrap();
    fs::write(versions_file(&dir), "{ not json").unwrap();

    let error = Versions::load(dir.data(), &pin(&platforms)).unwrap_err();

    assert!(matches!(error, VersionsError::Malformed { .. }), "{error}");
    assert_eq!(
        fs::read_to_string(versions_file(&dir)).unwrap(),
        "{ not json"
    );
}
#[test]

fn ask_first_waits_for_approval() {
    let dir = Dir::new();
    let platforms = [(this_platform(), unfetched(TAR))];
    let mut versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    versions.ready = Some(version("0.102.0"));

    assert_eq!(
        versions.current(BridgeUpdates::RegistryAskFirst),
        version(VERSION),
        "an unapproved version was used"
    );
    assert_eq!(versions.in_use, version(VERSION));
    assert_eq!(versions.ready, Some(version("0.102.0")));

    assert_eq!(versions.approve(), Some(version("0.102.0")));
    versions.save(dir.data(), ID).unwrap();
    let mut approved = Versions::load(dir.data(), &pin(&platforms)).unwrap();

    assert_eq!(
        approved.current(BridgeUpdates::RegistryAskFirst),
        version("0.102.0")
    );
    assert_eq!(approved.in_use, version("0.102.0"));
    assert_eq!(approved.previous, Some(version(VERSION)));
    assert_eq!(approved.ready, None);

    approved.ready = Some(version("0.103.0"));
    assert_eq!(
        approved.current(BridgeUpdates::RegistryAskFirst),
        version("0.102.0"),
        "a newer version was used without being approved"
    );
    assert_eq!(
        approved.current(BridgeUpdates::KelpiesPin),
        version("0.102.0"),
        "kelpie's pin was swapped out"
    );
}

#[test]

fn automatic_takes_the_ready_version_and_keeps_the_one_before() {
    let dir = Dir::new();
    let platforms = [(this_platform(), unfetched(TAR))];
    let mut versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    versions.previous = Some(version("0.101.0"));
    versions.ready = Some(version("0.102.0"));

    assert_eq!(
        versions.current(BridgeUpdates::RegistryAutomatic),
        version("0.102.0")
    );
    assert_eq!(versions.in_use, version("0.102.0"));
    assert_eq!(versions.previous, Some(version(VERSION)));
    assert_eq!(versions.ready, None);
    assert_eq!(
        versions.current(BridgeUpdates::RegistryAutomatic),
        version("0.102.0"),
        "the same version was taken into use twice"
    );
}

#[test]

fn a_version_that_failed_a_session_goes_back_to_the_one_before() {
    let dir = Dir::new();
    let platforms = [(this_platform(), unfetched(TAR))];
    let mut versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    versions.ready = Some(version("0.102.0"));
    let taken = versions.current(BridgeUpdates::RegistryAutomatic);
    assert_eq!(taken, version("0.102.0"));

    let back = versions.failed(&version("0.102.0"), "could not resume the session");

    assert_eq!(back, Some(version(VERSION)));
    assert_eq!(versions.in_use, version(VERSION));
    assert_eq!(versions.previous, None);
    assert!(versions.is_bad(&version("0.102.0")));
    assert_eq!(
        versions.bad,
        [Rejected {
            version: version("0.102.0"),
            reason: "could not resume the session".to_string(),
        }]
    );
    assert_eq!(
        versions.failed(&version(VERSION), "nothing left to go back to"),
        None,
        "a version with nothing before it was fallen back on"
    );
}

#[test]

fn prune_keeps_in_use_previous_and_the_pin() {
    let dir = Dir::new();
    let platforms = [(this_platform(), unfetched(TAR))];
    let bridge = dir.data().join("bridges").join(ID);
    for held in ["0.100.0", VERSION, "0.101.0", "0.102.0", "0.103.0"] {
        fs::create_dir_all(bridge.join(held)).unwrap();
        fs::write(bridge.join(held).join(CMD), COMMAND_TEXT).unwrap();
    }
    let mut versions = Versions::load(dir.data(), &pin(&platforms)).unwrap();
    versions.in_use = version("0.102.0");
    versions.previous = Some(version(VERSION));
    versions.ready = Some(version("0.103.0"));
    versions.save(dir.data(), ID).unwrap();

    // A session still running on 0.100.0 keeps that version on disk
    versions
        .prune(dir.data(), &pin(&platforms), &[version("0.100.0")])
        .unwrap();

    assert_eq!(
        entries(&bridge),
        ["0.100.0", "0.101.1", "0.102.0", "0.103.0", "versions.json"],
        "0.101.0 is the only version nothing needs"
    );
    versions.prune(dir.data(), &pin(&platforms), &[]).unwrap();
    assert_eq!(
        entries(&bridge),
        ["0.101.1", "0.102.0", "0.103.0", "versions.json"],
        "a version no session runs on was removed"
    );
}
