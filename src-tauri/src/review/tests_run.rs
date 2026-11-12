//! A reviewer's test, run in the worker's sandbox under the `cargo-test` lease
//!
//! The pass runs a reviewer's test, it does not trust it (`docs/design/review.md`, A
//! reviewer's failing test). The engine's [`kelpie_engine::review::tests_before`] and
//! [`kelpie_engine::review::tests_after`] put the test in the worktree and hand each run to a
//! [`TestRunner`]; [`SandboxedTests`] is the app's.
//!
//! A [`TestRun`] is `kelpie lease run cargo-test -- sh -c <command>`, in the worktree and
//! inside the sandbox the worker's own policy makes ([`Sandbox::start_policy`] on
//! [`TestRun::reach`]), so the command reaches nothing the worker does not, and two workers'
//! test runs take turns as their `cargo test` does. The command is the project's, so it is
//! whatever language it runs.
//!
//! The run's bound is the run's own: [`TestRun::ceiling`] goes to `kelpie lease run` in
//! [`LIMIT_VAR`], which counts it from the grant and not from the wait in the queue, and
//! ends the command with everything it started when it passes. Of what the command wrote,
//! [`OUTPUT_KEPT`] bytes are kept, from the end, as [`kelpie_engine::review::tail`] cuts them.
//!
//! [`Testing`] is what a project's pass runs tests with: the runner, and the [`Gear`] that
//! says what one ticket's run reaches and which lease door it queues on.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::Arc;

use crabpot::Config;
use kelpie_engine::lease::run::{LIMIT_VAR, NO_LEASE, NOT_RUN, TIMED_OUT};
use kelpie_engine::lease::{Leases, SOCKET_VAR, SessionLease};
use kelpie_engine::review::{TestOutcome, TestRun, TestRunner, tail};
use kelpie_engine::sandbox::{self, Sandbox, check_project, worker};
use kelpie_engine::session::Harness;
use kelpie_engine::settings::CARGO_TEST;
use kelpie_engine::tools::Door;
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::worker::Paths;
use kelpie_engine::worktree::clone;

use crate::workers::{WorkerPlan, git_common_dir};

/// What a run that has no lease door says: the ceiling and the queue are the lease's
const NO_DOOR: &str = "the cargo-test lease has no socket here, so the test is not run";

/// A shell's number for a process ended by SIGTERM, which the tree sends on a stop
const TERMINATED: u8 = 143;

/// A shell's number for a process ended by SIGKILL, which the tree sends after its grace
const KILLED: u8 = 137;

/// What one ticket's test runs reach, and the lease they queue on
#[derive(Debug)]
pub struct Kit {
    /// The worker's own sandbox settings ([`kelpie_engine::sandbox::worker::policy`])
    pub reach: Config,

    /// The socket the run's lease is served on, held while the run is
    pub lease: Option<SessionLease>,
}

/// What builds one ticket's [`Kit`], from the plan its worker runs under
///
/// The app's is [`app_testing`]'s. A test's hands in a kit of settings that reach nothing.
pub type Gear = dyn Fn(&str, &WorkerPlan) -> Result<Kit, String> + Send + Sync;

/// What a project's pass runs a reviewer's test with
#[derive(Clone)]
pub struct Testing {
    /// What runs one command
    pub runner: Arc<dyn TestRunner>,

    /// What each ticket's run reaches
    pub gear: Arc<Gear>,
}

// The ports are trait objects and closures, which no derive can print
impl std::fmt::Debug for Testing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Testing").finish_non_exhaustive()
    }
}

/// How one leased command ended: its number, and what it wrote
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exit {
    /// What `kelpie lease run` exited with: the command's own, or one of its own
    /// ([`NO_LEASE`], [`NOT_RUN`], [`TIMED_OUT`])
    pub code: u8,

    /// What the command wrote, its standard output then its standard error
    pub output: Vec<u8>,
}

/// What runs a command under the lease, for a [`SandboxedTests`]
pub trait Launch: Send + Sync {
    /// Run `script` in `run`'s folder, under the lease served on `door`, within
    /// [`TestRun::ceiling`] of the grant
    ///
    /// # Errors
    ///
    /// The command could not be put in its sandbox or started. The text says why.
    fn launch(&self, run: &TestRun, door: &Door, script: &str) -> Result<Exit, String>;
}

/// The app's [`TestRunner`]: each run is a leased command in the worker's sandbox
pub struct SandboxedTests {
    launch: Box<dyn Launch>,
}

impl SandboxedTests {
    /// Runs through `kelpie lease run` in a sandbox, a child of `tree` (ADR 0001)
    ///
    /// `kelpie` is this binary, which answers `lease run` before it opens a window.
    #[must_use]
    pub fn new(tree: Arc<ProcessTree>) -> Self {
        let kelpie = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kelpie"));
        Self::through(Box::new(Spawned { tree, kelpie }))
    }

    /// Runs through `launch`, which a test stands in for the sandbox and the binary
    #[must_use]
    pub fn through(launch: Box<dyn Launch>) -> Self {
        Self { launch }
    }
}

impl std::fmt::Debug for SandboxedTests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SandboxedTests").finish_non_exhaustive()
    }
}

impl TestRunner for SandboxedTests {
    fn run(&self, run: &TestRun) -> TestOutcome {
        let Some(door) = &run.door else {
            return TestOutcome::NotRun {
                reason: NO_DOOR.to_owned(),
            };
        };
        match self.launch.launch(run, door, &script(&run.command)) {
            Ok(exit) => outcome_of(&exit),
            Err(reason) => TestOutcome::NotRun { reason },
        }
    }
}

/// What the shell is given: the command, with its errors on its output
///
/// Both streams are one, so a failure reads in the order it was written.
fn script(command: &str) -> String {
    format!("exec 2>&1; {command}")
}

/// What an [`Exit`] says of the test
///
/// The lease's own numbers are no result of the test: a run that could not have the lease
/// or start its command never ran, and one past its bound timed out. A run ended by SIGTERM or
/// SIGKILL is one kelpie's own stop ended (the tree sends those), so it never ran to a result
/// and counts for nothing: a fix that may well pass is not ruled on it. Any other number but 0
/// is the command failing, with the end of what it wrote.
fn outcome_of(exit: &Exit) -> TestOutcome {
    let written = tail(&exit.output);
    match exit.code {
        0 => TestOutcome::Passed,
        TIMED_OUT => TestOutcome::TimedOut,
        NO_LEASE => TestOutcome::NotRun {
            reason: format!("the cargo-test lease could not be had: {written}"),
        },
        NOT_RUN => TestOutcome::NotRun {
            reason: format!("the command could not start: {written}"),
        },
        TERMINATED | KILLED => TestOutcome::NotRun {
            reason: format!("kelpie stopped the test run: {written}"),
        },
        _ => TestOutcome::Failed { output: written },
    }
}

/// `kelpie lease run cargo-test -- sh -c <script>`, as the run's folder and door say
fn leased_command(kelpie: &std::path::Path, run: &TestRun, door: &Door, script: &str) -> Command {
    let mut command = Command::new(kelpie);
    command
        .args(["lease", "run", CARGO_TEST, "--", "sh", "-c"])
        .arg(script)
        .current_dir(&run.cwd)
        .env(SOCKET_VAR, door.to_string())
        .env(LIMIT_VAR, run.ceiling.as_secs().max(1).to_string());
    command
}

/// The number a shell gives for how `status` ended: the code, or 128 plus the signal
fn exit_number(status: ExitStatus) -> u8 {
    #[cfg(unix)]
    let number = {
        use std::os::unix::process::ExitStatusExt as _;
        status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(1))
    };
    #[cfg(not(unix))]
    let number = status.code().unwrap_or(1);
    u8::try_from(number).unwrap_or(1)
}

/// The launcher the app runs: a sandbox of the run's policy around `kelpie lease run`
struct Spawned {
    tree: Arc<ProcessTree>,
    kelpie: PathBuf,
}

impl Launch for Spawned {
    fn launch(&self, run: &TestRun, door: &Door, script: &str) -> Result<Exit, String> {
        let sandbox = Sandbox::start_policy(run.reach.clone())
            .map_err(|error| format!("the worker's sandbox did not start: {error}"))?;
        let command = leased_command(&self.kelpie, run, door, script);
        let mut wrapped = sandbox
            .command(&command)
            .map_err(|error| format!("the command could not be put in the sandbox: {error}"))?;
        // Through the tree, so a stop of kelpie ends it (ADR 0001). The sandbox is held until
        // the command ends.
        let output = self
            .tree
            .output(&mut wrapped)
            .map_err(|error| format!("the command could not be started: {error}"))?;
        let mut written = output.stdout;
        written.extend(output.stderr);
        Ok(Exit {
            code: exit_number(output.status),
            output: written,
        })
    }
}

/// What a project's pass runs tests with in the app: [`SandboxedTests`] on `tree`, and each
/// ticket's run in the worker's own sandbox policy, queued on a lease of its own
///
/// `leases` is where the project's leases are held, asked for on each run so a settings file
/// that cannot be read fails that run and not the project's gate.
///
/// The policy is [`worker::policy`] with the worker's worktree, git dirs and state folder, the
/// project's allowlist and reach, and the lease door of this run, so a test reaches what the
/// worker's own `cargo test` does. The harness it is composed for is a bare one: a test needs
/// the hosts of the project's allowlist and GitHub's, not the worker's model's.
#[must_use]
pub fn app_testing(
    tree: &Arc<ProcessTree>,
    paths: &Paths,
    data_dir: &Path,
    folder: &Path,
    leases: impl Fn() -> Result<Arc<Leases>, String> + Send + Sync + 'static,
) -> Testing {
    let (tree_for_gear, paths, data_dir, folder) = (
        Arc::clone(tree),
        paths.clone(),
        data_dir.to_owned(),
        folder.to_owned(),
    );
    Testing {
        runner: Arc::new(SandboxedTests::new(Arc::clone(tree))),
        gear: Arc::new(move |ticket, plan| {
            let sessions_root = data_dir.join("projects");
            let project = sandbox::Project {
                folder: &folder,
                network_allowlist: &plan.network_allowlist,
                reads: plan.reads,
                sessions_root: &sessions_root,
            };
            check_project(&project).map_err(|error| error.to_string())?;
            let (worktree, state_dir) = (paths.worktree(ticket), paths.state_dir(ticket));
            let git_dir = clone::git_dir_of(&worktree);
            let common = git_common_dir(&tree_for_gear, &folder)?;
            // Not the worker's own state folder: binding a lease socket removes what is at its
            // path, and the worker's live session holds that one
            let socket_dir = state_dir.join("tests");
            std::fs::create_dir_all(&socket_dir)
                .map_err(|error| format!("could not make {}: {error}", socket_dir.display()))?;
            let lease = leases()?
                .session(&socket_dir)
                .map_err(|error| format!("the cargo-test lease has no socket: {error}"))?;
            let reach = worker::policy(
                &Harness::new("kelpie-test"),
                &project,
                &sandbox::Worker {
                    worktree: &worktree,
                    git_dir: &git_dir,
                    git_common_dir: &common,
                    state_dir: &state_dir,
                    kelpie_home: &data_dir,
                    door: None,
                    lease: Some(lease.door()),
                },
            )
            .map_err(|error| format!("the worker's sandbox policy is not usable: {error}"))?;
            Ok(Kit {
                reach,
                lease: Some(lease),
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::time::Duration;

    use super::*;

    fn exit(code: u8, text: &str) -> Exit {
        Exit {
            code,
            output: text.as_bytes().to_vec(),
        }
    }

    fn run(door: Option<Door>) -> TestRun {
        TestRun {
            command: "cargo test --test flag".to_owned(),
            cwd: PathBuf::from("worktree"),
            reach: Config::default(),
            settings: PathBuf::from("settings.json"),
            door,
            ceiling: Duration::from_secs(90),
        }
    }

    #[test]
    fn an_exit_of_0_passed_and_any_other_but_the_leases_own_failed_with_the_end_of_its_output() {
        assert_eq!(outcome_of(&exit(0, "ok")), TestOutcome::Passed);
        assert_eq!(
            outcome_of(&exit(101, "boom")),
            TestOutcome::Failed {
                output: "boom".to_owned()
            }
        );
        let long = "x".repeat(10_000);
        let TestOutcome::Failed { output } = outcome_of(&exit(1, &long)) else {
            panic!("exit 1 is a failure");
        };
        assert!(
            output.len() <= kelpie_engine::review::OUTPUT_KEPT,
            "{}",
            output.len()
        );
    }

    #[test]
    fn the_leases_own_exits_are_no_result_of_the_test() {
        assert_eq!(outcome_of(&exit(TIMED_OUT, "")), TestOutcome::TimedOut);
        for code in [NO_LEASE, NOT_RUN] {
            assert!(
                matches!(outcome_of(&exit(code, "why")), TestOutcome::NotRun { reason } if reason.contains("why")),
                "exit {code} never ran the test"
            );
        }
    }

    #[test]
    fn a_run_kelpie_stopped_is_not_a_failed_test() {
        for code in [TERMINATED, KILLED] {
            assert!(
                matches!(outcome_of(&exit(code, "cut")), TestOutcome::NotRun { reason } if reason.contains("stopped")),
                "exit {code} is the tree's stop, which rules nothing on the fix"
            );
        }
        assert!(
            matches!(outcome_of(&exit(139, "segv")), TestOutcome::Failed { .. }),
            "a test that crashes itself is still a failure"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_child_ended_by_the_trees_signals_has_the_numbers_a_stop_is_read_by() {
        let ended_by = |signal: &str| {
            let status = std::process::Command::new("sh")
                .args(["-c", &format!("kill -{signal} $$")])
                .status()
                .unwrap();
            exit_number(status)
        };
        assert_eq!(ended_by("TERM"), TERMINATED);
        assert_eq!(ended_by("KILL"), KILLED);
    }

    #[test]
    fn a_run_with_no_door_is_not_run_because_it_could_not_queue() {
        struct Never;
        impl Launch for Never {
            fn launch(&self, _: &TestRun, _: &Door, _: &str) -> Result<Exit, String> {
                panic!("a run with no door is never launched");
            }
        }
        let outcome = SandboxedTests::through(Box::new(Never)).run(&run(None));
        assert!(
            matches!(outcome, TestOutcome::NotRun { ref reason } if reason.contains("no socket")),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_launch_that_fails_is_a_run_that_never_ran() {
        struct Broken;
        impl Launch for Broken {
            fn launch(&self, _: &TestRun, _: &Door, _: &str) -> Result<Exit, String> {
                Err("no bwrap".to_owned())
            }
        }
        let door: Door = Door::parse("/tmp/kelpie-test.sock").expect("a door");
        let outcome = SandboxedTests::through(Box::new(Broken)).run(&run(Some(door)));
        assert_eq!(
            outcome,
            TestOutcome::NotRun {
                reason: "no bwrap".to_owned()
            }
        );
    }

    #[test]
    fn the_command_is_kelpie_lease_run_cargo_test_in_the_worktree_bounded_by_the_ceiling() {
        let door: Door = Door::parse("/tmp/kelpie-test.sock").expect("a door");
        let command = leased_command(
            Path::new("kelpie"),
            &run(Some(door.clone())),
            &door,
            "exec 2>&1; cargo test --test flag",
        );

        let args: Vec<OsString> = command.get_args().map(OsString::from).collect();
        assert_eq!(
            args,
            [
                "lease",
                "run",
                CARGO_TEST,
                "--",
                "sh",
                "-c",
                "exec 2>&1; cargo test --test flag"
            ]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("worktree")));
        let env = |name: &str| {
            command
                .get_envs()
                .find(|(key, _)| *key == name)
                .and_then(|(_, value)| value.map(OsString::from))
        };
        assert_eq!(env(SOCKET_VAR), Some(door.to_string().into()));
        assert_eq!(env(LIMIT_VAR), Some("90".into()), "the ceiling, in seconds");
    }

    #[test]
    fn the_shell_is_given_the_command_with_its_errors_on_its_output() {
        assert_eq!(script("make check"), "exec 2>&1; make check");
    }
}
