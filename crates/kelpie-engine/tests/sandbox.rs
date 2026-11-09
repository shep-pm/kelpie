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
    let filter = std::env::args().skip(1).find(|arg| !arg.starts_with('-'));
    let mut failed = false;
    for (name, test) in TESTS {
        if filter
            .as_ref()
            .is_some_and(|filter| !name.contains(filter.as_str()))
        {
            continue;
        }
        match panic::catch_unwind(AssertUnwindSafe(test)) {
            Ok(()) => println!("test {name} ... ok"),
            Err(_) => {
                println!("test {name} ... FAILED");
                failed = true;
            }
        }
    }
    std::process::exit(i32::from(failed));
}

const TESTS: [(&str, fn()); 3] = [
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

fn a_write_outside_the_state_dir_is_refused() {
    let scratch = Scratch::new("refused");
    let sandbox = scratch.sandbox(Path::new("/bin/sh"));
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
    let sandbox = scratch.sandbox(Path::new("/bin/sh"));
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
    let sandbox = scratch.sandbox(&exe);
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
