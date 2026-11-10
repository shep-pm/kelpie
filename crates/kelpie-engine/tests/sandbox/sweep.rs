//! The sweep finds a process kelpie started inside a sandbox
//!
//! A child of `tests/sandbox.rs`, which is its own `main`: the sandboxed process
//! is this binary run again with [`SLEEP`], so it carries the run's mark the sweep
//! looks for (crabpot's Linux helper is this binary run again too).

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use kelpie_engine::run::RunId;
use kelpie_engine::sandbox::Reads;
use kelpie_engine::tree::{ProcessTree, sweep_marked};

use super::WAIT;
use super::host::working_sandbox;
use super::support::Scratch;

/// The argument that makes this binary the sandboxed process the sweep looks for
pub(super) const SLEEP: &str = "--sleep-in-the-sandbox";

/// Says it is up by writing a file, then waits to be stopped
pub(super) fn sleep_in_the_sandbox(ready: Option<PathBuf>) {
    if let Some(ready) = ready {
        fs::write(ready, std::process::id().to_string()).expect("the state folder is writable");
    }
    thread::sleep(Duration::from_secs(60));
}

pub(super) fn the_sweep_finds_a_sandboxed_process() {
    let scratch = Scratch::new("sweep");
    let exe = std::env::current_exe().unwrap();
    let Some(sandbox) = working_sandbox(
        &scratch,
        "the_sweep_finds_a_sandboxed_process",
        &exe,
        Reads::Project,
    ) else {
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
