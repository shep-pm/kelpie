//! What the host can run: a sandbox that runs commands, or none at all
//!
//! A refusal test that ran in a sandbox which never started would pass, as a
//! command that did not run leaves no file and exits non-zero. So every test asks
//! for a sandbox that has run a command first, and a host that cannot fails.
//! `KELPIE_SKIP_SANDBOX_TESTS` skips it instead, shown as `SKIPPED`, for a host
//! known not to run one. A child of `tests/sandbox.rs`, which is its own `main`
//! (crabpot's Linux helper is this binary run again).

use std::path::Path;
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use kelpie_engine::sandbox::{Reads, Sandbox};
use kelpie_engine::tools::Door;

use super::support::{Scratch, sh};

/// Set to skip, not fail, the tests on a host that cannot run a sandbox
///
/// Off by default, so a runner that cannot run one is red and not a false
/// green. A skipped test is printed as `SKIPPED`, never as `ok`.
const SKIP: &str = "KELPIE_SKIP_SANDBOX_TESTS";

/// How many tests have skipped themselves, which `main` reads to print each
/// test's verdict
pub(super) static SKIPPED: AtomicUsize = AtomicUsize::new(0);

/// What `bwrap: loopback: Failed RTM_NEWADDR` means, said once
///
/// bwrap made its user namespace but the kernel gave it no capabilities in it,
/// so it cannot set up the namespace's loopback. On Ubuntu 24.04 and later that
/// is `kernel.apparmor_restrict_unprivileged_userns` = 1 on the host, which no
/// container option lifts: crabpot's README, Linux setup, gives the AppArmor
/// profile that does, or the sysctl that turns the restriction off.
const WITHOUT_CAPABILITIES: &str = "\nbwrap's user namespace has no capabilities, which on Ubuntu 24.04 and later is the host's `kernel.apparmor_restrict_unprivileged_userns` = 1 (see crabpot's README, Linux setup). The host, not the container's options, decides.";

/// What bwrap prints first when the kernel gives it no user namespace, as the
/// runner's does while `kernel.apparmor_restrict_unprivileged_userns` is on
fn host_refuses_sandboxes(stderr: &str) -> bool {
    cfg!(target_os = "linux") && stderr.starts_with("bwrap:")
}

/// `sandbox`, once it is known to run commands, or `None` when this host cannot
/// run one at all
///
/// A refusal test that ran in a sandbox which never started would pass, as a
/// command that did not run leaves no file and exits non-zero. So each test
/// asks for a sandbox that has run a command first, and a host that cannot
/// fails the test. `KELPIE_SKIP_SANDBOX_TESTS` skips it instead, shown as
/// `SKIPPED`, for a host known not to run one.
pub(super) fn working(sandbox: Sandbox, test: &str) -> Option<Sandbox> {
    host_can_run(test, &sh(&sandbox, "true")).then_some(sandbox)
}

/// [`working`], on a sandbox for a session in the scratch that reads as `reads`,
/// whose harness is `program`
pub(super) fn working_sandbox(
    scratch: &Scratch,
    test: &str,
    program: &Path,
    reads: Reads,
) -> Option<Sandbox> {
    working_sandbox_with_door(scratch, test, program, reads, None)
}

/// [`working_sandbox`], with kelpie's tools `door` named in the policy
pub(super) fn working_sandbox_with_door(
    scratch: &Scratch,
    test: &str,
    program: &Path,
    reads: Reads,
    door: Option<&Door>,
) -> Option<Sandbox> {
    working(scratch.sandbox(program, reads, door), test)
}

/// Whether `probe`, `true` run in a sandbox, shows this host runs one
///
/// A host that cannot fails the test, or skips it when [`SKIP`] is set.
pub(super) fn host_can_run(test: &str, probe: &Output) -> bool {
    if probe.status.success() {
        return true;
    }
    let stderr = String::from_utf8_lossy(&probe.stderr);
    assert!(
        host_refuses_sandboxes(&stderr) && std::env::var_os(SKIP).is_some(),
        "`true` did not run in the sandbox: {probe:?}\nThis host cannot run a sandbox. Set {SKIP} to skip these tests.{}",
        if stderr.contains("RTM_NEWADDR") {
            WITHOUT_CAPABILITIES
        } else {
            ""
        }
    );
    SKIPPED.fetch_add(1, Ordering::SeqCst);
    println!(
        "SKIPPED {test}: this host cannot run a sandbox ({})",
        stderr.trim()
    );
    false
}

pub(super) fn a_kernel_without_user_namespaces_is_told_from_a_command_that_failed() {
    // The runner's own words, from the log of the run that went red
    let refused = "bwrap: No permissions to create new namespace, likely because the kernel does not allow non-privileged user namespaces.";

    assert_eq!(host_refuses_sandboxes(refused), cfg!(target_os = "linux"));
    assert!(!host_refuses_sandboxes("sh: true: not found"));
    assert!(!host_refuses_sandboxes(""));
}
