//! Tests for what the PM's last start found about its sandbox

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use kelpie_engine::sandbox::{Refusal, SandboxError};

use super::install::PROFILE;
use super::{Install, RefusalView, SandboxSetup, SandboxView, proven};

#[test]
fn a_start_that_found_no_refusal_says_none() {
    let setup = SandboxSetup::default();
    assert_eq!(setup.view(), SandboxView::default());

    setup.record(None);
    assert_eq!(setup.view(), SandboxView::default());
}

#[test]
fn a_refusal_is_kept_until_a_later_start_finds_none() {
    let setup = SandboxSetup::default();
    setup.record(Some(Refusal::NoCapabilities));
    assert!(setup.view().refusal.is_some());
    assert!(
        setup.view().refusal.is_some(),
        "a look at the view forgets nothing"
    );

    setup.record(None);
    assert_eq!(setup.view(), SandboxView::default());
}

#[test]
fn the_two_refusals_are_told_apart_in_the_view() {
    let setup = SandboxSetup::default();

    setup.record(Some(Refusal::NoCapabilities));
    assert_eq!(setup.view().refusal, Some(RefusalView::NoCapabilities));

    setup.record(Some(Refusal::NoNamespaces));
    assert_eq!(setup.view().refusal, Some(RefusalView::NoNamespaces));

    // The webview reads these two strings
    let wire = serde_json::to_string(&setup.view()).unwrap();
    assert_eq!(
        wire,
        r#"{"refusal":"no_namespaces","unsandboxed":false,"install":"package_only"}"#
    );
}

#[test]
fn an_unsandboxed_start_is_kept_with_its_refusal() {
    let setup = SandboxSetup::default();

    setup.record_unsandboxed(Refusal::NoCapabilities);

    assert_eq!(
        setup.view(),
        SandboxView {
            refusal: Some(RefusalView::NoCapabilities),
            unsandboxed: true,
            install: Install::PackageOnly,
        }
    );

    // The next start finds a sandbox that works, so the session is sandboxed again
    setup.record(None);
    assert_eq!(setup.view(), SandboxView::default());
}

/// A folder of its own, holding a file the way a package leaves its profile
struct Packaged(PathBuf);

impl Packaged {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "kelpie-sandbox-setup-{}",
            kelpie_engine::run::RunId::new().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("kelpie.apparmor"), "profile kelpie {}").unwrap();
        Self(dir)
    }

    fn profile(&self) -> PathBuf {
        self.0.join("kelpie.apparmor")
    }
}

impl Drop for Packaged {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_packaged_host_is_offered_install() {
    let package = Packaged::new();

    assert_eq!(
        Install::detect(None, &package.profile()),
        Install::Available
    );
}

#[test]
fn an_appimage_is_offered_only_run_without_a_sandbox() {
    // Even where the profile is on disk, a launch from a mount has no path to name in it
    let package = Packaged::new();
    let appimage = OsStr::new("/tmp/.mount_kelpie1a2b3c/kelpie.AppImage");

    assert_eq!(
        Install::detect(Some(appimage), &package.profile()),
        Install::PackageOnly
    );
}

#[test]
fn a_host_without_the_packaged_profile_is_offered_only_run_without_a_sandbox() {
    // A dev build says what an AppImage says
    let package = Packaged::new();
    let missing = package.0.join("no-such.apparmor");

    assert_eq!(Install::detect(None, &missing), Install::PackageOnly);
}

#[test]
fn what_a_start_records_carries_what_the_host_offers() {
    let setup = SandboxSetup::offering(Install::Available);
    assert_eq!(setup.view().install, Install::Available);

    setup.record(Some(Refusal::NoCapabilities));
    assert_eq!(setup.view().install, Install::Available);

    setup.record_unsandboxed(Refusal::NoCapabilities);
    assert_eq!(setup.view().install, Install::Available);

    let wire = serde_json::to_string(&setup.view()).unwrap();
    assert_eq!(
        wire,
        r#"{"refusal":"no_capabilities","unsandboxed":true,"install":"available"}"#
    );
}

#[test]
fn the_packaged_profile_path_is_where_tauri_conf_ships_it() {
    let conf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
    let conf: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(conf).unwrap()).unwrap();

    for package in ["deb", "rpm"] {
        let files = &conf["bundle"]["linux"][package]["files"];
        assert_eq!(
            files[PROFILE], "linux/kelpie.apparmor",
            "the {package} must ship the profile at {PROFILE}"
        );
    }
}

#[test]
fn a_probe_that_refuses_gives_the_refusal_and_not_the_sandbox() {
    let error = proven(Ok("sandbox"), |_| {
        Err(SandboxError::Refused(Refusal::NoCapabilities))
    })
    .unwrap_err();

    assert!(matches!(
        error,
        SandboxError::Refused(Refusal::NoCapabilities)
    ));
    assert!(!error.to_string().contains("RTM_NEWADDR"));
}

#[test]
fn a_probe_that_passes_gives_the_started_value() {
    let started = proven(Ok("sandbox"), |sandbox| {
        assert_eq!(*sandbox, "sandbox");
        Ok(())
    });

    assert_eq!(started.unwrap(), "sandbox");
}

#[test]
fn a_start_that_failed_runs_no_probe() {
    let error = proven(
        Err::<&str, _>(SandboxError::Probe("no bwrap".to_owned())),
        |_| panic!("ready ran on a start that failed"),
    )
    .unwrap_err();

    assert!(matches!(error, SandboxError::Probe(text) if text == "no bwrap"));
}
