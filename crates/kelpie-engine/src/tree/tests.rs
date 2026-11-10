use std::fs;
use std::io::{BufRead as _, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::*;

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(10);

const GRACE: Duration = Duration::from_secs(5);

/// A folder of its own for one test's run file
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-tree-{}", RunId::new().unwrap()));
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

fn sh(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]).stdout(Stdio::piped());
    command
}

/// Read the `pid <n>` lines a script prints up to the line `ready`, and keep
/// the reader
fn ready(stdout: ChildStdout) -> (BufReader<ChildStdout>, Vec<u32>) {
    let mut reader = BufReader::new(stdout);
    let mut pids = Vec::new();
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).unwrap();
        assert_ne!(read, 0, "the script ended before it was ready");
        match line.trim().strip_prefix("pid ") {
            Some(pid) => pids.push(pid.parse().unwrap()),
            None if line.trim() == "ready" => return (reader, pids),
            None => {}
        }
    }
}

/// Not a test: a process the tests start, because it has an environment the
/// sweep may read where a system binary's is hidden
#[test]
#[ignore = "a helper process that other tests start"]
fn sleep_a_minute() {
    println!("pid {}", std::process::id());
    println!("ready");
    std::thread::sleep(Duration::from_secs(60));
}

/// Wait until none of `pids` is running
///
/// Pipes would do, but on macOS a child another test starts at the same moment
/// can inherit a pipe's write end and hold it open.
fn assert_gone(pids: &[u32]) {
    let deadline = Instant::now() + WAIT;
    while let Some(pid) = pids
        .iter()
        .find(|pid| sweep::read(**pid).is_some_and(|reading| !reading.zombie))
    {
        assert!(Instant::now() < deadline, "pid {pid} still runs");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Kills a process that is not under a tree, even when a test fails
struct Stranger(std::process::Child);

impl Drop for Stranger {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_child_carries_the_run_in_its_environment() {
    let dir = Dir::new();
    let run = RunId::new().unwrap();
    let tree = ProcessTree::new(run, dir.run_file()).unwrap();

    // A value the command already had is replaced
    let mut child = tree
        .spawn(sh("echo $KELPIE_RUN").env(ENV, "not-this-run"))
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();

    assert_eq!(line.trim(), run.to_string());
    child.wait().unwrap();
}

#[test]
fn stop_all_ends_a_child_and_its_grandchild() {
    let dir = Dir::new();
    let tree = ProcessTree::new(RunId::new().unwrap(), dir.run_file()).unwrap();
    let mut child = tree
        .spawn(&mut sh(r#"sleep 60 & echo "pid $!"; echo ready; sleep 60"#))
        .unwrap();
    let (_stdout, mut pids) = ready(child.stdout.take().unwrap());
    pids.push(child.id());

    tree.stop_all(GRACE).unwrap();

    assert_gone(&pids);
    assert!(!child.wait().unwrap().success());
}

#[test]
fn one_child_stops_without_stopping_the_tree() {
    let dir = Dir::new();
    let run = RunId::new().unwrap();
    let tree = ProcessTree::new(run, dir.run_file()).unwrap();
    let mut stopped = tree
        .spawn(&mut sh(r#"sleep 60 & echo "pid $!"; echo ready; sleep 60"#))
        .unwrap();
    let (_stdout, mut pids) = ready(stopped.stdout.take().unwrap());
    pids.push(stopped.id());
    let other = tree.spawn(&mut sh("sleep 60")).unwrap();
    assert_eq!(
        RunFile::read(&dir.run_file()).unwrap().groups.len(),
        2,
        "both groups are listed before either is stopped"
    );

    stopped.stop(GRACE).unwrap();

    assert_gone(&pids);
    assert!(!stopped.wait().unwrap().success());
    let groups = RunFile::read(&dir.run_file()).unwrap().groups;
    assert_eq!(
        groups.iter().map(|group| group.pid).collect::<Vec<_>>(),
        [other.id()]
    );
    // The tree is open still, so a session can start its next process
    tree.spawn(&mut sh("true")).unwrap().wait().unwrap();
    tree.stop_all(GRACE).unwrap();
}

#[test]
fn a_grandchild_that_leaves_the_group_is_still_swept_by_its_mark() {
    // `perl`, since `setsid(1)` is absent on macOS
    let script = r#"perl -MPOSIX -e 'POSIX::setsid(); exec @ARGV' "$0" --ignored --exact --nocapture tree::tests::sleep_a_minute & sleep 60"#;
    let dir = Dir::new();
    let tree = ProcessTree::new(RunId::new().unwrap(), dir.run_file()).unwrap();
    let mut command = sh(script);
    command.arg(std::env::current_exe().unwrap());
    let mut child = tree.spawn(&mut command).unwrap();
    let (_stdout, mut pids) = ready(child.stdout.take().unwrap());
    pids.push(child.id());

    tree.stop_all(GRACE).unwrap();

    assert_gone(&pids);
}

#[test]
fn leftovers_of_a_dead_run_are_stopped_at_launch() {
    let dir = Dir::new();
    let tree = ProcessTree::new(RunId::new().unwrap(), dir.run_file()).unwrap();
    let mut child = tree
        .spawn(&mut sh(r#"sleep 60 & echo "pid $!"; echo ready; sleep 60"#))
        .unwrap();
    let (_stdout, mut pids) = ready(child.stdout.take().unwrap());
    pids.push(child.id());
    // The app dies without a word: nothing stops the tree or reaps its children
    std::mem::forget(child);
    std::mem::forget(tree);

    sweep_leftovers(&dir.run_file()).unwrap();

    assert_gone(&pids);
}

#[test]
fn a_record_in_hand_is_swept_and_not_the_file_that_path_holds_now() {
    let dir = Dir::new();
    let dead = RunId::new().unwrap();
    let fresh = RunId::new().unwrap();

    // The run that died: its record is in hand, and its group is still running
    let old = ProcessTree::new(dead, dir.run_file()).unwrap();
    let leftover = old.spawn(Command::new("sleep").arg("60")).unwrap().id();
    let record = RunFile::read(&dir.run_file()).unwrap();

    // The next launch takes the same path and starts a group of its own, which
    // is what a reaper of the dead run can race with
    let live = ProcessTree::new(fresh, dir.run_file()).unwrap();
    let running = live.spawn(Command::new("sleep").arg("60")).unwrap().id();

    sweep_record(&record).unwrap();

    assert_gone(&[leftover]);
    assert!(
        sweep::read(running).is_some_and(|reading| !reading.zombie),
        "the newer launch's group was stopped by a sweep of the run before it"
    );
    live.stop_all(GRACE).unwrap();
}

/// A process with no mark, alone in its group, and a run file that names it
/// with `started` moved by `skew`
fn recorded_stranger(dir: &Dir, skew: u64) -> Stranger {
    use std::os::unix::process::CommandExt as _;

    let child = Command::new("sleep")
        .arg("60")
        .process_group(0)
        .spawn()
        .unwrap();
    let started = sweep::read(child.id()).unwrap().started;
    RunFile {
        run: RunId::new().unwrap().to_string(),
        groups: vec![Group {
            pid: child.id(),
            started: started + skew,
        }],
    }
    .write(&dir.run_file())
    .unwrap();
    Stranger(child)
}

#[test]
fn a_recorded_group_is_stopped_by_its_leader_alone() {
    let dir = Dir::new();
    let mut leader = recorded_stranger(&dir, 0);

    sweep_leftovers(&dir.run_file()).unwrap();

    assert!(
        leader.0.wait().unwrap().code().is_none(),
        "ended by a signal"
    );
}

#[test]
fn a_recorded_pid_with_another_start_time_is_left_alone() {
    let dir = Dir::new();
    let mut stranger = recorded_stranger(&dir, 1000);

    sweep_leftovers(&dir.run_file()).unwrap();

    assert_eq!(stranger.0.try_wait().unwrap(), None);
}

#[test]
fn the_run_file_lists_live_groups_only() {
    let dir = Dir::new();
    let run = RunId::new().unwrap();
    let tree = ProcessTree::new(run, dir.run_file()).unwrap();
    let listed = || {
        let file = RunFile::read(&dir.run_file()).unwrap();
        assert_eq!(file.run, run.to_string());
        file.groups
            .iter()
            .map(|group| group.pid)
            .collect::<Vec<_>>()
    };
    assert_eq!(listed(), Vec::<u32>::new());

    // `cat` ends when its input does
    let mut first = tree
        .spawn(Command::new("cat").stdin(Stdio::piped()))
        .unwrap();
    let second = tree.spawn(Command::new("sleep").arg("60")).unwrap();
    assert_eq!(listed(), [first.id(), second.id()]);

    drop(first.stdin.take());
    first.wait().unwrap();
    assert_eq!(listed(), [second.id()]);

    tree.stop_all(GRACE).unwrap();
    assert_eq!(listed(), Vec::<u32>::new());
}

/// Kills a group when a test ends, so a failed one leaves no loop running
struct KillGroup(u32);

impl Drop for KillGroup {
    fn drop(&mut self) {
        let _ = shep_tree::group::signal_group(self.0, Signal::Kill);
    }
}

#[test]
fn a_group_member_that_ignores_term_is_killed_after_its_leader_is_gone() {
    use std::os::unix::process::CommandExt as _;

    // The member has no mark and ends only on `SIGKILL`; the leader dies on `SIGTERM`
    let script =
        r#"sh -c 'trap "" TERM; echo "pid $$"; echo ready; while :; do sleep 1; done' & wait"#;
    let dir = Dir::new();
    let mut leader = sh(script).process_group(0).spawn().unwrap();
    let _cleanup = KillGroup(leader.id());
    let (_stdout, pids) = ready(leader.stdout.take().unwrap());
    RunFile {
        run: RunId::new().unwrap().to_string(),
        groups: vec![Group {
            pid: leader.id(),
            started: sweep::read(leader.id()).unwrap().started,
        }],
    }
    .write(&dir.run_file())
    .unwrap();
    // Reaped as soon as it dies, as init would
    std::thread::spawn(move || leader.wait());

    sweep_leftovers(&dir.run_file()).unwrap();

    assert_gone(&pids);
}

/// A stand-in reaper: it blocks on its standard input and exits when it ends
///
/// The reaper binary is another crate's. The engine's half of the deal is what
/// this tests: it starts the path it is given, holds a pipe to it, and leaves
/// it unmarked.
fn stand_in_reaper(dir: &Dir) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let path = dir.0.join("reaper");
    fs::write(&path, "#!/bin/sh\ncat >/dev/null\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn the_reaper_is_not_marked_with_the_run() {
    let dir = Dir::new();
    let run = RunId::new().unwrap();
    let tree = ProcessTree::with_reaper(&stand_in_reaper(&dir), run, dir.run_file()).unwrap();
    let reaper = tree.reaper_id().unwrap();
    let child = tree.spawn(Command::new("sleep").arg("60")).unwrap();

    let file = RunFile::read(&dir.run_file()).unwrap();
    assert_eq!(
        file.groups
            .iter()
            .map(|group| group.pid)
            .collect::<Vec<_>>(),
        [child.id()],
        "the run file records the marked child, and not the reaper"
    );

    tree.stop_all(GRACE).unwrap();

    assert_gone(&[child.id()]);
    assert!(
        sweep::read(reaper).is_some_and(|reading| !reading.zombie),
        "the sweep of the run's mark left the unmarked reaper running"
    );

    drop(tree);
    assert_gone(&[reaper]);
}

#[test]
fn nothing_starts_after_stop_all() {
    let dir = Dir::new();
    let tree = ProcessTree::new(RunId::new().unwrap(), dir.run_file()).unwrap();
    tree.stop_all(GRACE).unwrap();

    let refused = tree.spawn(Command::new("sleep").arg("60"));

    assert!(refused.is_err());
}

#[test]
fn an_exit_noticed_while_another_handle_drops_leaves_the_run_file() {
    let dir = Dir::new();
    let tree = ProcessTree::new(RunId::new().unwrap(), dir.run_file()).unwrap();
    let mut first = tree
        .spawn(Command::new("cat").stdin(Stdio::piped()))
        .unwrap();
    let second = tree.spawn(Command::new("sleep").arg("60")).unwrap();
    drop(first.stdin.take());
    assert_gone(&[first.id()]);
    // The process table stops listing a process a moment before `wait` can reap it
    std::thread::sleep(Duration::from_millis(200));

    drop(second);

    let file = RunFile::read(&dir.run_file()).unwrap();
    assert_eq!(file.groups.len(), 1, "only the sleep is listed: {file:?}");
    tree.stop_all(GRACE).unwrap();
}

#[test]
fn stopping_one_child_does_not_hold_the_tree_for_its_grace() {
    let dir = Dir::new();
    let tree = ProcessTree::new(RunId::new().unwrap(), dir.run_file()).unwrap();
    let mut stubborn = tree
        .spawn(&mut sh(
            "trap '' TERM; echo ready; while :; do sleep 1; done",
        ))
        .unwrap();
    let (reader, _) = ready(stubborn.stdout.take().unwrap());
    let grace = Duration::from_secs(3);
    let stopping = std::thread::spawn(move || {
        let _reader = reader;
        stubborn.stop(grace).unwrap()
    });
    // The term is out and ignored, so the stop is now waiting out its grace
    std::thread::sleep(Duration::from_millis(300));

    let began = Instant::now();
    let mut other = tree.spawn(Command::new("sleep").arg("60")).unwrap();
    assert!(other.try_wait().unwrap().is_none());
    let waited = began.elapsed();

    assert!(
        waited < Duration::from_secs(1),
        "a spawn and a try_wait waited {waited:?} behind another child's stop"
    );
    assert!(!stopping.join().unwrap().success());
    tree.stop_all(GRACE).unwrap();
}
