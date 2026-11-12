//! A leased run: the numbers it exits with, and the lease it gives back

use std::fs;
use std::path::Path;
use std::time::Instant;

use super::*;
use crate::lease::{Leases, SessionLease};
use crate::settings::{CARGO_TEST, ConfigDir, Settings};
use crate::tree::sweep;

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(10);

#[test]
fn the_first_failed_knock_says_how_long_it_will_ask() {
    assert_eq!(
        asking_again("cannot reach kelpie", PATIENCE),
        "cannot reach kelpie: asking again every 1s, up to 60 times while kelpie comes back, then exiting 75"
    );
}

#[test]
fn a_run_that_names_no_command_is_usage() {
    for args in [
        &["run", "cargo-test", "--"][..],
        &["run", "cargo-test"][..],
        &["run"][..],
        &["status"][..],
        &[][..],
    ] {
        assert_eq!(dispatch(&os(args)), 2, "{args:?}");
    }
}

#[test]
fn the_bound_is_what_the_environment_names_and_never_more_than_the_run_limit() {
    let var = |text: &str| Some(OsString::from(text));
    assert_eq!(limit_from(None), RUN_LIMIT);
    assert_eq!(limit_from(var("30").as_deref()), Duration::from_secs(30));
    assert_eq!(
        limit_from(var("999999").as_deref()),
        RUN_LIMIT,
        "a command cannot ask for a longer hold than the run's own limit"
    );
    for unreadable in ["", "soon", "-5", "0"] {
        assert_eq!(
            limit_from(var(unreadable).as_deref()),
            RUN_LIMIT,
            "{unreadable:?}"
        );
    }
}

fn os(words: &[&str]) -> Vec<OsString> {
    words.iter().map(|word| OsString::from(*word)).collect()
}

/// A folder of its own under the temporary folder, removed when it drops
///
/// Named short, since on macOS a socket's path may be 103 bytes at most.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let folder = env::temp_dir().join(format!("kelpie-lr-{}", short_run()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        Self(folder)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

/// A random folder name short enough that a session's `lease.sock` inside it is
/// still a path macOS will bind
fn short_run() -> String {
    RunId::new().unwrap().to_string()[..8].to_owned()
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The leases of a test, with `capacity` places for `cargo-test`
fn leases(dir: &Scratch, capacity: u32) -> Leases {
    let config = dir.path().join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        format!(r#"{{"lease-capacity": {{"cargo-test": {capacity}}}}}"#),
    )
    .unwrap();
    Leases::new(&Settings::load(&ConfigDir::at(&config), "web").unwrap())
}

/// The tree a test's command runs through
fn tree(dir: &Scratch) -> ProcessTree {
    ProcessTree::new(RunId::new().unwrap(), dir.path().join("run.json")).unwrap()
}

/// Wait until `name` holds `want` places, as kelpie's lease status says
///
/// Only the rejoin test waits this way, and that test is macOS-only.
#[cfg(target_os = "macos")]
async fn holders_until(leases: &Leases, name: &str, want: usize) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while held(leases, name) != want {
        assert!(
            tokio::time::Instant::now() < deadline,
            "{name} holds {}, not {want}",
            held(leases, name)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn words<'a>(words: &'a [&'a str]) -> Vec<&'a OsStr> {
    words.iter().map(|word| OsStr::new(*word)).collect()
}

/// How many places of `name` are held, as kelpie's lease status says
fn held(leases: &Leases, name: &str) -> usize {
    leases
        .status()
        .iter()
        .find(|status| status.kind == name)
        .map_or(0, |status| status.holders.len())
}

/// The pid a script wrote to `file`, once it has written it
fn pid_in(file: &Path) -> u32 {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(text) = fs::read_to_string(file)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        assert!(Instant::now() < deadline, "{file:?} holds no pid");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait for kelpie to let `name`'s place go, which it does on its own task
///
/// A run gives the place back by closing its connection and returns at once, so
/// kelpie hears the end a moment later than the run does.
fn given_back(leases: &Leases, name: &str) {
    let deadline = Instant::now() + WAIT;
    while held(leases, name) != 0 {
        assert!(
            Instant::now() < deadline,
            "{name} is still held, though its command ended"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait until none of `pids` runs
fn assert_gone(pids: &[u32]) {
    let deadline = Instant::now() + WAIT;
    while let Some(pid) = pids
        .iter()
        .find(|pid| sweep::read(**pid).is_some_and(|reading| !reading.zombie))
    {
        assert!(Instant::now() < deadline, "pid {pid} still runs");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[tokio::test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn the_lease_is_let_go_when_the_command_ends() {
    let dir = Scratch::new();
    let leases = leases(&dir, 1);
    let session: SessionLease = leases.session(&dir.path().join("w")).unwrap();
    let tree = tree(&dir);

    assert_eq!(
        run(
            session.door(),
            CARGO_TEST,
            &words(&["sh", "-c", "exit 3"]),
            RUN_LIMIT,
            PATIENCE,
            &tree,
        )
        .await,
        3
    );
    given_back(&leases, CARGO_TEST);
    // The next command is granted at once, with no kelpie in the way
    assert_eq!(
        run(
            session.door(),
            CARGO_TEST,
            &words(&["sh", "-c", "true"]),
            RUN_LIMIT,
            PATIENCE,
            &tree,
        )
        .await,
        0
    );
    let _ = tree.stop_all(KILL_AFTER);
}

#[tokio::test]
#[cfg(all(unix, any(target_os = "macos", target_os = "linux")))]
async fn a_command_past_its_bound_is_ended_with_its_children_and_exits_124() {
    let dir = Scratch::new();
    let leases = leases(&dir, 1);
    let session = leases.session(&dir.path().join("w")).unwrap();
    let tree = tree(&dir);
    let pidfile = dir.path().join("sleep.pid");
    let script = format!("sleep 300 & echo $! > {}; wait", pidfile.display());
    let began = Instant::now();

    let code = run(
        session.door(),
        CARGO_TEST,
        &words(&["sh", "-c", &script]),
        Duration::from_millis(300),
        PATIENCE,
        &tree,
    )
    .await;

    assert_eq!(code, TIMED_OUT);
    assert!(
        began.elapsed() < WAIT,
        "it waited for the sleep instead of ending it"
    );
    assert_gone(&[pid_in(&pidfile)]);
    given_back(&leases, CARGO_TEST);
    let _ = tree.stop_all(KILL_AFTER);
}

#[tokio::test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn a_command_that_cannot_start_gives_the_lease_back() {
    let dir = Scratch::new();
    let leases = leases(&dir, 1);
    let session = leases.session(&dir.path().join("w")).unwrap();
    let tree = tree(&dir);

    assert_eq!(
        run(
            session.door(),
            CARGO_TEST,
            &words(&["kelpie-no-such-program"]),
            RUN_LIMIT,
            PATIENCE,
            &tree,
        )
        .await,
        NOT_RUN
    );
    given_back(&leases, CARGO_TEST);
    let _ = tree.stop_all(KILL_AFTER);
}

#[tokio::test]
#[cfg(any(target_os = "macos", target_os = "linux"))]
async fn a_run_with_no_kelpie_behind_the_socket_exits_75() {
    let dir = Scratch::new();
    let tree = tree(&dir);
    #[cfg(target_os = "macos")]
    let door = Door::parse(dir.path().join("nothing.sock").to_str().unwrap()).unwrap();
    #[cfg(target_os = "linux")]
    let door = Door::parse("127.0.0.1:1").unwrap();
    let patience = Patience {
        tries: 2,
        every: Duration::from_millis(10),
    };

    assert_eq!(
        run(
            &door,
            CARGO_TEST,
            &words(&["true"]),
            RUN_LIMIT,
            patience,
            &tree,
        )
        .await,
        NO_LEASE
    );
    let _ = tree.stop_all(KILL_AFTER);
}

#[test]
fn a_signal_ending_a_run_carries_the_shell_number() {
    assert_eq!(Caught::Interrupt.number(), 130);
    assert_eq!(Caught::Hangup.number(), 129);
    assert_eq!(Caught::Terminate.number(), 143);
}

#[test]
#[cfg(unix)]
fn a_command_killed_by_a_signal_exits_as_the_shell_says() {
    use std::os::unix::process::ExitStatusExt as _;

    let killed = std::process::Command::new("sh")
        .args(["-c", "kill -TERM $$"])
        .status()
        .unwrap();
    assert_eq!(exit_number(killed), 143);
    assert_eq!(exit_number(ExitStatus::from_raw(0)), 0);
    assert_eq!(exit_number(ExitStatus::from_raw(0x0100)), 1);
}

/// A run whose kelpie went away asks the kelpie that comes back to count it
///
/// macOS only, because a restarted kelpie is at the same socket there. Linux
/// binds a port the OS picks, so the run's door is gone with the kelpie that
/// opened it.
#[tokio::test]
#[cfg(target_os = "macos")]
async fn a_run_asks_a_restarted_kelpie_to_count_its_running_command_again() {
    let dir = Scratch::new();
    let state = dir.path().join("w");
    let patience = Patience {
        tries: 40,
        every: Duration::from_millis(100),
    };

    let first = leases(&dir, 1);
    let session = first.session(&state).unwrap();
    let door = session.door().clone();
    let tree = tree(&dir);
    let command = words(&["sh", "-c", "sleep 5"]);

    let gone_and_back = async {
        holders_until(&first, CARGO_TEST, 1).await;
        // kelpie goes away: its sockets close and its runtime ends, while the
        // command it granted keeps running
        drop(session);
        drop(first);
        // ...and comes back on the same socket, where the run is still going
        let back = leases(&dir, 1);
        let session = back.session(&state).unwrap();
        assert_eq!(
            session.door(),
            &door,
            "the restarted kelpie is at the socket"
        );
        holders_until(&back, CARGO_TEST, 1).await;
    };

    let (code, ()) = tokio::join!(
        run(&door, CARGO_TEST, &command, RUN_LIMIT, patience, &tree),
        gone_and_back
    );
    let _ = tree.stop_all(KILL_AFTER);
    assert_eq!(code, 0, "the command ran out, counted all along");
}
