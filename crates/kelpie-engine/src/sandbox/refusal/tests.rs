use super::*;
use crate::sandbox::{Project, Reads, check_project};

#[test]
fn a_loopback_failure_is_no_capabilities() {
    let stderr = "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted\n";

    assert_eq!(Refusal::of(stderr), Some(Refusal::NoCapabilities));
}

#[test]
fn a_missing_namespace_permission_is_no_namespaces() {
    let stderr = "bwrap: No permissions to create new namespace, likely because the kernel does not allow non-privileged user namespaces.\n";

    assert_eq!(Refusal::of(stderr), Some(Refusal::NoNamespaces));
}

#[test]
fn another_bwrap_failure_is_neither() {
    let stderr = "bwrap: Can't find source path /nowhere: No such file or directory\n";

    assert_eq!(Refusal::of(stderr), None);
    assert_eq!(Refusal::of(""), None);
}

#[test]
fn the_refusal_words_in_a_commands_own_output_are_not_a_refusal() {
    let later_line = "sh: 1: oops\nbwrap: loopback: Failed RTM_NEWADDR: Operation not permitted\n";
    let after_a_program = "git: bwrap: No permissions to create new namespace\n";

    assert_eq!(Refusal::of(later_line), None);
    assert_eq!(Refusal::of(after_a_program), None);
}

#[cfg(unix)]
fn output(code: i32, stderr: &str) -> Output {
    use std::os::unix::process::ExitStatusExt;

    Output {
        status: std::process::ExitStatus::from_raw(code << 8),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

#[cfg(unix)]
#[test]
fn an_exit_of_zero_is_ready_whatever_stderr_says() {
    let stderr = "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted\n";

    assert!(judge(&output(0, stderr)).is_ok());
}

#[cfg(unix)]
#[test]
fn a_refusal_in_a_failed_probe_is_the_refused_error() {
    let loopback = "bwrap: loopback: Failed RTM_NEWADDR: Operation not permitted\n";
    let namespaces = "bwrap: No permissions to create new namespace\n";

    assert!(matches!(
        judge(&output(1, loopback)),
        Err(SandboxError::Refused(Refusal::NoCapabilities))
    ));
    assert!(matches!(
        judge(&output(1, namespaces)),
        Err(SandboxError::Refused(Refusal::NoNamespaces))
    ));
}

#[cfg(unix)]
#[test]
fn a_failure_that_is_no_refusal_keeps_stderr_in_the_probe_error() {
    let stderr = "sh: 1: true: not found\n";

    let Err(SandboxError::Probe(detail)) = judge(&output(127, stderr)) else {
        panic!("a failed probe that is no refusal is not a probe error");
    };
    assert_eq!(detail, stderr);
}

#[test]
fn a_project_at_the_home_folder_is_refused_without_starting_a_sandbox() {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .expect("the OS names a home folder");
    let project = Project {
        folder: &home,
        network_allowlist: &[],
        reads: Reads::Project,
        sessions_root: std::path::Path::new("/state/projects"),
    };

    let error = check_project(&project).expect_err("the home folder is no project");

    assert!(matches!(error, SandboxError::ProjectAtHome { folder } if folder == home));
}
