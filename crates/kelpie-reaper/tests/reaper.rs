//! The reaper binary, started the way the engine starts it: it holds a pipe,
//! and the tests close it or kill whoever holds it.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use kelpie_engine::run::RunId;
use kelpie_engine::tree::{ProcessTree, Reaper};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

/// How long a test waits for the reaper to stop something
const WAIT: Duration = Duration::from_secs(5);

/// How often a test looks at the process table while it waits
const POLL: Duration = Duration::from_millis(20);

/// The binary this package builds
fn reaper() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_kelpie-reaper"))
}

/// A folder of its own for one test's run file
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-reaper-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn run_file(&self) -> PathBuf {
        self.0.join("run.json")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Start the reaper with a piped stdin the caller holds
fn start(run: &RunId, run_file: &Path) -> Child {
    Command::new(reaper())
        .args([
            "--run",
            &run.to_string(),
            "--run-file",
            &run_file.to_string_lossy(),
        ])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap()
}

/// Whether the process table still lists `pid` as running
fn running(pid: u32) -> bool {
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
    system.process(pid).is_some_and(|process| {
        !matches!(
            process.status(),
            ProcessStatus::Zombie | ProcessStatus::Dead
        )
    })
}

/// Wait until none of `pids` is running, and fail the test if one is
fn assert_gone(pids: &[u32]) {
    let deadline = Instant::now() + WAIT;
    while let Some(pid) = pids.iter().copied().find(|pid| running(*pid)) {
        assert!(Instant::now() < deadline, "pid {pid} still runs");
        std::thread::sleep(POLL);
    }
}

/// Wait for `child` to exit, and fail the test if it never does
fn assert_exits(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(Instant::now() < deadline, "pid {} still runs", child.id());
        std::thread::sleep(POLL);
    }
}

#[test]
fn closing_the_pipe_stops_every_marked_child() {
    let dir = Dir::new();
    let run = RunId::new().unwrap();
    let tree = ProcessTree::with_reaper(reaper(), run, dir.run_file()).unwrap();
    let child = tree.spawn(Command::new("sleep").arg("60")).unwrap();
    let marked = child.id();

    drop(tree);

    assert_gone(&[marked]);
}

#[test]
fn a_parent_killed_with_sigkill_leaves_nothing_running() {
    let dir = Dir::new();
    let run = RunId::new().unwrap();
    let tree = ProcessTree::new(run, dir.run_file()).unwrap();
    let child = tree.spawn(Command::new("sleep").arg("60")).unwrap();
    let marked = child.id();

    // A helper holds the pipe the way kelpie does, and dies the way kelpie can
    let mut reaper = start(&run, &dir.run_file());
    let mut holder = Command::new("sleep")
        .arg("60")
        .stdin(Stdio::from(reaper.stdin.take().unwrap()))
        .spawn()
        .unwrap();
    let held = holder.id();
    holder.kill().unwrap();
    holder.wait().unwrap();

    assert_gone(&[marked]);
    assert!(assert_exits(&mut reaper).success());
    assert!(!running(held));
}

#[test]
fn the_reaper_exits_when_there_is_nothing_to_stop() {
    let dir = Dir::new();
    let run = RunId::new().unwrap();
    let tree = ProcessTree::new(run, dir.run_file()).unwrap();
    let reaper = Reaper::start(reaper(), &run, &dir.run_file()).unwrap();

    drop(tree);
    let status = reaper.quit().unwrap();

    assert!(status.success());
}
