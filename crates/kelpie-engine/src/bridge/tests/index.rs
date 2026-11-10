//! Reading the registry's index into a pin, and what makes an entry unusable
//!
//! `bridge/registry.rs`

use std::collections::BTreeMap;

use super::*;
#[test]

fn the_index_gives_a_pin_for_the_stable_version() {
    let server = Server::start(Duration::ZERO);

    let index = registry::fetch(&server.url(REGISTRY)).unwrap();

    let pin = index.bridge(ID).expect("claude-acp is in the index");
    assert_eq!((pin.id, pin.registry_id, pin.version), (ID, ID, "0.102.0"));
    assert_eq!(pin.platforms.len(), PLATFORMS.len());
    let (_, arm_mac) = pin
        .platforms
        .iter()
        .find(|(platform, _)| *platform == Platform::MacOsArm64)
        .expect("the index has macOS on arm64");
    assert_eq!(
        *arm_mac,
        Download {
            url: server.url(ZIP),
            sha256: hash_file(&fixture_path(ZIP)).unwrap(),
            cmd: CMD.to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
        },
        "the command is the file's name, without the registry's ./"
    );
    assert!(
        index.bridge("codex-acp").is_none(),
        "an agent with no binary distribution has no pin"
    );
}
#[test]

fn a_malformed_args_entry_gives_no_pin() {
    let server = Server::start(Duration::ZERO);

    let index = registry::fetch(&server.url(REGISTRY)).unwrap();

    // Both fixture entries carry an archive for every platform kelpie ships on,
    // so the shape of what they carry is the only reason kelpie has no pin
    assert!(
        index.bridge("malformed-args").is_none(),
        "args that are not a list were read"
    );
    assert!(
        index.bridge("malformed-env").is_none(),
        "env that is not a map was read"
    );
    assert!(
        index.bridge(GOOSE).is_some(),
        "a well-shaped entry was left out too"
    );
}

#[test]

fn a_binary_pin_carries_its_args_and_env() {
    let server = Server::start(Duration::ZERO);

    let index = registry::fetch(&server.url(REGISTRY)).unwrap();

    let pin = index.bridge(GOOSE).expect("goose is in the index");
    let (_, archive) = pin
        .platforms
        .iter()
        .find(|(platform, _)| *platform == this_platform())
        .expect("the index has a goose archive for this platform");
    assert_eq!(archive.args, [ACP.to_string()]);
    assert_eq!(
        archive.env,
        BTreeMap::from([(GOOSE_PROVIDER.to_string(), OLLAMA.to_string())]),
        "the entry's env was not read"
    );
}
