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
//! Each file the PM must not write is looked for on disk afterwards, outside
//! the sandbox, since the file system opens a file in any case and Unicode
//! spelling it folds. Each refusal sits beside a control that ran in the same
//! sandbox, since a command that never ran refuses everything.
//!
//! A child of `tests/sandbox.rs`, which is its own `main` (crabpot's Linux helper
//! is this binary run again).

use std::env;
use std::fs;
use std::io::Write as _;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use kelpie_engine::sandbox::{Project, Sandbox};
use kelpie_engine::session::Harness;

use super::host_can_run;

/// The logins of Claude Code and goose in the home folder, as a person has them
/// (`sandbox::denied`)
const LOGINS: [&str; 4] = [
    ".claude.json",
    ".claude/.credentials.json",
    ".config/goose/config.yaml",
    ".local/share/goose/sessions/sessions.db",
];

/// A folder of its own in the home folder, which the sandbox closes, with a
/// project and a sibling repo that the PM reads, a state folder it writes, and
/// another repo that it reads none of
///
/// In the home folder because only there does the policy close anything: its
/// reads are the project and the folders beside it, with `~` denied under them.
/// Removed when it drops, with the login files it had to make.
struct World {
    root: PathBuf,
    /// What this world made in the home folder to stand in for a login that
    /// the host does not have, files and the folders it made for them
    made: Vec<PathBuf>,
}

impl World {
    /// `tag` is kept short: a Unix socket's path has about a hundred bytes
    fn new(tag: &str) -> Self {
        let root = home().join(format!(".kelpie-escapes-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for folder in [
            "work/kelpie/src",
            "work/sibling/src",
            "state",
            "other-repo/src",
            "other-repo/.git",
        ] {
            fs::create_dir_all(root.join(folder)).expect("the world's folders are made");
        }
        let root = fs::canonicalize(&root).expect("the world is there");
        let world = Self {
            root,
            made: Vec::new(),
        };
        let files = [
            ("work/kelpie/src/lib.rs", "the project's own\n"),
            ("work/sibling/src/lib.rs", "a sibling's own\n"),
            ("state/notes.md", "the PM's own\n"),
            ("other-repo/.env", "SECRET=1\n"),
            ("other-repo/src/lib.rs", "private work\n"),
            ("other-repo/.git/config", "[remote]\n"),
        ];
        for (path, text) in files {
            fs::write(world.path(path), text).expect("the world's files are written");
        }
        world
    }

    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    fn project(&self) -> PathBuf {
        self.path("work/kelpie")
    }

    fn state(&self) -> PathBuf {
        self.path("state")
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
        let folder = self.project();
        let sandbox = Sandbox::start(&Self::harness(), &project_in(&folder), &self.state())
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

    /// Asserts `script` wrote nothing at `written`, under the world's root
    fn refused(&self, sandbox: &Sandbox, script: &str, written: &str) {
        let out = self.run(sandbox, script);
        let path = self.path(written);
        assert!(
            !path.exists() && path.symlink_metadata().is_err(),
            "{script} wrote {written}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Puts a login of the home folder in place when the host has none, and
    /// returns what it holds
    fn login(&mut self, relative: &str) -> Vec<u8> {
        let path = home().join(relative);
        if !path.exists() {
            // The folders it needs that are missing, outermost first
            let mut missing: Vec<&Path> = path
                .parent()
                .into_iter()
                .flat_map(Path::ancestors)
                .take_while(|folder| !folder.exists())
                .collect();
            missing.reverse();
            for folder in missing {
                fs::create_dir(folder).expect("a login's folder is made");
                self.made.push(folder.to_path_buf());
            }
            let mut file = fs::File::options()
                .write(true)
                .create_new(true)
                .open(&path)
                .expect("a login stand-in is made");
            writeln!(file, "kelpie-escapes-secret-{}", std::process::id()).unwrap();
            self.made.push(path.clone());
        }
        fs::read(&path).unwrap_or_default()
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        // Files before the folders that held them, innermost folder first
        for path in self.made.iter().rev() {
            let _ = fs::remove_file(path).or_else(|_| fs::remove_dir(path));
        }
    }
}

/// A project in `folder`, with the PM's own settings
fn project_in(folder: &Path) -> Project<'_> {
    Project {
        folder,
        network_allowlist: &[],
        read_anywhere: false,
    }
}

fn home() -> PathBuf {
    let home = env::var_os("HOME").expect("HOME names the home folder the policy closes");
    fs::canonicalize(home).expect("the home folder is there")
}

/// Whether `output` holds `secret` anywhere
fn holds(output: &[u8], secret: &[u8]) -> bool {
    !secret.is_empty() && output.windows(secret.len()).any(|part| part == secret)
}

/// Claude Code's and goose's own files are refused to the PM in any case and
/// spelling: the ones its project would carry, written, and the ones its home
/// would carry, read
pub fn harness_files_are_refused_in_any_case_and_spelling() {
    let mut world = World::new("own");
    let secrets: Vec<(&str, Vec<u8>)> = LOGINS
        .iter()
        .map(|login| (*login, world.login(login)))
        .collect();
    let Some(sandbox) = world.sandbox("harness_files_are_refused_in_any_case_and_spelling") else {
        return;
    };
    world.control(&sandbox);

    // What the project would carry, which the PM does not write
    for (script, written) in [
        (
            "mkdir other && echo {} > other/settings.json && mv other .claude",
            "work/kelpie/.claude",
        ),
        ("mkdir .claude", "work/kelpie/.claude"),
        ("mkdir .CLAUDE", "work/kelpie/.claude"),
        ("echo {} > .mcp.json", "work/kelpie/.mcp.json"),
        ("echo {} > .MCP.JSON", "work/kelpie/.mcp.json"),
        ("echo {} > .mcp.j\u{17f}on", "work/kelpie/.mcp.json"),
        ("ln -s /etc/hosts .mcp.json", "work/kelpie/.mcp.json"),
        (
            "mkdir -p src/.CLAUDE/skills && echo x > src/.CLAUDE/skills/s.md",
            "work/kelpie/src/.claude",
        ),
        ("ln -s ../outside .claude", "work/kelpie/.claude"),
        (
            "mkdir other && echo x > other/config.yaml && mv other .goose",
            "work/kelpie/.goose",
        ),
        ("mkdir .goose", "work/kelpie/.goose"),
        ("mkdir .GOOSE", "work/kelpie/.goose"),
        ("echo x > .goosehints", "work/kelpie/.goosehints"),
        ("echo x > .GOOSEHINTS", "work/kelpie/.goosehints"),
        ("ln -s /etc/hosts .goosehints", "work/kelpie/.goosehints"),
        (
            "mkdir -p src/.Goose/recipes && echo x > src/.Goose/recipes/r.yaml",
            "work/kelpie/src/.goose",
        ),
        ("ln -s ../outside .goose", "work/kelpie/.goose"),
    ] {
        world.refused(&sandbox, script, written);
    }

    // Once the folders are there, what is written in them
    for folder in [".claude", ".goose"] {
        fs::create_dir_all(world.project().join(folder)).unwrap();
    }
    for (script, written) in [
        (
            "echo {} > .claude/settings.local.json",
            "work/kelpie/.claude/settings.local.json",
        ),
        (
            "echo {} > .CLAUDE/Settings.Local.json",
            "work/kelpie/.claude/settings.local.json",
        ),
        (
            "mkdir -p .claude/agents && echo x > .claude/agents/a.md",
            "work/kelpie/.claude/agents",
        ),
        (
            "cp /etc/hosts .claude/settings.json",
            "work/kelpie/.claude/settings.json",
        ),
        (
            "echo x > .goose/config.yaml",
            "work/kelpie/.goose/config.yaml",
        ),
        (
            "echo x > .Goose/Config.YAML",
            "work/kelpie/.goose/config.yaml",
        ),
    ] {
        world.refused(&sandbox, script, written);
    }

    // What its home would carry, which the PM does not read. A login the host
    // lacks was made for this, so each holds something to leak.
    let home = home();
    for (login, secret) in &secrets {
        let at = home.join(login);
        let upper = home.join(login.to_uppercase());
        let dotted = PathBuf::from(format!("{}/./{login}", home.display()));
        let doubled = PathBuf::from(format!("{}//{login}", home.display()));
        let folded = home.join(login.replace('s', "\u{17f}"));
        for spelling in [&at, &upper, &dotted, &doubled, &folded] {
            let out = world.run(&sandbox, &format!("cat '{}'", spelling.display()));
            assert!(
                !holds(&out.stdout, secret),
                "{} was read as {}: {}",
                login,
                spelling.display(),
                String::from_utf8_lossy(&out.stdout)
            );
        }
        // And not written, but only of a stand-in: a person's own login is not
        // put at risk by a sandbox that fails
        if world.made.contains(&at) {
            world.run(
                &sandbox,
                &format!("echo x >> '{}'; echo x > '{}'", at.display(), at.display()),
            );
            assert_eq!(&fs::read(&at).unwrap_or_default(), secret, "{login}");
        }
    }
}

/// A `.claude` folder below the project, or beside it, is refused as the
/// project's own is
pub fn a_nested_claude_folder_is_refused() {
    let world = World::new("nest");
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

    for (script, written) in [
        (
            "echo {} > src/.claude/settings.local.json",
            "work/kelpie/src/.claude/settings.local.json",
        ),
        (
            "mkdir -p src/.claude/agents && echo x > src/.claude/agents/a.md",
            "work/kelpie/src/.claude/agents",
        ),
        (
            "echo x > lib/.Goose/config.yaml",
            "work/kelpie/lib/.goose/config.yaml",
        ),
        (
            "echo {} > ../sibling/.claude/settings.local.json",
            "work/sibling/.claude/settings.local.json",
        ),
    ] {
        world.refused(&sandbox, script, written);
    }
}

/// The PM reads its project, a repo beside it and its own state folder, and
/// nothing of another repo of the same home folder, private ones and their
/// `.env` files included
pub fn the_pm_reads_the_project_and_no_other_repo_of_the_home() {
    let world = World::new("read");
    let Some(sandbox) = world.sandbox("the_pm_reads_the_project_and_no_other_repo_of_the_home")
    else {
        return;
    };

    for (path, text) in [
        ("work/kelpie/src/lib.rs", "the project's own\n"),
        ("work/sibling/src/lib.rs", "a sibling's own\n"),
        ("state/notes.md", "the PM's own\n"),
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
pub fn a_host_not_allowed_and_a_socket_not_listed_are_refused() {
    let world = World::new("net");
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
pub fn a_socket_the_policy_lists_is_still_refused_on_linux() {
    let world = World::new("sock");
    let socket = world.project().join("s.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    let folder = world.project();
    let mut config =
        kelpie_engine::sandbox::policy(&World::harness(), &project_in(&folder), &world.state())
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
