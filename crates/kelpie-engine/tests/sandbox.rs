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
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::thread;
use std::time::{Duration, Instant};

use kelpie_engine::run::RunId;
use kelpie_engine::sandbox::{Project, Reads, Sandbox, run_from_args};
use kelpie_engine::session::Harness;
use kelpie_engine::tools::{
    self, DOOR_VAR, Door, Reach, SessionKey, SessionTools, TOKEN_VAR, Tools,
};
use kelpie_engine::tree::{ProcessTree, sweep_marked};
use serde_json::{Value, json};

// SK's sandbox escapes, tried against the PM's policy. Their own file, and run
// here, since a sandboxed test needs this binary's `main` on Linux
#[path = "sandbox/escapes.rs"]
mod escapes;

/// The argument that makes this binary the sandboxed process the sweep looks for
const SLEEP: &str = "--sleep-in-the-sandbox";

/// The argument that makes this binary dial an address and say whether it got through
const DIAL: &str = "--dial";

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(20);

fn main() {
    run_from_args();
    // The relay is this binary run as `kelpie tools`, as the app's `main` runs it
    tools::run_from_args();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some(SLEEP) => {
            sleep_in_the_sandbox(args.next().map(PathBuf::from));
            return;
        }
        Some(DIAL) => std::process::exit(dial(&args.next().unwrap_or_default())),
        // One of the escapes, in the copy of this binary its test started
        Some(escapes::IN_OWN_HOME) => escapes::run_case(&args.next().expect("a case's tag")),
        _ => {}
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

const TESTS: &[(&str, fn())] = &[
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
        "a_real_read_outside_the_project_follows_the_setting",
        a_real_read_outside_the_project_follows_the_setting,
    ),
    (
        "the_sweep_finds_a_sandboxed_process",
        the_sweep_finds_a_sandboxed_process,
    ),
    (
        "the_relay_reaches_kelpie_from_inside_the_sandbox",
        the_relay_reaches_kelpie_from_inside_the_sandbox,
    ),
    ("the_door_opens_nothing_else", the_door_opens_nothing_else),
    (
        "an_allowed_host_is_reached_by_a_running_sandbox",
        an_allowed_host_is_reached_by_a_running_sandbox,
    ),
    (
        "escapes::harness_files_are_refused_in_any_case_and_spelling",
        escapes::harness_files_are_refused_in_any_case_and_spelling,
    ),
    (
        "escapes::a_nested_claude_folder_is_refused",
        escapes::a_nested_claude_folder_is_refused,
    ),
    (
        "escapes::the_pm_reads_the_project_and_no_other_repo_of_the_home",
        escapes::the_pm_reads_the_project_and_no_other_repo_of_the_home,
    ),
    (
        "escapes::a_403_to_the_connect_is_the_proxys_refusal_and_a_failed_connect_is_not",
        escapes::a_403_to_the_connect_is_the_proxys_refusal_and_a_failed_connect_is_not,
    ),
    (
        "escapes::a_host_not_allowed_and_a_socket_not_listed_are_refused",
        escapes::a_host_not_allowed_and_a_socket_not_listed_are_refused,
    ),
    // Only Linux turns a listed socket off; macOS reaches it
    #[cfg(target_os = "linux")]
    (
        "escapes::a_socket_the_policy_lists_is_still_refused_on_linux",
        escapes::a_socket_the_policy_lists_is_still_refused_on_linux,
    ),
];

/// Exit 0 when `address`, a socket's path or `host:port`, answers, and 1 when not
fn dial(address: &str) -> i32 {
    let reached = if Path::new(address).is_absolute() {
        UnixStream::connect(address).is_ok()
    } else {
        address.parse::<SocketAddr>().is_ok_and(|address| {
            TcpStream::connect_timeout(&address, Duration::from_secs(3)).is_ok()
        })
    };
    i32::from(!reached)
}

/// Says it is up by writing a file, then waits to be stopped
fn sleep_in_the_sandbox(ready: Option<PathBuf>) {
    if let Some(ready) = ready {
        fs::write(ready, std::process::id().to_string()).expect("the state folder is writable");
    }
    thread::sleep(Duration::from_secs(60));
}

/// A folder of its own under the target's temp folder, with a project, the state
/// folders of two sessions, and a folder outside all of them, removed when it
/// drops
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(test: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("sandbox-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // The project sits one below the folder it may read beside, so the
        // `outside` folder is beside neither it nor the session's state
        for folder in [
            "work/project",
            "outside",
            "data/projects/pm",
            "data/projects/other",
        ] {
            fs::create_dir_all(root.join(folder)).expect("the scratch folders are made");
        }
        // The path as the OS names it, since a sandbox compares real paths
        let root = fs::canonicalize(root).expect("the scratch folder is there");
        Self { root }
    }

    fn project(&self) -> PathBuf {
        self.root.join("work/project")
    }

    fn state(&self) -> PathBuf {
        self.sessions_root().join("pm")
    }

    /// The folder holding this session's state and another's, as kelpie's data
    /// folder holds one per session
    fn sessions_root(&self) -> PathBuf {
        self.root.join("data/projects")
    }

    fn other(&self) -> PathBuf {
        self.sessions_root().join("other")
    }

    fn outside(&self) -> PathBuf {
        self.root.join("outside")
    }

    /// A sandbox for a session in the project that reads as `reads`, on `program`,
    /// with kelpie's tools `door` named in its policy when it has one
    fn sandbox(&self, program: &Path, reads: Reads, door: Option<&Door>) -> Sandbox {
        let mut harness = Harness::new(program);
        // The harness's own install is read, as a real one's is
        harness.installs = program
            .parent()
            .map(Path::to_path_buf)
            .into_iter()
            .collect();
        let (folder, state, sessions_root) = (self.project(), self.state(), self.sessions_root());
        let allowlist: Vec<String> = Vec::new();
        let project = Project {
            folder: &folder,
            network_allowlist: &allowlist,
            reads,
            sessions_root: &sessions_root,
        };
        Sandbox::start_with_door(&harness, &project, &state, door)
            .unwrap_or_else(|error| panic!("the sandbox did not start: {error}"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A folder under the home folder, removed when it drops
///
/// The home folder is the area `Reads::Project` closes whole and `Reads::Anywhere`
/// opens, so a file outside the project only follows the setting when it sits
/// inside it. kelpie's own name and this process's id keep the folder from
/// meeting another run's.
struct HomeFolder {
    folder: PathBuf,
}

impl HomeFolder {
    fn new(test: &str) -> Self {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .expect("the OS names a home folder");
        let folder =
            PathBuf::from(home).join(format!(".kelpie-sandbox-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("the home folder takes a scratch folder for the test");
        Self { folder }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.folder.join(name)
    }
}

impl Drop for HomeFolder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.folder);
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

/// A sandbox for `program` that is known to run commands, or `None` when this
/// host cannot run one at all.
///
/// A refusal test that ran in a sandbox which never started would pass, as a
/// command that did not run leaves no file and exits non-zero. So each test
/// asks for a sandbox that has run a command first, and a host that cannot
/// fails the test. `KELPIE_SKIP_SANDBOX_TESTS` skips it instead, shown as
/// `SKIPPED`, for a host known not to run one.
fn working_sandbox(scratch: &Scratch, test: &str, program: &Path, reads: Reads) -> Option<Sandbox> {
    working_sandbox_with_door(scratch, test, program, reads, None)
}

/// [`working_sandbox`], with kelpie's tools `door` named in the policy
fn working_sandbox_with_door(
    scratch: &Scratch,
    test: &str,
    program: &Path,
    reads: Reads,
    door: Option<&Door>,
) -> Option<Sandbox> {
    let sandbox = scratch.sandbox(program, reads, door);
    host_can_run(test, &sh(&sandbox, "true")).then_some(sandbox)
}

/// Whether `probe`, `true` run in a sandbox, shows this host runs one
///
/// A host that cannot fails the test, or skips it when [`SKIP`] is set.
fn host_can_run(test: &str, probe: &Output) -> bool {
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

fn a_write_outside_the_state_dir_is_refused() {
    let scratch = Scratch::new("refused");
    let Some(sandbox) = working_sandbox(
        &scratch,
        "a_write_outside_the_state_dir_is_refused",
        Path::new("/bin/sh"),
        Reads::Project,
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
        Reads::Project,
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

/// `path` read by `sh` inside `sandbox`, `None` when the sandbox refused it
fn cat(sandbox: &Sandbox, path: &Path) -> Option<String> {
    let run = sh(sandbox, &format!("cat {}", path.display()));
    run.status
        .success()
        .then(|| String::from_utf8_lossy(&run.stdout).into_owned())
}

/// A real read outside the project follows the setting
///
/// The area `Reads::Project` closes is the home folder and the folder holding
/// every session's state. crabpot allows every read the policy does not deny
/// (ADR 0005's reads wide), so a file in some other folder is read at either
/// setting and proves nothing: the file outside the project has to be one the
/// policy closes.
fn a_real_read_outside_the_project_follows_the_setting() {
    let name = "a_real_read_outside_the_project_follows_the_setting";
    let (scratch, home) = (
        Scratch::new("read-anywhere"),
        HomeFolder::new("read-anywhere"),
    );
    let (project, outside) = (scratch.project().join("notes.md"), home.file("notes.md"));
    let (other, own) = (scratch.other().join("log"), scratch.state().join("log"));
    for (file, text) in [
        (&project, "in the project"),
        (&outside, "outside the project"),
        (&other, "another session's state"),
        (&own, "this session's own state"),
    ] {
        fs::write(file, text).expect("the scratch files are written");
    }

    let Some(closed) = working_sandbox(&scratch, name, Path::new("/bin/sh"), Reads::Project) else {
        return;
    };
    assert_eq!(cat(&closed, &project).as_deref(), Some("in the project"));
    assert_eq!(
        cat(&closed, &own).as_deref(),
        Some("this session's own state"),
        "the session cannot read its own state folder with Read anywhere off"
    );
    let refused = cat(&closed, &outside);
    assert!(
        refused.is_none(),
        "a file in the home folder was read with Read anywhere off: {refused:?}"
    );
    drop(closed);

    let Some(wide) = working_sandbox(&scratch, name, Path::new("/bin/sh"), Reads::Anywhere) else {
        return;
    };
    assert_eq!(
        cat(&wide, &outside).as_deref(),
        Some("outside the project"),
        "Read anywhere did not reach outside the project"
    );
    assert_eq!(
        cat(&wide, &own).as_deref(),
        Some("this session's own state"),
        "the session cannot read its own state folder"
    );
    let other_read = cat(&wide, &other);
    assert!(
        other_read.is_none(),
        "another session's state was read: {other_read:?}"
    );
}

fn the_sweep_finds_a_sandboxed_process() {
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

/// The agent's end of an MCP conversation with the relay running in a sandbox
///
/// Returns what the server said, and the sessions kelpie was serving while the
/// relay was still up: once the relay is stopped its connection ends, so that
/// is read before.
fn mcp_through_the_sandbox(
    sandbox: &Sandbox,
    exe: &Path,
    tools: &Tools,
    session: &SessionTools,
) -> (Vec<Value>, Vec<SessionKey>) {
    let (door, token) = (session.door(), session.token());
    let mut relay = Command::new(exe);
    relay
        .arg("tools")
        .env(DOOR_VAR, door.to_string())
        .env(TOKEN_VAR, token);
    let mut child = sandbox
        .command(&relay)
        .expect("the relay goes in the sandbox")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the sandbox runs the relay");
    let mut to_relay = child.stdin.take().unwrap();
    let from_relay = child.stdout.take().unwrap();
    let (sender, lines) = channel();
    thread::spawn(move || {
        for line in BufReader::new(from_relay).lines().map_while(Result::ok) {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let hear = || {
        let line = lines
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("the relay carried nothing back from inside the sandbox"));
        serde_json::from_str::<Value>(&line).unwrap_or_else(|error| panic!("{error}: {line}"))
    };
    let mut say = |message: Value| writeln!(to_relay, "{message}").unwrap();
    say(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"},
        },
    }));
    let initialized = hear();
    say(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    say(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let listed = hear();
    let connected = tools.connected();
    drop(to_relay);
    let _ = child.kill();
    let _ = child.wait();
    (vec![initialized, listed], connected)
}

fn the_relay_reaches_kelpie_from_inside_the_sandbox() {
    let scratch = Scratch::new("relay");
    let exe = std::env::current_exe().unwrap();
    let tools = Tools::start(&scratch.outside()).expect("the door opens");
    let session = tools.session();
    let Some(sandbox) = working_sandbox_with_door(
        &scratch,
        "the_relay_reaches_kelpie_from_inside_the_sandbox",
        &exe,
        Reads::Project,
        Some(session.door()),
    ) else {
        return;
    };

    let (heard, connected) = mcp_through_the_sandbox(&sandbox, &exe, &tools, &session);

    assert_eq!(heard[0]["id"], 1, "{}", heard[0]);
    assert!(heard[0]["result"]["capabilities"]["tools"].is_object());
    assert_eq!(heard[1]["id"], 2);
    let listed: Vec<&Value> = heard[1]["result"]["tools"]
        .as_array()
        .expect("a list of tools")
        .iter()
        .map(|tool| &tool["name"])
        .collect();
    assert_eq!(listed, ["request_network"], "{}", heard[1]);
    assert_eq!(
        connected,
        [session.key()],
        "the connection is the session's, by its token"
    );
}

fn the_door_opens_nothing_else() {
    let scratch = Scratch::new("door");
    let exe = std::env::current_exe().unwrap();
    let tools = Tools::start(&scratch.outside()).expect("the door opens");
    let session = tools.session();
    let Some(sandbox) = working_sandbox_with_door(
        &scratch,
        "the_door_opens_nothing_else",
        &exe,
        Reads::Project,
        Some(session.door()),
    ) else {
        return;
    };
    let dial_in_sandbox = |address: &str| {
        let mut command = Command::new(&exe);
        command.args([DIAL, address]);
        sandbox
            .command(&command)
            .expect("the dial goes in the sandbox")
            .output()
            .expect("the sandbox runs the dial")
    };
    // The control: the door answers, so the refusals below are the policy's and
    // not a sandbox that dials nothing
    let control = dial_in_sandbox(&session.door().to_string());
    assert!(
        control.status.success(),
        "the door was not reached: {control:?}"
    );
    // Anything else on the loopback, which accepts if it is let through
    let other_port = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let other_port_address = other_port.local_addr().unwrap().to_string();
    let mut others = vec![("another loopback port", other_port_address)];
    // macOS names the door as a socket, so another socket is a test of its own
    let other_socket = scratch.outside().join("other.sock");
    let _listener = if session.door().socket().is_some() {
        let listener = UnixListener::bind(&other_socket).unwrap();
        others.push(("another socket", other_socket.display().to_string()));
        Some(listener)
    } else {
        None
    };

    for (what, address) in others {
        let refused = dial_in_sandbox(&address);

        assert!(
            !refused.status.success(),
            "{what} ({address}) was let through: {refused:?}"
        );
    }
}

/// `curl`'s answer to a CONNECT through the sandbox's proxy to `https://{host}/`
fn connect_code(sandbox: &Sandbox, host: &str) -> String {
    let out = sh(
        sandbox,
        &format!("curl -sS -m 20 -o /dev/null -w '%{{http_connect}}' https://{host}/"),
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A host the proxy refused is, once `request_network` is allowed, no longer
/// refused by the same sandbox, still running
fn an_allowed_host_is_reached_by_a_running_sandbox() {
    let scratch = Scratch::new("widen");
    let exe = std::env::current_exe().unwrap();
    let Some(sandbox) = working_sandbox(
        &scratch,
        "an_allowed_host_is_reached_by_a_running_sandbox",
        &exe,
    ) else {
        return;
    };
    assert_eq!(
        connect_code(&sandbox, "example.com"),
        "403",
        "the proxy did not refuse the host before the answer"
    );
    let network = sandbox.network();

    assert!(network.allow("example.com"), "the sandbox took the host");

    // Let through: a 200 with a network, and no 403 without one
    let code = connect_code(&sandbox, "example.com");
    assert_ne!(code, "403", "the host was still refused after the answer");
    assert_eq!(
        connect_code(&sandbox, "example.org"),
        "403",
        "only the host asked for was let through"
    );
}
