//! A real command inside crabpot, on macOS and Linux (ADR 0005)
//!
//! This file is its own `main`, as the app's is: crabpot runs the host binary
//! again as its helper on Linux, so [`run_from_args`] comes first. A test of
//! the same name in a `libtest` binary could not run there.

#![cfg(unix)]
#![allow(
    missing_docs,
    reason = "a test binary with a main of its own, which no one links"
)]

use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use kelpie_engine::run::RunId;
use kelpie_engine::sandbox::{Project, Sandbox, run_from_args};
use kelpie_engine::session::Harness;
use kelpie_engine::tree::{ProcessTree, sweep_marked};

/// The argument that makes this binary the sandboxed process the sweep looks for
const SLEEP: &str = "--sleep-in-the-sandbox";

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(20);

fn main() {
    run_from_args();
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some(SLEEP) {
        sleep_in_the_sandbox(args.next().map(PathBuf::from));
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let selection = Selection::of(&args);
    let mut failed = false;
    let (mut ran, mut filtered) = (0_usize, 0_usize);
    for (name, test) in TESTS {
        if !selection.runs(name) {
            filtered += 1;
            continue;
        }
        ran += 1;
        let skipped = SKIPPED.load(Ordering::SeqCst);
        match panic::catch_unwind(AssertUnwindSafe(test)) {
            Ok(()) if SKIPPED.load(Ordering::SeqCst) > skipped => {
                println!("test {name} ... SKIPPED");
            }
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed = true;
            }
        }
    }
    println!("ran {ran} tests, {filtered} filtered out");
    std::process::exit(i32::from(failed));
}

/// Which tests an invocation names, read as `libtest` reads its arguments
///
/// A bare word is a name filter, `--skip` names tests to leave out and `--exact`
/// makes a filter match a whole name. The other options `libtest` takes are
/// ignored, and those that take a value (`--test-threads 1`) keep it from
/// being read as a filter.
struct Selection {
    filters: Vec<String>,
    skips: Vec<String>,
    exact: bool,
}

/// The options `libtest` takes a value for, as `--name value`
const VALUED: [&str; 7] = [
    "--test-threads",
    "--skip",
    "--format",
    "--color",
    "--logfile",
    "--report-time",
    "--shuffle-seed",
];

impl Selection {
    fn of(args: &[String]) -> Self {
        let mut selection = Self {
            filters: Vec::new(),
            skips: Vec::new(),
            exact: false,
        };
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let (option, value) = match arg.split_once('=') {
                Some((option, value)) if arg.starts_with("--") => (option, Some(value.to_owned())),
                _ => (arg.as_str(), None),
            };
            if option == "--exact" {
                selection.exact = true;
            } else if VALUED.contains(&option) {
                let value = value.or_else(|| args.next().cloned());
                if let (true, Some(value)) = (option == "--skip", value) {
                    selection.skips.push(value);
                }
            } else if !arg.starts_with('-') {
                selection.filters.push(arg.clone());
            }
        }
        selection
    }

    fn matches(&self, name: &str, word: &str) -> bool {
        if self.exact {
            name == word
        } else {
            name.contains(word)
        }
    }

    fn runs(&self, name: &str) -> bool {
        (self.filters.is_empty() || self.filters.iter().any(|word| self.matches(name, word)))
            && !self.skips.iter().any(|word| self.matches(name, word))
    }
}

const TESTS: [(&str, fn()); 5] = [
    (
        "the_value_of_a_libtest_option_is_not_a_filter",
        the_value_of_a_libtest_option_is_not_a_filter,
    ),
    (
        "a_kernel_without_user_namespaces_is_told_from_a_command_that_failed",
        a_kernel_without_user_namespaces_is_told_from_a_command_that_failed,
    ),
    (
        "a_write_outside_the_state_dir_is_refused",
        a_write_outside_the_state_dir_is_refused,
    ),
    (
        "a_sandboxed_command_reads_the_project_and_writes_the_state_dir",
        a_sandboxed_command_reads_the_project_and_writes_the_state_dir,
    ),
    (
        "the_sweep_finds_a_sandboxed_process",
        the_sweep_finds_a_sandboxed_process,
    ),
];

/// Says it is up by writing a file, then waits to be stopped
fn sleep_in_the_sandbox(ready: Option<PathBuf>) {
    if let Some(ready) = ready {
        fs::write(ready, std::process::id().to_string()).expect("the state folder is writable");
    }
    thread::sleep(Duration::from_secs(60));
}

/// A folder of its own under the target's temp folder, with a project, a state
/// folder and a folder outside both, removed when it drops
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(test: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("sandbox-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for folder in ["project", "state", "outside"] {
            fs::create_dir_all(root.join(folder)).expect("the scratch folders are made");
        }
        // The path as the OS names it, since a sandbox compares real paths
        let root = fs::canonicalize(root).expect("the scratch folder is there");
        Self { root }
    }

    fn project(&self) -> PathBuf {
        self.root.join("project")
    }

    fn state(&self) -> PathBuf {
        self.root.join("state")
    }

    fn outside(&self) -> PathBuf {
        self.root.join("outside")
    }

    /// A sandbox for a session in the project, whose harness is `program`
    fn sandbox(&self, program: &Path) -> Sandbox {
        let mut harness = Harness::new(program);
        // The harness's own install is read, as a real one's is
        harness.installs = program
            .parent()
            .map(Path::to_path_buf)
            .into_iter()
            .collect();
        let project = Project {
            folder: &self.project(),
            network_allowlist: &[],
            read_anywhere: false,
        };
        Sandbox::start(&harness, &project, &self.state())
            .unwrap_or_else(|error| panic!("the sandbox did not start: {error}"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// `script` run by `sh` inside the sandbox
fn sh(sandbox: &Sandbox, script: &str) -> Output {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    sandbox
        .command(&command)
        .expect("the command goes in the sandbox")
        .output()
        .expect("the sandbox runs sh")
}

/// Set to skip, not fail, the tests on a host that cannot run a sandbox
///
/// Off by default, so a runner that cannot run one is red and not a false
/// green. A skipped test is printed as `SKIPPED`, never as `ok`.
const SKIP: &str = "KELPIE_SKIP_SANDBOX_TESTS";

/// How many tests have skipped themselves, which `main` reads to print each
/// test's verdict
static SKIPPED: AtomicUsize = AtomicUsize::new(0);

/// What bwrap prints first when the kernel gives it no user namespace, as the
/// runner's does while `kernel.apparmor_restrict_unprivileged_userns` is on
fn host_refuses_sandboxes(stderr: &str) -> bool {
    cfg!(target_os = "linux") && stderr.starts_with("bwrap:")
}

/// A sandbox for `program` that is known to run commands, or `None` when this
/// host cannot run one at all.
///
/// A refusal test that ran in a sandbox which never started would pass, as a
/// command that did not run leaves no file and exits non-zero. So each test
/// asks for a sandbox that has run a command first, and a host that cannot
/// fails the test. `KELPIE_SKIP_SANDBOX_TESTS` skips it instead, shown as
/// `SKIPPED`, for a host known not to run one.
fn working_sandbox(scratch: &Scratch, test: &str, program: &Path) -> Option<Sandbox> {
    let sandbox = scratch.sandbox(program);
    let probe = sh(&sandbox, "true");
    if probe.status.success() {
        return Some(sandbox);
    }
    let stderr = String::from_utf8_lossy(&probe.stderr);
    assert!(
        host_refuses_sandboxes(&stderr) && std::env::var_os(SKIP).is_some(),
        "`true` did not run in the sandbox: {probe:?}\nThis host cannot run a sandbox. Set {SKIP} to skip these tests."
    );
    SKIPPED.fetch_add(1, Ordering::SeqCst);
    println!(
        "SKIPPED {test}: this host cannot run a sandbox ({})",
        stderr.trim()
    );
    None
}

fn a_write_outside_the_state_dir_is_refused() {
    let scratch = Scratch::new("refused");
    let Some(sandbox) = working_sandbox(
        &scratch,
        "a_write_outside_the_state_dir_is_refused",
        Path::new("/bin/sh"),
    ) else {
        return;
    };
    // The control: a write the policy allows lands, so the refusals below are
    // the policy's and not a sandbox that ran nothing
    let allowed = scratch.state().join("control");
    let control = sh(&sandbox, &format!("echo yes > {}", allowed.display()));
    assert!(control.status.success(), "{control:?}");
    assert!(allowed.exists(), "a write to the state folder did not land");
    let outside = scratch.outside().join("nope");
    let project = scratch.project().join("nope");

    let to_outside = sh(&sandbox, &format!("echo no > {}", outside.display()));
    let to_project = sh(&sandbox, &format!("echo no > {}", project.display()));

    for (name, run, path) in [
        ("outside", to_outside, outside),
        ("project", to_project, project),
    ] {
        assert!(
            !run.status.success(),
            "a write to the {name} folder was not refused: {run:?}"
        );
        assert!(!path.exists(), "a write to the {name} folder landed");
    }
}

fn a_sandboxed_command_reads_the_project_and_writes_the_state_dir() {
    let scratch = Scratch::new("allowed");
    fs::write(scratch.project().join("notes.md"), "from the project").unwrap();
    let Some(sandbox) = working_sandbox(
        &scratch,
        "a_sandboxed_command_reads_the_project_and_writes_the_state_dir",
        Path::new("/bin/sh"),
    ) else {
        return;
    };
    let written = scratch.state().join("claude").join("out");
    fs::create_dir_all(written.parent().unwrap()).unwrap();

    let run = sh(
        &sandbox,
        &format!(
            "cat {} > {}",
            scratch.project().join("notes.md").display(),
            written.display()
        ),
    );

    assert!(run.status.success(), "{run:?}");
    assert_eq!(fs::read_to_string(&written).unwrap(), "from the project");
}

fn the_sweep_finds_a_sandboxed_process() {
    let scratch = Scratch::new("sweep");
    let exe = std::env::current_exe().unwrap();
    let Some(sandbox) = working_sandbox(&scratch, "the_sweep_finds_a_sandboxed_process", &exe)
    else {
        return;
    };
    let ready = scratch.state().join("ready");
    let run = RunId::new().unwrap();
    let tree = ProcessTree::new(run, scratch.root.join("run.json")).unwrap();
    let mut command = Command::new(&exe);
    command.arg(SLEEP).arg(&ready);
    let mut wrapped = sandbox.command(&command).unwrap();

    // The tree spawns it as it spawns anything else, so it carries the run's mark
    let mut child = tree.spawn(&mut wrapped).unwrap();
    let waited = Instant::now();
    while !ready.exists() {
        assert!(
            waited.elapsed() < WAIT,
            "the sandboxed process never said it was up"
        );
        thread::sleep(Duration::from_millis(20));
    }

    let report = sweep_marked(&run);

    assert!(
        report.termed.contains(&child.id()),
        "the sweep for the run's mark left the sandboxed pid {} out: {report:?}",
        child.id()
    );
    let status = child.wait().unwrap();
    assert!(
        status.code().is_none(),
        "ended by {status:?}, not by a signal"
    );
}

fn a_kernel_without_user_namespaces_is_told_from_a_command_that_failed() {
    // The runner's own words, from the log of the run that went red
    let refused = "bwrap: No permissions to create new namespace, likely because the kernel does not allow non-privileged user namespaces.";

    assert_eq!(host_refuses_sandboxes(refused), cfg!(target_os = "linux"));
    assert!(!host_refuses_sandboxes("sh: true: not found"));
    assert!(!host_refuses_sandboxes(""));
}

fn the_value_of_a_libtest_option_is_not_a_filter() {
    let words =
        |args: &[&str]| -> Vec<String> { args.iter().map(|arg| (*arg).to_owned()).collect() };
    let name = "the_sweep_finds_a_sandboxed_process";

    let threads = Selection::of(&words(&["--test-threads", "1", "--nocapture"]));
    assert!(threads.runs(name), "a thread count filtered the tests out");

    let skip = Selection::of(&words(&["--skip", "sweep"]));
    assert!(!skip.runs(name), "--skip sweep ran the sweep");
    assert!(skip.runs("a_write_outside_the_state_dir_is_refused"));

    let skip_equals = Selection::of(&words(&["--skip=sweep", "--format=terse"]));
    assert!(!skip_equals.runs(name));

    let filter = Selection::of(&words(&["--test-threads", "1", "refused"]));
    assert!(filter.runs("a_write_outside_the_state_dir_is_refused"));
    assert!(!filter.runs(name));

    let exact = Selection::of(&words(&["--exact", "refused"]));
    assert!(!exact.runs("a_write_outside_the_state_dir_is_refused"));
}
