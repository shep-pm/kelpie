//! Whether the PM runs in a sandbox, from what the folder check and the probe found
//!
//! `confinement` is pure: each test hands it the two results `open` gets and
//! reads what it decides.

use std::path::{Path, PathBuf};

use kelpie_engine::sandbox::{Project, Refusal, check_project};

use super::*;

fn refused() -> Result<(), SandboxError> {
    Err(SandboxError::Refused(Refusal::NoCapabilities))
}

#[test]
fn a_passing_probe_runs_sandboxed_whatever_the_setting() {
    for run_without in [false, true] {
        assert!(matches!(
            confinement(Ok(()), Ok(()), run_without),
            Ok(Confinement::Sandboxed)
        ));
    }
}

#[test]
fn a_refusal_with_the_setting_off_is_an_error() {
    let found = confinement(Ok(()), refused(), false);

    assert!(matches!(
        found,
        Err(SandboxError::Refused(Refusal::NoCapabilities))
    ));
}

#[test]
fn a_refusal_with_the_setting_on_runs_without_a_sandbox() {
    for refusal in [Refusal::NoCapabilities, Refusal::NoNamespaces] {
        let found = confinement(Ok(()), Err(SandboxError::Refused(refusal)), true);

        assert!(matches!(found, Ok(Confinement::Unsandboxed)));
    }
}

#[test]
fn a_project_in_the_home_folder_is_refused_with_the_setting_on() {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .expect("the OS names a home folder");
    let at_home = || {
        check_project(&Project {
            folder: &home,
            network_allowlist: &[],
            reads: Reads::Project,
            sessions_root: Path::new("/state/projects"),
        })
    };

    for probe in [Ok(()), refused()] {
        let found = confinement(at_home(), probe, true);

        assert!(matches!(
            found,
            Err(SandboxError::ProjectAtHome { folder }) if folder == home
        ));
    }
}

#[test]
fn another_failure_is_never_run_without_a_sandbox() {
    let failures = [
        SandboxError::Probe("sh: not found".to_owned()),
        SandboxError::StateDir(std::io::Error::other("read-only")),
    ];
    for failure in failures {
        let found = confinement(Ok(()), Err(failure), true);

        assert!(found.is_err(), "the setting covers the two refusals only");
    }
}

#[test]
fn an_unsandboxed_session_stays_marked_on_the_sends_that_reuse_it() {
    let agent = FakeAgent::new()
        .turn(Turn::new().text("one"))
        .turn(Turn::new().text("two"));
    // An opener that finds the sandbox refused and runs the session without one
    let opener: Opener = {
        let agent = agent.clone();
        Box::new(move |opening| {
            opening
                .sandbox_setup
                .record_unsandboxed(Refusal::NoCapabilities);
            Ok(agent.session(
                opening.project.path.clone(),
                opening.project.pm_session.clone(),
                Choice::default(),
            ))
        })
    };
    let chat = Chat::with(opener, Box::new(|| Ok(None)), None);
    chat.state().set_project(chat.dir.clone()).unwrap();
    chat.send("First").until_turn_ends();
    assert!(chat.state().sandbox_setup().view().unsandboxed);

    chat.send("Second").until_turn_ends();

    assert_eq!(agent.launches(), 1, "the second send reused the session");
    assert!(
        chat.state().sandbox_setup().view().unsandboxed,
        "the session still runs without a sandbox"
    );
}
