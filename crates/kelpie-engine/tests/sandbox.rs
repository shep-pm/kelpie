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

use kelpie_engine::harness::global;
use kelpie_engine::run::RunId;
use kelpie_engine::sandbox::{Project, Sandbox, run_from_args};
use kelpie_engine::session::Harness;
use kelpie_engine::tree::{ProcessTree, sweep_marked};

// SK's sandbox escapes, tried against the PM's policy. Their own file, and run
// here, since a sandboxed test needs this binary's `main` on Linux
#[path = "sandbox/escapes.rs"]
mod escapes;

/// The argument that makes this binary the sandboxed process the sweep looks for
const SLEEP: &str = "--sleep-in-the-sandbox";

/// The argument that makes this binary run the global folder checks with `HOME` moved
/// into the scratch folder, which is how a test reaches a path the policy denies
const IN_HOME: &str = "--in-the-persons-home";

/// What that child exits with when this host runs no sandbox at all
const CHILD_SKIPPED: i32 = 3;

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(20);

fn main() {
    run_from_args();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some(SLEEP) => {
            sleep_in_the_sandbox(args.next().map(PathBuf::from));
            return;
        }
        // One of the escapes, in the copy of this binary its test started
        Some(escapes::IN_OWN_HOME) => escapes::run_case(&args.next().expect("a case's tag")),
        _ => {}
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|word| word == IN_HOME) {
        let root = args
            .get(1)
            .expect("the scratch folder follows --in-the-persons-home");
        std::process::exit(in_the_persons_home(Path::new(root)));
    }
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
        "the_global_folder_is_read_only_when_on",
        the_global_folder_is_read_only_when_on,
    ),
    (
        "the_sweep_finds_a_sandboxed_process",
        the_sweep_finds_a_sandboxed_process,
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

    /// A folder of this scratch, made when it is asked for
    fn folder(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).expect("the scratch folder is made");
        path
    }

    /// A sandbox for a session in the project, whose harness is `program`
    fn sandbox(&self, program: &Path) -> Sandbox {
        self.sandbox_for(program, &self.project(), &[])
    }

    /// A sandbox for a session in `project`, whose harness is `program` and reads
    /// `global_reads` of the person's own global folder
    fn sandbox_for(&self, program: &Path, project: &Path, global_reads: &[PathBuf]) -> Sandbox {
        sandbox_in(program, project, &self.state(), global_reads)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A sandbox for a session whose harness is `program`, working in `project`, keeping its
/// files in `state` and reading `global_reads` of the person's own global folder
fn sandbox_in(program: &Path, project: &Path, state: &Path, global_reads: &[PathBuf]) -> Sandbox {
    let mut harness = Harness::new(program);
    // The harness's own install is read, as a real one's is
    harness.installs = program
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    harness.global_reads = global_reads.to_vec();
    let confined = Project {
        folder: project,
        network_allowlist: &[],
        read_anywhere: false,
    };
    Sandbox::start(&harness, &confined, state)
        .unwrap_or_else(|error| panic!("the sandbox did not start: {error}"))
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

/// shorthand for [`working`] with a sandbox for a session on `program` in the project
fn working_sandbox(scratch: &Scratch, test: &str, program: &Path) -> Option<Sandbox> {
    working(scratch.sandbox(program), test)
}

/// `sandbox`, once it is known to run commands, or `None` when this host cannot
/// run one at all
///
/// A refusal test that ran in a sandbox which never started would pass, as a
/// command that did not run leaves no file and exits non-zero. So each test
/// asks for a sandbox that has run a command first, and a host that cannot
/// fails the test. `KELPIE_SKIP_SANDBOX_TESTS` skips it instead, shown as
/// `SKIPPED`, for a host known not to run one.
fn working(sandbox: Sandbox, test: &str) -> Option<Sandbox> {
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

/// A session reads the entry of the person's global folder its harness linked, and nothing
/// else there, whatever the setting it is run with
///
/// Every path the policy denies lies under `~`, so the checks run in a child whose `HOME`
/// is the scratch folder: only there is the person's folder one the policy closes and a
/// link reaches into. `set_var` is unsafe, which the crate forbids, so the scratch is
/// named to the child instead of set here.
fn the_global_folder_is_read_only_when_on() {
    let scratch = Scratch::new("global-folder");
    // The person's own folder, kept out of the folders beside the project so only
    // what is linked can be read
    let global = scratch.folder("home/.claude");
    let (login, linked) = (global.join(".credentials.json"), global.join("CLAUDE.md"));
    fs::write(&login, "their login\n").unwrap();
    fs::write(&linked, "the person's own instructions\n").unwrap();

    let run = Command::new(std::env::current_exe().expect("this binary"))
        .arg(IN_HOME)
        .arg(&scratch.root)
        .env("HOME", &scratch.root)
        .output()
        .expect("the child runs with the scratch folder as its home folder");

    match run.status.code() {
        Some(CHILD_SKIPPED) => {
            SKIPPED.fetch_add(1, Ordering::SeqCst);
            println!(
                "SKIPPED the_global_folder_is_read_only_when_on: the child found no sandbox to run"
            );
        }
        Some(0) => {}
        _ => panic!(
            "the linked entry, the login beside it or a write through the link was not what the policy says: {}",
            String::from_utf8_lossy(&run.stderr)
        ),
    }
}

/// The checks of [`the_global_folder_is_read_only_when_on`], with `root` as this home folder
fn in_the_persons_home(root: &Path) -> i32 {
    let (global, project, state) = (
        root.join("home/.claude"),
        root.join("work/project"),
        root.join("state"),
    );
    let (login, linked) = (global.join(".credentials.json"), global.join("CLAUDE.md"));
    // The session's config folder, holding the person's entry as a harness builder links it
    let (config, through) = (state.join("claude"), state.join("claude").join("CLAUDE.md"));
    let reads = global::link(&global, &config, &["CLAUDE.md"]).unwrap_or_else(|error| {
        panic!("the person's entry was not linked into the session: {error}")
    });

    let Some(on) = working(
        sandbox_in(Path::new("/bin/sh"), &project, &state, &reads),
        "the_global_folder_is_read_only_when_on",
    ) else {
        return CHILD_SKIPPED;
    };

    let read = sh(&on, &format!("cat {}", through.display()));
    assert!(
        read.status.success(),
        "the linked entry was not read through the session's name for it: {read:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&read.stdout),
        "the person's own instructions\n"
    );

    let login_read = sh(&on, &format!("cat {}", login.display()));
    assert!(
        !login_read.status.success(),
        "the person's login was read through their global folder: {login_read:?}"
    );

    let written = sh(&on, &format!("echo nope > {}", through.display()));
    assert!(
        !written.status.success(),
        "a write through the linked entry was not refused: {written:?}"
    );
    assert_eq!(
        fs::read_to_string(&linked).unwrap(),
        "the person's own instructions\n",
        "a sandboxed write changed the person's own file"
    );
    drop(on);

    // The same session with the setting off: the link is taken back, and the person's
    // own file is closed again
    let cleared = global::link(&global, &config, &[])
        .unwrap_or_else(|error| panic!("the link was not taken back: {error}"));
    assert!(cleared.is_empty(), "{cleared:?}");
    assert!(!through.exists(), "the link stayed in the session's folder");

    let Some(off) = working(
        sandbox_in(Path::new("/bin/sh"), &project, &state, &[]),
        "the_global_folder_is_read_only_when_on",
    ) else {
        return CHILD_SKIPPED;
    };

    let read = sh(&off, &format!("cat {}", linked.display()));
    assert!(
        !read.status.success(),
        "the person's own file was read with nothing linked into the session: {read:?}"
    );
    0
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
