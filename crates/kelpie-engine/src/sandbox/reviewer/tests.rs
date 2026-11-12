use std::path::Path;

use crabpot::Config;
use serde_json::json;

use super::*;
use crate::sandbox::Reads;
use crate::session::Harness;

const PROJECT: &str = "/work/kelpie";
const WORKTREE: &str = "/data/kelpie/worktrees/74";
const CLONE_GIT_DIR: &str = "/data/kelpie/worktrees/74.git";
const COMMON_DIR: &str = "/work/kelpie/.git";
const STATE: &str = "/data/kelpie/state/74-review";
const TESTS: &str = "/data/kelpie/state/74-review/tests";
const KELPIE_HOME: &str = "/data/kelpie";
const SESSIONS: &str = "/data/kelpie/projects";

/// A harness whose model answers on `hosts`
fn on(hosts: &[&str]) -> Harness {
    let mut harness = Harness::new("agent");
    harness.hosts = hosts.iter().map(|host| (*host).to_owned()).collect();
    harness
}

fn project<'a>(allowlist: &'a [String], read_anywhere: bool) -> Project<'a> {
    Project {
        folder: Path::new(PROJECT),
        network_allowlist: allowlist,
        reads: if read_anywhere {
            Reads::Anywhere
        } else {
            Reads::Project
        },
        sessions_root: Path::new(SESSIONS),
    }
}

fn reviewer(tests: Option<&Path>) -> Reviewer<'_> {
    Reviewer {
        worktree: Path::new(WORKTREE),
        git_common_dir: Path::new(COMMON_DIR),
        state_dir: Path::new(STATE),
        tests,
        kelpie_home: Path::new(KELPIE_HOME),
    }
}

fn built(harness: &Harness, allowlist: &[String], tests: Option<&Path>) -> Config {
    policy(harness, &project(allowlist, false), &reviewer(tests)).unwrap()
}

/// What crabpot makes of `paths` as a list of its own
fn entries(paths: &[&str]) -> Config {
    let settings = json!({
        "network": {"allowedDomains": [], "deniedDomains": []},
        "filesystem": {
            "denyRead": paths, "allowRead": paths, "allowWrite": paths, "denyWrite": paths,
        },
    });
    Config::from_json(&settings.to_string()).unwrap()
}

#[test]
fn the_worktree_and_both_git_dirs_are_read_only() {
    let tests = Path::new(TESTS);

    for read_anywhere in [false, true] {
        let config = policy(
            &on(&["api.anthropic.com"]),
            &project(&[], read_anywhere),
            &reviewer(Some(tests)),
        )
        .unwrap();

        let read = config.filesystem.allow_read.unwrap();
        let folders = entries(&[WORKTREE, CLONE_GIT_DIR, COMMON_DIR]);
        let named = folders.filesystem.allow_read.unwrap();
        for (entry, path) in named.iter().zip([WORKTREE, CLONE_GIT_DIR, COMMON_DIR]) {
            assert!(
                read.contains(entry),
                "{path} is not read with read_anywhere {read_anywhere}"
            );
            assert!(
                !config.filesystem.allow_write.contains(entry),
                "{path} is written with read_anywhere {read_anywhere}"
            );
        }
        assert_eq!(
            config.filesystem.allow_write,
            entries(&[STATE, TESTS]).filesystem.allow_write,
            "a reviewer wrote somewhere besides its state and tests folders"
        );
    }
}

#[test]
fn the_tests_folder_is_writable_only_when_given() {
    let tests = Path::new(TESTS);

    let with = built(&on(&[]), &[], Some(tests));
    let without = built(&on(&[]), &[], None);

    assert_eq!(
        with.filesystem.allow_write,
        entries(&[STATE, TESTS]).filesystem.allow_write
    );
    assert_eq!(
        without.filesystem.allow_write,
        entries(&[STATE]).filesystem.allow_write,
        "the state folder alone, where the project has no test command"
    );
    let named = entries(&[TESTS]).filesystem.allow_read.unwrap();
    assert!(
        with.filesystem.allow_read.unwrap().contains(&named[0]),
        "the tests folder is written and not read"
    );
}

#[test]
fn the_network_is_the_harnesses_hosts_and_no_github() {
    let allowlist = ["docs.rs".to_owned()];

    let config = built(&on(&["api.anthropic.com"]), &allowlist, None);

    let hosts: Vec<&str> = config
        .network
        .allowed_domains
        .iter()
        .map(|entry| entry.as_str())
        .collect();
    assert_eq!(hosts, ["api.anthropic.com", "docs.rs"]);
    assert!(config.network.denied_domains.is_empty());
    assert_eq!(config.network.strict_allowlist, Some(true));
}

#[test]
fn no_tools_door_or_lease_socket_is_named() {
    let config = built(&on(&[]), &[], Some(Path::new(TESTS)));

    assert_eq!(config.network.allow_unix_sockets, None);
    assert_eq!(config.network.allow_tcp, None);
    assert_eq!(config.network.allow_local_binding, None);
}

#[test]
fn kelpies_data_folder_is_closed_but_for_the_state_folder() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let mut harness = on(&[]);
    harness.installs = vec!["/opt/claude/bin".into()];

    for read_anywhere in [false, true] {
        let config = compose(
            &harness,
            &project(&[], read_anywhere),
            &reviewer(None),
            Some(home),
        )
        .unwrap();

        let closed = entries(&[&format!("{KELPIE_HOME}/**")]);
        for entry in &closed.filesystem.deny_read {
            assert!(
                config.filesystem.deny_read.contains(entry),
                "{entry:?} is open with read_anywhere {read_anywhere}"
            );
        }
        let state = entries(&[STATE]).filesystem.allow_read.unwrap();
        assert!(
            config.filesystem.allow_read.unwrap().contains(&state[0]),
            "the state folder is not read with read_anywhere {read_anywhere}"
        );
    }
}
