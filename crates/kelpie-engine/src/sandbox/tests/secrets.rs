use std::fs;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::*;

#[test]
fn read_anywhere_still_denies_every_harness_login() {
    let logins = [
        "~/.claude/.credentials.json",
        "~/.claude.json",
        "~/.codex/**",
        "~/.pi/**",
        "~/.config/goose/**",
        "~/.local/share/goose/**",
        "~/.local/state/goose/**",
        "~/Library/Application Support/Block/goose/**",
    ];

    // Whichever harness the session runs on, and whatever the project reads
    for harness in [on(&["api.anthropic.com"]), on(&["localhost"])] {
        for reads in [Reads::Project, Reads::Anywhere] {
            let config = built(&harness, &[], reads);
            for login in logins {
                assert!(
                    denies(&config, login),
                    "{login:?} is readable with {reads:?}"
                );
            }
        }
    }
}

#[test]
fn the_policy_denies_the_secrets_a_tool_keeps_in_the_home_folder() {
    let secrets = [
        "~/.ssh/**",
        "~/.aws/**",
        "~/.gnupg/**",
        "~/.docker/**",
        "~/.config/gh/**",
        "~/.netrc",
        "~/.git-credentials",
        "~/.npmrc",
        "~/.cargo/credentials",
        "~/.cargo/credentials.toml",
        "~/Library/Keychains/**",
    ];

    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&on(&[]), &[], reads);
        for secret in secrets {
            assert!(
                denies(&config, secret),
                "{secret:?} is readable with {reads:?}"
            );
        }
    }
}

/// The home folder, the folder holding it, the folder holding every session's
/// state, and a folder the policy denies its reads, each reachable through a
/// link as well as by name
///
/// As `/home` linked to `/data/home` spells one folder two ways, `/home/<name>`
/// and `/data/home/<name>`. The scratch folder is removed when this drops, links
/// included: a link is removed as the name it is, not as what it names.
#[cfg(unix)]
pub(super) struct Linked {
    scratch: PathBuf,
}

#[cfg(unix)]
impl Linked {
    pub(super) fn new(test: &str) -> Self {
        let scratch = std::env::temp_dir().join(format!(
            "kelpie-sandbox-linked-{test}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&scratch);
        for folder in ["real/projects", "project"] {
            fs::create_dir_all(scratch.join(folder)).expect("the scratch folders are made");
        }
        for (link, named) in [
            ("home", home()),
            ("above", above_home()),
            ("state", scratch.join("real")),
            ("secret", home().join(".ssh")),
        ] {
            std::os::unix::fs::symlink(named, scratch.join(link)).expect("the link is made");
        }
        Self { scratch }
    }

    /// The home folder, named through a link
    pub(super) fn home(&self) -> PathBuf {
        self.scratch.join("home")
    }

    /// The folder holding the home folder, named through a link
    pub(super) fn above(&self) -> PathBuf {
        self.scratch.join("above")
    }

    /// The folder holding every session's state, named through a link
    pub(super) fn sessions_root(&self) -> PathBuf {
        self.scratch.join("state/projects")
    }

    /// That same folder, named as the OS names it
    pub(super) fn real_sessions_root(&self) -> PathBuf {
        self.scratch.join("real/projects")
    }

    /// A folder the policy denies its reads, named through a link
    pub(super) fn secret(&self) -> PathBuf {
        self.scratch.join("secret")
    }

    /// A folder of its own, holding nothing the policy closes
    pub(super) fn project(&self) -> PathBuf {
        self.scratch.join("project")
    }
}

#[cfg(unix)]
impl Drop for Linked {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.scratch);
    }
}

/// The folder holding the home folder, such as `/Users`
fn above_home() -> PathBuf {
    home()
        .parent()
        .expect("the OS puts the home folder in one")
        .to_path_buf()
}

/// The topmost folder holding the home folder, which is `/` on macOS and Linux
fn root_of_home() -> PathBuf {
    let mut folder = home();
    while let Some(parent) = folder.parent() {
        folder = parent.to_path_buf();
    }
    folder
}

/// A session whose project folder is `wanted` is refused, naming it
///
/// Its state folder is under the machine's temp folder, so a refusal that came
/// too late would leave it behind.
fn refuses(name: &str, wanted: &Path, reads: Reads) {
    let scratch = std::env::temp_dir()
        .join("kelpie-sandbox-refused")
        .join(name);
    let _ = fs::remove_dir_all(&scratch);
    let (sessions_root, state) = (scratch.join("projects"), scratch.join("projects/pm"));
    let allowlist: Vec<String> = Vec::new();
    let project = Project {
        folder: wanted,
        network_allowlist: &allowlist,
        reads,
        sessions_root: &sessions_root,
    };

    let error = Sandbox::start(&on(&[]), &project, &state)
        .expect_err("a project that holds the home folder is refused");
    let (wanted, message) = (wanted.to_string_lossy().into_owned(), error.to_string());

    let SandboxError::ProjectAtHome { folder } = &error else {
        panic!("the refusal is not the project folder's, with {reads:?}: {error}");
    };
    assert_eq!(folder.to_string_lossy(), wanted, "{reads:?}");
    assert!(
        message.contains(&wanted) && message.contains("move the project into a folder of its own"),
        "the refusal names neither the folder nor what to do: {message}"
    );
    assert!(
        !state.exists(),
        "the state folder was made, so the session started with {reads:?}: {state:?}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn a_project_at_the_home_folder_is_refused() {
    let home = home();

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("at-home", &home, reads);
    }
}

#[test]
fn a_project_above_the_home_folder_is_refused() {
    let above = above_home();

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("above-home", &above, reads);
    }
}

#[test]
fn a_project_at_the_root_is_refused() {
    let root = root_of_home();
    if cfg!(unix) {
        assert_eq!(root, PathBuf::from("/"), "the root the refusal names");
    }

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("at-root", &root, reads);
    }
}

/// A project that reaches the home folder through a link is the home folder
///
/// `Sandbox::start` compares the folder a project really names, as crabpot
/// matches it, and not the folder as it was spelled.
#[cfg(unix)]
#[test]
fn a_project_that_links_to_the_home_folder_is_refused() {
    let linked = Linked::new("at-home");

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("links-to-home", &linked.home(), reads);
    }
}

#[cfg(unix)]
#[test]
fn a_project_that_links_to_the_folder_holding_home_is_refused() {
    let linked = Linked::new("above-home");

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses("links-above-home", &linked.above(), reads);
    }
}

/// A session whose project folder `wanted` is at or inside the denied folder
/// `denied` is refused, naming both
///
/// Its state folder is under the machine's temp folder, so a refusal that came
/// too late would leave it behind.
fn refuses_denied(name: &str, wanted: &Path, denied: &Path, reads: Reads) {
    let scratch = std::env::temp_dir()
        .join("kelpie-sandbox-refused")
        .join(name);
    let _ = fs::remove_dir_all(&scratch);
    let (sessions_root, state) = (scratch.join("projects"), scratch.join("projects/pm"));
    let allowlist: Vec<String> = Vec::new();
    let project = Project {
        folder: wanted,
        network_allowlist: &allowlist,
        reads,
        sessions_root: &sessions_root,
    };

    let error = Sandbox::start(&on(&[]), &project, &state)
        .expect_err("a project at or inside a denied folder is refused");
    let (wanted, message) = (wanted.to_string_lossy().into_owned(), error.to_string());

    let SandboxError::ProjectInDenied {
        folder,
        denied: held,
    } = &error
    else {
        panic!("the refusal is not the project folder's, with {reads:?}: {error}");
    };
    assert_eq!(folder.to_string_lossy(), wanted, "{reads:?}");
    assert_eq!(held, &resolved(denied), "{reads:?}");
    let held = held.to_string_lossy().into_owned();
    assert!(
        message.contains(&wanted)
            && message.contains(&held)
            && message.contains("move the project out of"),
        "the refusal names neither folder nor what to do: {message}"
    );
    assert!(
        !state.exists(),
        "the state folder was made, so the session started with {reads:?}: {state:?}"
    );
    let _ = fs::remove_dir_all(&scratch);
}

#[test]
fn a_project_inside_a_login_folder_is_refused() {
    let denied = home().join(".codex");
    let folder = denied.join("skills/x");

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses_denied("inside-codex", &folder, &denied, reads);
    }
}

#[test]
fn a_project_inside_a_secret_folder_is_refused() {
    let secret = home().join(".ssh");
    let folder = secret.join("keys/x");

    for reads in [Reads::Project, Reads::Anywhere] {
        refuses_denied("inside-ssh", &folder, &secret, reads);
    }

    // And the same folder reached through a link, which is only refused when
    // the folder the link names is there to be reached
    #[cfg(unix)]
    {
        let secret = SecretFolder::new();
        let linked = Linked::new("inside-secret");

        for reads in [Reads::Project, Reads::Anywhere] {
            refuses_denied("links-into-ssh", &linked.secret(), &secret.folder, reads);
        }
    }
}

/// The folder `~/.ssh`, made when this machine has none and taken back when this
/// made it
///
/// A project reached through a link is only refused when the folder the link
/// names is there to be reached, so the test that links to it needs it to be
/// one. It is made private, as a person's own would be.
#[cfg(unix)]
struct SecretFolder {
    folder: PathBuf,
    made: bool,
}

#[cfg(unix)]
impl SecretFolder {
    fn new() -> Self {
        let folder = home().join(".ssh");
        let made = !folder.try_exists().expect("the home folder is read");
        if made {
            fs::create_dir_all(&folder)
                .expect("the home folder takes the denied folder the test needs");
            fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))
                .expect("the denied folder is private");
        }
        Self { folder, made }
    }
}

#[cfg(unix)]
impl Drop for SecretFolder {
    fn drop(&mut self) {
        if self.made {
            let _ = fs::remove_dir(&self.folder);
        }
    }
}

/// The refusal `Sandbox::start_worker` gives a project at `wanted`, which it
/// gives before the settings are built or the state folder made
///
/// The worker's folders are under the machine's temp folder, so a refusal that
/// came too late would leave them behind.
fn refuses_worker(name: &str, wanted: &Path) -> SandboxError {
    let scratch = std::env::temp_dir()
        .join("kelpie-sandbox-refused")
        .join(name);
    let _ = fs::remove_dir_all(&scratch);
    let (sessions_root, state) = (scratch.join("projects"), scratch.join("state"));
    let (worktree, git_dir, common) = (
        scratch.join("worktree"),
        scratch.join("worktree.git"),
        scratch.join("project/.git"),
    );
    let allowlist: Vec<String> = Vec::new();
    let project = Project {
        folder: wanted,
        network_allowlist: &allowlist,
        reads: Reads::Project,
        sessions_root: &sessions_root,
    };
    let worker = Worker {
        worktree: &worktree,
        git_dir: &git_dir,
        git_common_dir: &common,
        state_dir: &state,
        kelpie_home: &scratch,
        door: None,
        lease: None,
    };

    let error = Sandbox::start_worker(&on(&[]), &project, &worker)
        .expect_err("a worker whose project is not a folder of its own is refused");
    assert!(
        !state.exists(),
        "the worker's state folder was made, so it started: {state:?}"
    );
    let _ = fs::remove_dir_all(&scratch);
    error
}

#[test]
fn a_worker_in_the_home_folder_is_refused() {
    let home = home();

    let error = refuses_worker("worker-at-home", &home);
    let SandboxError::ProjectAtHome { folder } = &error else {
        panic!("the refusal is not the project folder's: {error}");
    };
    assert_eq!(folder, &home);
    assert!(
        error
            .to_string()
            .contains("move the project into a folder of its own"),
        "the refusal says nothing about what to do: {error}"
    );
}

#[test]
fn a_worker_inside_a_login_folder_is_refused() {
    let denied = home().join(".codex");
    let folder = denied.join("skills/x");

    let error = refuses_worker("worker-inside-codex", &folder);
    let SandboxError::ProjectInDenied {
        folder: named,
        denied: held,
    } = &error
    else {
        panic!("the refusal is not the project folder's: {error}");
    };
    assert_eq!(
        named, &folder,
        "the refusal names the folder as it was given"
    );
    assert_eq!(
        held,
        &resolved(&denied),
        "the refusal names the denied folder"
    );
}
