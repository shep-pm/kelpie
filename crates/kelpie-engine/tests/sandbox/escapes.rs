//! SK's sandbox escapes, each tried against the PM's policy and each refused
//!
//! Lifted from SK's `adapters/claude/escapes.rs` at `932c57a5` (`docs/lifted.md`).
//! A real `sh` runs inside crabpot with [`kelpie_engine::sandbox::policy`], as
//! the PM's policy builds it: a shell stands in for the harness, since inside
//! the sandbox its file tools, hooks and commands are alike.
//!
//! The PM writes its state folder and nothing else, so each escape SK tried
//! from a worker's worktree is tried here from the PM's project, which is read
//! only. SK's escapes that need a worker (its worktree, build folder, clone and
//! live turn) wait for workers.
//!
//! What the PM must not write is found by comparing the world's folders before
//! and after, outside the sandbox, so a file written under any spelling shows,
//! whether or not the host's file system folds it. Each refusal sits beside a
//! control that ran in the same sandbox, since a command that never ran refuses
//! everything.
//!
//! Each case runs in a copy of this binary whose `HOME` is a folder of its own
//! under the target's temp folder. The policy closes `~`, so the world has to
//! be in a home, and a home of its own keeps the stand-ins for the logins, and
//! the writes that a failing sandbox would let through, out of the person's real
//! one. A killed run leaves a folder in the target's temp folder and nothing in
//! the home folder.
//!
//! A child of `tests/sandbox.rs`, which is its own `main` (crabpot's Linux helper
//! is this binary run again).

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::Ordering;

use kelpie_engine::sandbox::{Project, Reads, Sandbox};
use kelpie_engine::session::Harness;

use super::{SKIPPED, host_can_run};

/// The logins of Claude Code and goose in the home folder, as a person has them
/// (`sandbox::denied`)
const LOGINS: [&str; 4] = [
    ".claude.json",
    ".claude/.credentials.json",
    ".config/goose/config.yaml",
    ".local/share/goose/sessions/sessions.db",
];

/// The folders of a PM's world, in the home folder, which the sandbox closes: a
/// project and a sibling repo that the PM reads, the state folder it writes
/// under the folder holding every session's state, another repo that it reads
/// none of, and the logins of Claude Code and goose
///
/// The policy reads the project and the folders beside it with `~` denied under
/// them, so the world is the home folder itself, which [`in_own_home`] makes.
struct World {
    root: PathBuf,
}

impl World {
    fn new() -> Self {
        let root = home();
        for folder in [
            "work/kelpie/src",
            "work/sibling/src",
            "projects/pm",
            "other-repo/src",
            "other-repo/.git",
        ] {
            fs::create_dir_all(root.join(folder)).expect("the world's folders are made");
        }
        let world = Self { root };
        let files = [
            ("work/kelpie/src/lib.rs", "the project's own\n"),
            ("work/sibling/src/lib.rs", "a sibling's own\n"),
            ("projects/pm/notes.md", "the PM's own\n"),
            ("other-repo/.env", "SECRET=1\n"),
            ("other-repo/src/lib.rs", "private work\n"),
            ("other-repo/.git/config", "[remote]\n"),
        ];
        for (path, text) in files {
            fs::write(world.path(path), text).expect("the world's files are written");
        }
        for login in LOGINS {
            let path = world.path(login);
            fs::create_dir_all(path.parent().unwrap()).expect("a login's folder is made");
            fs::write(&path, SECRET).expect("a login is written");
        }
        world
    }

    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    fn project(&self) -> PathBuf {
        self.path("work/kelpie")
    }

    /// The folder holding every session's state, as kelpie's data folder holds
    /// one per session
    fn sessions_root(&self) -> PathBuf {
        self.path("projects")
    }

    fn state(&self) -> PathBuf {
        self.sessions_root().join("pm")
    }

    fn harness() -> Harness {
        let mut harness = Harness::new("/bin/sh");
        harness.hosts = vec!["api.anthropic.com".to_owned()];
        harness.installs = vec![PathBuf::from("/bin")];
        harness
    }

    /// The PM's sandbox, known to run commands, or `None` on a host that cannot
    /// run one (`KELPIE_SKIP_SANDBOX_TESTS`)
    fn sandbox(&self, test: &str) -> Option<Sandbox> {
        let (folder, sessions_root, state) = (self.project(), self.sessions_root(), self.state());
        let sandbox = Sandbox::start(
            &Self::harness(),
            &project_in(&folder, &sessions_root),
            &state,
        )
        .unwrap_or_else(|error| panic!("the sandbox did not start: {error}"));
        host_can_run(test, &self.run(&sandbox, "true")).then_some(sandbox)
    }

    /// `script` run by `sh` in the project, inside the sandbox
    fn run(&self, sandbox: &Sandbox, script: &str) -> Output {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]).current_dir(self.project());
        sandbox
            .command(&command)
            .expect("the command goes in the sandbox")
            .output()
            .expect("the sandbox runs sh")
    }

    /// Asserts the PM read the project's own file, which is what makes a
    /// refusal in the same sandbox its policy's
    fn control(&self, sandbox: &Sandbox) {
        let out = self.run(sandbox, "cat src/lib.rs");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "the project's own\n",
            "the control read failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Asserts `script` changed nothing in the world but its state folder
    fn refused(&self, sandbox: &Sandbox, script: &str) {
        let before = self.snapshot();
        let out = self.run(sandbox, script);
        let after = self.snapshot();
        assert!(
            before == after,
            "{script} changed the world: {:?}\n{}",
            before.symmetric_difference(&after).collect::<Vec<_>>(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Every name in the world but the state folder's, with its kind and size
    fn snapshot(&self) -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut folders = vec![self.root.clone()];
        while let Some(folder) = folders.pop() {
            for entry in fs::read_dir(&folder).expect("a folder of the world is read") {
                let path = entry.expect("an entry is read").path();
                if path == self.state() {
                    continue;
                }
                let meta = fs::symlink_metadata(&path).expect("an entry has metadata");
                if meta.is_dir() {
                    folders.push(path.clone());
                }
                let kind = if meta.is_dir() {
                    "folder"
                } else if meta.is_file() {
                    "file"
                } else {
                    "other"
                };
                let relative = path.strip_prefix(&self.root).unwrap().display();
                seen.insert(format!("{relative} {kind} {}", meta.len()));
            }
        }
        seen
    }
}

/// What every login of the world holds
const SECRET: &str = "kelpie-escapes-secret\n";

/// A project in `folder`, with the PM's own settings: reads closed to the
/// project, and `sessions_root` holding every session's state
fn project_in<'a>(folder: &'a Path, sessions_root: &'a Path) -> Project<'a> {
    Project {
        folder,
        network_allowlist: &[],
        reads: Reads::Project,
        sessions_root,
    }
}

fn home() -> PathBuf {
    let home = env::var_os("HOME").expect("HOME names the home folder the policy closes");
    fs::canonicalize(home).expect("the home folder is there")
}

/// Claude Code's and goose's own files are refused to the PM in any case and
/// spelling: the ones its project would carry, written, and the ones its home
/// would carry, read
fn harness_files_are_refused() {
    let world = World::new();
    let Some(sandbox) = world.sandbox("harness_files_are_refused_in_any_case_and_spelling") else {
        return;
    };
    world.control(&sandbox);

    // What the project would carry, which the PM does not write
    for script in [
        "mkdir other && echo {} > other/settings.json && mv other .claude",
        "mkdir .claude",
        "mkdir .CLAUDE",
        "echo {} > .mcp.json",
        "echo {} > .MCP.JSON",
        "echo {} > .mcp.j\u{17f}on",
        "ln -s /etc/hosts .mcp.json",
        "mkdir -p src/.CLAUDE/skills && echo x > src/.CLAUDE/skills/s.md",
        "ln -s ../outside .claude",
        "mkdir other && echo x > other/config.yaml && mv other .goose",
        "mkdir .goose",
        "mkdir .GOOSE",
        "echo x > .goosehints",
        "echo x > .GOOSEHINTS",
        "ln -s /etc/hosts .goosehints",
        "mkdir -p src/.Goose/recipes && echo x > src/.Goose/recipes/r.yaml",
        "ln -s ../outside .goose",
    ] {
        world.refused(&sandbox, script);
    }

    // Once the folders are there, what is written in them
    for folder in [".claude", ".goose"] {
        fs::create_dir_all(world.project().join(folder)).unwrap();
    }
    for script in [
        "echo {} > .claude/settings.local.json",
        "mkdir -p .CLAUDE && echo {} > .CLAUDE/Settings.Local.json",
        "mkdir -p .claude/agents && echo x > .claude/agents/a.md",
        "cp /etc/hosts .claude/settings.json",
        "echo x > .goose/config.yaml",
        "mkdir -p .Goose && echo x > .Goose/Config.YAML",
    ] {
        world.refused(&sandbox, script);
    }

    // What its home would carry, which the PM does not read. Each login holds
    // something, and a login the policy hides is an empty read or an error,
    // whichever the platform makes of it, so any output is a leak.
    let home = home();
    for login in LOGINS {
        assert_eq!(fs::read_to_string(home.join(login)).unwrap(), SECRET);
        let spellings = [
            home.join(login),
            home.join(login.to_uppercase()),
            PathBuf::from(format!("{}/./{login}", home.display())),
            PathBuf::from(format!("{}//{login}", home.display())),
            home.join(login.replace('s', "\u{17f}")),
        ];
        for spelling in spellings {
            let out = world.run(&sandbox, &format!("cat '{}'", spelling.display()));
            assert!(
                out.stdout.is_empty(),
                "{login} was read as {}: {}",
                spelling.display(),
                String::from_utf8_lossy(&out.stdout)
            );
        }
        // And not written
        let at = home.join(login);
        world.run(
            &sandbox,
            &format!("echo x >> '{}'; echo x > '{}'", at.display(), at.display()),
        );
        assert_eq!(fs::read_to_string(&at).unwrap(), SECRET, "{login} changed");
    }
}

/// A `.claude` folder below the project, or beside it, is refused as the
/// project's own is
fn a_nested_claude_folder() {
    let world = World::new();
    for folder in [
        "work/kelpie/src/.claude",
        "work/kelpie/lib/.Goose",
        "work/sibling/.claude",
    ] {
        fs::create_dir_all(world.path(folder)).unwrap();
    }
    let Some(sandbox) = world.sandbox("a_nested_claude_folder_is_refused") else {
        return;
    };
    world.control(&sandbox);

    for script in [
        "echo {} > src/.claude/settings.local.json",
        "mkdir -p src/.claude/agents && echo x > src/.claude/agents/a.md",
        "mkdir -p lib/.GOOSE && echo x > lib/.GOOSE/config.yaml",
        "echo {} > ../sibling/.claude/settings.local.json",
    ] {
        world.refused(&sandbox, script);
    }
}

/// The PM reads its project, a repo beside it and its own state folder, and
/// nothing of another repo of the same home folder, private ones and their
/// `.env` files included
fn the_pm_reads() {
    let world = World::new();
    let Some(sandbox) = world.sandbox("the_pm_reads_the_project_and_no_other_repo_of_the_home")
    else {
        return;
    };

    for (path, text) in [
        ("work/kelpie/src/lib.rs", "the project's own\n"),
        ("work/sibling/src/lib.rs", "a sibling's own\n"),
        ("projects/pm/notes.md", "the PM's own\n"),
    ] {
        let out = world.run(&sandbox, &format!("cat {}", world.path(path).display()));
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            text,
            "{path}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    for path in [
        "other-repo/.env",
        "other-repo/src/lib.rs",
        "other-repo/.git/config",
    ] {
        let out = world.run(&sandbox, &format!("cat {}", world.path(path).display()));
        let why = String::from_utf8_lossy(&out.stderr);
        // macOS seatbelt refuses the open. Linux crabpot covers a denied
        // folder with an empty tmpfs, so the name is not there at all.
        let expected = if cfg!(target_os = "linux") {
            "No such file or directory"
        } else {
            "Operation not permitted"
        };
        assert!(
            !out.status.success() && out.stdout.is_empty() && why.contains(expected),
            "{path} was read: {out:?}"
        );
    }
}

/// Whether `curl`'s `%{http_connect}` shows the proxy refusing the CONNECT
///
/// The proxy answers one to a host the policy does not list with a 403. A
/// `curl` that could not reach a proxy at all prints `000`, and fails the same
/// way with no network, so only the 403 shows the proxy refused it.
fn refused_by_the_proxy(connect_code: &str) -> bool {
    connect_code.trim() == "403"
}

/// A refusal is the proxy's 403 to the CONNECT, which a failed connect is not
pub fn a_403_to_the_connect_is_the_proxys_refusal_and_a_failed_connect_is_not() {
    // `curl -sS -w '%{http_connect}'` against a proxy that refuses, and with none to reach
    assert!(refused_by_the_proxy("403"));
    assert!(!refused_by_the_proxy("000"));
    assert!(!refused_by_the_proxy(""));
}

/// What `perl` exits with when it cannot reach a Unix socket
const UNREACHED: i32 = 7;

/// A script that dials the Unix socket it is given
const DIAL: &str = "perl -MIO::Socket::UNIX -e 'IO::Socket::UNIX->new(Peer => $ARGV[0]) or exit 7'";

/// A host off the list is refused by the proxy, and a Unix socket the policy
/// does not list is not reached
fn a_host_and_a_socket() {
    let world = World::new();
    let Some(sandbox) = world.sandbox("a_host_not_allowed_and_a_socket_not_listed_are_refused")
    else {
        return;
    };

    let out = world.run(
        &sandbox,
        "curl -sS -m 20 -o /dev/null -w '%{http_connect}' https://example.com/",
    );
    let code = String::from_utf8_lossy(&out.stdout);
    assert!(
        refused_by_the_proxy(&code),
        "example.com was not refused by the proxy: {code:?}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // In the project, which the PM reads, so the policy's sockets are what
    // refuses it and not a folder it cannot see
    let socket = world.project().join("s.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    let outside = Command::new("sh")
        .args(["-c", &format!("{DIAL} {}", socket.display())])
        .status()
        .unwrap();
    assert!(
        outside.success(),
        "a command outside it did not reach the socket"
    );
    let out = world.run(&sandbox, &format!("{DIAL} {}", socket.display()));
    assert_eq!(
        out.status.code(),
        Some(UNREACHED),
        "a socket outside was reached: {out:?}"
    );
}

/// Linux turns every Unix socket off whatever `allowUnixSockets` lists, so a
/// socket the policy lists is refused too. On macOS the same policy reaches it.
#[cfg(target_os = "linux")]
fn a_listed_socket() {
    let world = World::new();
    let socket = world.project().join("s.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    let (folder, state, sessions_root) = (world.project(), world.state(), world.sessions_root());
    let mut config = kelpie_engine::sandbox::policy(
        &World::harness(),
        &project_in(&folder, &sessions_root),
        &state,
    )
    .expect("the policy is built");
    let listed = vec![socket.display().to_string()];
    config.network.allow_unix_sockets = Some(listed.clone());
    assert_eq!(
        config.network.allow_unix_sockets,
        Some(listed),
        "the socket is listed"
    );
    fs::create_dir_all(world.state()).unwrap();
    let sandbox = crabpot::Sandbox::start(&config).expect("the sandbox starts");
    let run = |script: &str| {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]).current_dir(&folder);
        sandbox
            .command(&command)
            .expect("the command goes in the sandbox")
            .output()
            .expect("the sandbox runs sh")
    };
    if !host_can_run(
        "a_socket_the_policy_lists_is_still_refused_on_linux",
        &run("true"),
    ) {
        return;
    }

    // The control: a command outside the sandbox reaches it, so the refusal is
    // the sandbox's
    let outside = Command::new("sh")
        .args(["-c", &format!("{DIAL} {}", socket.display())])
        .status()
        .unwrap();
    assert!(
        outside.success(),
        "a command outside it did not reach the socket"
    );

    let out = run(&format!("{DIAL} {}", socket.display()));

    assert_eq!(
        out.status.code(),
        Some(UNREACHED),
        "the listed socket was reached: {out:?}"
    );
}

/// The cases, each by the short tag its test passes to [`in_own_home`]
const CASES: &[(&str, fn())] = &[
    ("own", harness_files_are_refused),
    ("nest", a_nested_claude_folder),
    ("read", the_pm_reads),
    ("net", a_host_and_a_socket),
    ("mask", super::a_denied_folder_under_an_allowed_one),
    #[cfg(target_os = "linux")]
    ("sock", a_listed_socket),
];

/// The first argument that makes the test binary run one case in its own home
pub const IN_OWN_HOME: &str = "--in-own-home";

/// What the binary exits with when its case skipped itself
const SKIP_EXIT: i32 = 3;

/// Runs the case `tag` and exits, for the copy of the binary [`in_own_home`] starts
pub fn run_case(tag: &str) -> ! {
    let case = CASES
        .iter()
        .find(|(name, _)| *name == tag)
        .unwrap_or_else(|| panic!("no case {tag}"));
    (case.1)();
    std::process::exit(if SKIPPED.load(Ordering::SeqCst) > 0 {
        SKIP_EXIT
    } else {
        0
    });
}

/// A folder of its own, removed when it drops
struct OwnHome(PathBuf);

impl Drop for OwnHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs the case `tag` in a copy of this binary whose home folder is one of its
/// own, and fails when it does
///
/// `tag` is short: a Unix socket's path has about a hundred bytes. A case that
/// needs a home of its own is in `CASES`, whichever file its own test is in.
pub(super) fn in_own_home(tag: &str) {
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("esc-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).expect("the home folder is made");
    // The path as the OS names it, since a sandbox compares real paths
    let home = OwnHome(fs::canonicalize(folder).expect("the home folder is there"));
    let status = Command::new(env::current_exe().unwrap())
        .args([IN_OWN_HOME, tag])
        .env("HOME", &home.0)
        .status()
        .expect("the case runs");
    if status.code() == Some(SKIP_EXIT) {
        SKIPPED.fetch_add(1, Ordering::SeqCst);
    } else {
        assert!(status.success(), "the case {tag} failed: {status:?}");
    }
}

/// Claude Code's and goose's own files are refused to the PM in any case and
/// spelling: the ones its project would carry, written, and the ones its home
/// would carry, read
pub fn harness_files_are_refused_in_any_case_and_spelling() {
    in_own_home("own");
}

/// A `.claude` folder below the project, or beside it, is refused as the
/// project's own is
pub fn a_nested_claude_folder_is_refused() {
    in_own_home("nest");
}

/// The PM reads its project, a repo beside it and its own state folder, and
/// nothing of another repo of the same home folder, private ones and their
/// `.env` files included
pub fn the_pm_reads_the_project_and_no_other_repo_of_the_home() {
    in_own_home("read");
}

/// A host off the list is refused by the proxy, and a Unix socket the policy
/// does not list is not reached
pub fn a_host_not_allowed_and_a_socket_not_listed_are_refused() {
    in_own_home("net");
}

/// Linux turns every Unix socket off whatever `allowUnixSockets` lists, so a
/// socket the policy lists is refused too. On macOS the same policy reaches it.
#[cfg(target_os = "linux")]
pub fn a_socket_the_policy_lists_is_still_refused_on_linux() {
    in_own_home("sock");
}
