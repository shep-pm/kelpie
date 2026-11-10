use std::path::Path;

use crabpot::Config;
use serde_json::json;

use super::*;
use crate::session::Harness;
use crate::tools::Door;

const PROJECT: &str = "/work/kelpie";
const STATE: &str = "/state/pm";

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
        read_anywhere,
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

fn built(harness: &Harness, allowlist: &[String], read_anywhere: bool) -> Config {
    policy(
        harness,
        &project(allowlist, read_anywhere),
        Path::new(STATE),
    )
    .unwrap()
}

#[test]
fn the_policy_denies_every_harness_login() {
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
    let wanted = entries(&logins).filesystem.deny_read;

    // Whichever harness the session runs on, and whatever the project reads
    for harness in [on(&["api.anthropic.com"]), on(&["localhost"])] {
        for read_anywhere in [false, true] {
            let config = built(&harness, &[], read_anywhere);
            for login in &wanted {
                assert!(
                    config.filesystem.deny_read.contains(login),
                    "{login:?} is readable with read_anywhere {read_anywhere}"
                );
            }
        }
    }
}

#[test]
fn the_policy_denies_the_secrets_a_tool_keeps_in_the_home_folder() {
    let secrets = entries(&["~/.ssh/**", "~/.aws/**", "~/.config/gh/**", "~/.netrc"]);

    let config = built(&on(&[]), &[], true);

    for secret in &secrets.filesystem.deny_read {
        assert!(config.filesystem.deny_read.contains(secret), "{secret:?}");
    }
}

#[test]
fn writes_are_the_state_dir_alone() {
    let mut harness = on(&["api.anthropic.com"]);
    harness.installs = vec!["/opt/claude".into()];

    for read_anywhere in [false, true] {
        let config = built(&harness, &["docs.rs".to_owned()], read_anywhere);

        assert_eq!(
            config.filesystem.allow_write,
            entries(&[STATE]).filesystem.allow_write
        );
        assert!(config.filesystem.deny_write.is_empty());
    }
}

#[test]
fn reads_are_the_project_the_folders_beside_it_and_the_state_dir() {
    let mut harness = on(&[]);
    harness.installs = vec!["/opt/claude/bin".into(), "/work/kelpie".into()];

    let config = built(&harness, &[], false);

    assert_eq!(
        config.filesystem.allow_read,
        Some(
            entries(&["/work/kelpie", "/work", STATE, "/opt/claude/bin"])
                .filesystem
                .allow_read
                .unwrap()
        )
    );
    assert!(
        config
            .filesystem
            .deny_read
            .contains(&entries(&["~"]).filesystem.deny_read[0]),
        "the home folder is closed"
    );
}

#[test]
fn a_project_that_reads_anywhere_closes_nothing_but_its_logins_and_secrets() {
    let config = built(&on(&[]), &[], true);

    assert!(
        !config
            .filesystem
            .deny_read
            .contains(&entries(&["~"]).filesystem.deny_read[0])
    );
    assert_eq!(config.filesystem.allow_read, Some(Vec::new()));
}

#[test]
fn the_folders_beside_a_project_are_left_out_when_they_would_open_the_root() {
    let folder = Path::new("/kelpie");
    let project = Project {
        folder,
        network_allowlist: &[],
        read_anywhere: false,
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

    let config = built(&on(&["api.anthropic.com", "localhost"]), &allowlist, false);

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

    let config = built(&on(&["api.anthropic.com"]), &allowlist, false);

    assert_eq!(config.network.allowed_domains.len(), 1);
}

#[test]
fn a_domain_crabpot_cannot_read_is_refused_naming_its_place() {
    let allowlist = ["not a domain".to_owned()];

    let error = policy(&on(&[]), &project(&allowlist, false), Path::new(STATE)).unwrap_err();

    assert!(
        error.to_string().contains("allowedDomains.0"),
        "the refusal names the entry's place: {error}"
    );
}

/// The settings with `door` named, as crabpot reads them
fn with_door(door: &Door, read_anywhere: bool) -> Config {
    policy_with_door(
        &on(&["api.anthropic.com"]),
        &project(&[], read_anywhere),
        Path::new(STATE),
        Some(door),
    )
    .unwrap()
}

#[test]
fn a_socket_door_is_the_one_unix_socket_the_policy_allows() {
    let door = Door::parse("/data/kelpie/0123.sock").unwrap();

    let config = with_door(&door, false);

    let wanted = read(json!({
        "network": {
            "allowedDomains": [], "deniedDomains": [],
            "allowUnixSockets": ["/data/kelpie/0123.sock"],
        },
        "filesystem": {"denyRead": [], "allowWrite": [], "denyWrite": []},
    }));
    assert_eq!(
        config.network.allow_unix_sockets,
        wanted.network.allow_unix_sockets
    );
    assert_eq!(config.network.allow_tcp, None, "no TCP grant beside it");
}

/// Crabpot refuses `allowTcp` where Seatbelt cannot narrow it to one host
#[cfg(target_os = "linux")]
#[test]
fn a_tcp_door_is_the_one_address_the_policy_allows() {
    let door = Door::parse("127.0.0.1:41999").unwrap();

    let config = with_door(&door, false);

    let wanted = read(json!({
        "network": {
            "allowedDomains": [], "deniedDomains": [],
            "allowTcp": ["127.0.0.1:41999"],
        },
        "filesystem": {"denyRead": [], "allowWrite": [], "denyWrite": []},
    }));
    assert_eq!(config.network.allow_tcp, wanted.network.allow_tcp);
    assert_eq!(config.network.allow_unix_sockets, None);
}

#[test]
fn a_door_does_not_open_local_binding() {
    let door = if cfg!(target_os = "linux") {
        "127.0.0.1:41999"
    } else {
        "/data/kelpie/0123.sock"
    };

    let config = with_door(&Door::parse(door).unwrap(), false);

    assert_eq!(config.network.allow_local_binding, None);
}

#[test]
fn no_door_grants_no_socket_and_no_address() {
    let config = built(&on(&[]), &[], false);

    assert_eq!(config.network.allow_unix_sockets, None);
    assert_eq!(config.network.allow_tcp, None);
}

#[test]
fn the_relay_binary_is_readable_when_the_home_folder_is_closed() {
    let door = Door::parse(if cfg!(target_os = "linux") {
        "127.0.0.1:41999"
    } else {
        "/data/kelpie/0123.sock"
    })
    .unwrap();
    let exe = std::env::current_exe().unwrap();

    let config = with_door(&door, false);

    assert!(
        config.filesystem.allow_read.unwrap().contains(
            &entries(&[&exe.to_string_lossy()])
                .filesystem
                .allow_read
                .unwrap()[0]
        ),
        "the harness starts it inside the sandbox"
    );
}
