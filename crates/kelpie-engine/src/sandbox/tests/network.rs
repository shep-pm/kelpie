use std::path::Path;

use crabpot::Config;
use serde_json::json;

use super::*;
use crate::tools::Door;

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

/// The settings with `door` named, as crabpot reads them
fn with_door(door: &Door, reads: Reads) -> Config {
    policy_with_door(
        &on(&["api.anthropic.com"]),
        &project(&[], reads),
        Path::new(STATE),
        Some(door),
    )
    .unwrap()
}

#[test]
fn a_socket_door_is_the_one_unix_socket_the_policy_allows() {
    let door = Door::parse("/data/kelpie/0123.sock").unwrap();

    let config = with_door(&door, Reads::Project);

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

    let config = with_door(&door, Reads::Project);

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

    let config = with_door(&Door::parse(door).unwrap(), Reads::Project);

    assert_eq!(config.network.allow_local_binding, None);
}

#[test]
fn no_door_grants_no_socket_and_no_address() {
    let config = built(&on(&[]), &[], Reads::Project);

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

    let config = with_door(&door, Reads::Project);

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
