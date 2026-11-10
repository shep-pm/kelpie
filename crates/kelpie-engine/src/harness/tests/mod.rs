use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::HarnessError;
use super::claude::{self, LOGIN};
use super::goose::{self, Provider};
use crate::login::Store;
use crate::run::RunId;
use crate::session::Harness;

/// The installed bridge command every test hands to `harness`
const BRIDGE: &str = "/kelpie-data/bridges/claude-acp/0.101.1/claude-agent-acp";

/// The session's state dir every test hands to `harness`
const STATE: &str = "/kelpie-data/projects/1/pm";

/// The login every test that needs one hands over
const TOKEN: &str = "test-token";

/// How long the hand-run test waits for one turn of a real session
const TURN: Duration = Duration::from_secs(180);

/// A folder of its own, named after this test's run
fn temp(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kelpie-{prefix}-{}", RunId::new().unwrap()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A folder holding a runnable `cmd`, as the person's install would
fn installed_with(cmd: &str) -> PathBuf {
    let dir = temp("harness-installed");
    let program = dir.join(cmd);
    fs::write(&program, "#!/bin/sh\necho kelpie harness fixture\n").unwrap();
    #[cfg(unix)]
    {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&program, Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// A folder holding a runnable `claude`, as the person's install would
fn installed() -> PathBuf {
    installed_with("claude")
}

/// A folder holding a runnable `goose`, as a person's install would
fn goose_installed() -> PathBuf {
    installed_with("goose")
}

/// A folder holding a `cmd` that cannot be run
fn not_runnable(cmd: &str) -> PathBuf {
    let dir = temp("harness-not-runnable");
    fs::write(dir.join(cmd), "not a program\n").unwrap();
    dir
}

/// `dirs` as the whole `PATH`
fn path_of(dirs: impl IntoIterator<Item: AsRef<OsStr>>) -> OsString {
    env::join_paths(dirs).unwrap()
}

/// A store that holds `TOKEN` as Claude Code's login
fn logged_in() -> Store {
    let store = Store::memory();
    store.set(LOGIN, TOKEN).unwrap();
    store
}

/// Build a harness with `path` as the `PATH` it searches, and the person's own
/// global folder left out
fn build(path: &OsStr, logins: &Store) -> Result<Harness, HarnessError> {
    claude::harness_on_path(BRIDGE.into(), Path::new(STATE), logins, path, None, false)
}

/// The provider a test names, with its host when it has one
fn provider(name: &str, host: Option<&str>) -> Provider {
    Provider {
        name: name.to_owned(),
        host: host.map(str::to_owned),
    }
}

/// Build the harness of `cmd` with `path` as the `PATH` it searches
fn build_on(path: &OsStr, provider: &Provider) -> Result<Harness, HarnessError> {
    goose::harness_on_path(Path::new(STATE), provider, path, None, false)
}

/// The value `harness` sets `name` to
fn env_of(harness: &Harness, name: &str) -> Option<OsString> {
    harness
        .env
        .iter()
        .find(|(known, _)| known == name)
        .map(|(_, value)| value.clone())
}

mod bridge_pin;
mod build;
mod global_folder;
mod hand_run;
mod installs;
mod own_files;
