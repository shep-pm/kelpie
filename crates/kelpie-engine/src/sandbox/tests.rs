use std::path::{Path, PathBuf};

use crabpot::Config;
use serde_json::json;

use super::*;
use crate::session::Harness;

const PROJECT: &str = "/work/kelpie";
const BESIDE: &str = "/work";
const SESSIONS: &str = "/state/projects";
const STATE: &str = "/state/projects/pm";
const INSTALL: &str = "/opt/claude/bin";

/// A harness whose model answers on `hosts`
fn on(hosts: &[&str]) -> Harness {
    let mut harness = Harness::new("agent");
    harness.hosts = hosts.iter().map(|host| (*host).to_owned()).collect();
    harness
}

/// A harness installed in `folders`, which the session reads
fn installed(mut harness: Harness, folders: &[&str]) -> Harness {
    harness.installs = folders
        .iter()
        .map(|folder| (*folder).to_owned().into())
        .collect();
    harness
}

fn project<'a>(allowlist: &'a [String], reads: Reads) -> Project<'a> {
    Project {
        folder: Path::new(PROJECT),
        network_allowlist: allowlist,
        reads,
        sessions_root: Path::new(SESSIONS),
    }
}

/// The settings as crabpot reads them, from the same keys the policy is built on
fn read(settings: serde_json::Value) -> Config {
    let text = settings.to_string();
    Config::from_json(&text).unwrap_or_else(|error| panic!("{error}: {text}"))
}

/// What crabpot makes of `paths` as a list of its own
fn entries(paths: &[&str]) -> Config {
    read(json!({
        "network": {"allowedDomains": [], "deniedDomains": []},
        "filesystem": {
            "denyRead": paths, "allowRead": paths, "allowWrite": paths, "denyWrite": [],
        },
    }))
}

fn built(harness: &Harness, allowlist: &[String], reads: Reads) -> Config {
    policy(harness, &project(allowlist, reads), Path::new(STATE)).unwrap()
}

/// The folder this machine calls home, which the policy closes over
fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .expect("the OS names a home folder")
        .into()
}

/// Whether `config` denies `path` whole, as crabpot reads it
fn denies(config: &Config, path: &str) -> bool {
    let wanted = entries(&[path]).filesystem.deny_read;
    config.filesystem.deny_read.contains(&wanted[0])
}

/// Whether `config` denies `path` as a place to write
fn writes_denied(config: &Config, path: &str) -> bool {
    let wanted = read(json!({
        "network": {"allowedDomains": [], "deniedDomains": []},
        "filesystem": {"denyRead": [], "allowRead": [], "allowWrite": [], "denyWrite": [path]},
    }))
    .filesystem
    .deny_write;
    config.filesystem.deny_write.contains(&wanted[0])
}

/// Whether `config` opens `path` back inside what it closes
fn allows(config: &Config, path: &str) -> bool {
    let wanted = entries(&[path]).filesystem.allow_read.unwrap();
    let allowed = config.filesystem.allow_read.as_ref().unwrap();
    allowed.iter().any(|entry| wanted.contains(entry))
}

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

#[test]
fn read_anywhere_writes_only_the_state_dir() {
    let harness = installed(on(&["api.anthropic.com"]), &["/opt/claude"]);

    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&harness, &["docs.rs".to_owned()], reads);

        assert_eq!(
            config.filesystem.allow_write,
            entries(&[STATE]).filesystem.allow_write,
            "with {reads:?}"
        );
        for log in ["~/.npm/_logs", "~/.claude/debug"] {
            assert!(
                writes_denied(&config, log),
                "{log:?} is writable with {reads:?}"
            );
        }
    }
}

#[test]
fn reads_stay_in_the_project_by_default() {
    let harness = installed(on(&[]), &[INSTALL, PROJECT]);

    let config = built(&harness, &[], Reads::Project);

    assert_eq!(
        config.filesystem.allow_read,
        entries(&[PROJECT, BESIDE, STATE, INSTALL])
            .filesystem
            .allow_read
    );
    assert!(denies(&config, "~"), "the home folder is closed");
}

#[test]
fn read_anywhere_reads_outside_the_project() {
    let harness = installed(on(&[]), &[INSTALL]);

    let config = built(&harness, &[], Reads::Anywhere);

    assert!(!denies(&config, "~"), "the home folder is closed");
    // Only what the policy closes itself is opened back. The folders beside the
    // project are named nowhere, since nothing closes them.
    assert_eq!(
        config.filesystem.allow_read,
        entries(&[PROJECT, STATE, INSTALL]).filesystem.allow_read
    );
}

#[test]
fn every_other_sessions_state_stays_out_of_reach() {
    for reads in [Reads::Project, Reads::Anywhere] {
        let config = built(&on(&[]), &[], reads);

        assert!(denies(&config, &format!("{SESSIONS}/**")), "{reads:?}");
        assert!(
            allows(&config, STATE),
            "the session cannot read its own state folder with {reads:?}"
        );
    }
}

/// A project at the home folder is the one shape where an allow would open back
/// the logins the policy denies under it
#[test]
fn a_project_at_the_home_folder_keeps_the_logins_under_it_closed() {
    let home = home();
    let text = home.to_string_lossy().into_owned();

    for reads in [Reads::Project, Reads::Anywhere] {
        let project = Project {
            folder: &home,
            network_allowlist: &[],
            reads,
            sessions_root: Path::new(SESSIONS),
        };

        let config = policy(&on(&[]), &project, Path::new(STATE)).unwrap();

        assert!(
            !allows(&config, &text),
            "the home folder is opened back with {reads:?}"
        );
        assert!(denies(&config, "~/.ssh/**"), "{reads:?}");
    }
}

#[test]
fn the_folders_beside_a_project_are_left_out_when_they_would_open_the_root() {
    let folder = Path::new("/kelpie");
    let project = Project {
        folder,
        network_allowlist: &[],
        reads: Reads::Project,
        sessions_root: Path::new(SESSIONS),
    };

    let config = policy(&on(&[]), &project, Path::new(STATE)).unwrap();

    assert_eq!(
        config.filesystem.allow_read,
        entries(&["/kelpie", STATE]).filesystem.allow_read
    );
}

#[test]
fn the_network_is_the_harness_host_plus_the_allowlist() {
    let allowlist = ["docs.rs".to_owned(), "*.crates.io".to_owned()];

    let config = built(
        &on(&["api.anthropic.com", "localhost"]),
        &allowlist,
        Reads::Project,
    );

    let hosts: Vec<&str> = config
        .network
        .allowed_domains
        .iter()
        .map(|entry| entry.as_str())
        .collect();
    assert_eq!(
        hosts,
        ["api.anthropic.com", "localhost", "docs.rs", "*.crates.io"]
    );
    assert!(config.network.denied_domains.is_empty());
    assert_eq!(
        config.network.strict_allowlist,
        Some(true),
        "a host off the list is refused, not asked about"
    );
}

#[test]
fn a_host_the_harness_and_the_allowlist_both_name_is_listed_once() {
    let allowlist = ["api.anthropic.com".to_owned()];

    let config = built(&on(&["api.anthropic.com"]), &allowlist, Reads::Project);

    assert_eq!(config.network.allowed_domains.len(), 1);
}

#[test]
fn a_domain_crabpot_cannot_read_is_refused_naming_its_place() {
    let allowlist = ["not a domain".to_owned()];

    let error = policy(
        &on(&[]),
        &project(&allowlist, Reads::Project),
        Path::new(STATE),
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("allowedDomains.0"),
        "the refusal names the entry's place: {error}"
    );
}
