//! What a session reaches: the harness's hosts, the project's allowlist and the door

use std::collections::HashSet;

use serde_json::json;

use super::{Project, text};
use crate::session::Harness;
use crate::tools::Door;

/// The macOS services that answer for the keychain, which a session never asks
pub(in crate::sandbox) const KEYCHAIN_SERVICES: [&str; 2] =
    ["com.apple.securityd.xpc", "com.apple.SecurityServer"];

/// The `network` block of the settings a session on `harness` runs under, in
/// `project`, with kelpie's tools `door` named when it has one
///
/// The door is the one extra route out: its socket (macOS) or its address (Linux)
/// and nothing beside it. Only macOS has a keychain to ask, so only there are its
/// services denied.
pub(super) fn network(
    harness: &Harness,
    project: &Project<'_>,
    door: Option<&Door>,
) -> serde_json::Value {
    let mut network = json!({
        "allowedDomains": hosts(harness, project),
        "deniedDomains": [],
        // A host off the list is refused rather than asked about: the ask is
        // the sandbox request prompt's, not the proxy's
        "strictAllowlist": true,
    });
    if let Some(door) = door {
        if let Some(socket) = door.socket() {
            network["allowUnixSockets"] = json!([text(socket)]);
        }
        if let Some(address) = door.tcp() {
            network["allowTcp"] = json!([address.to_string()]);
        }
    }
    // Only macOS has a keychain to ask
    if cfg!(target_os = "macos") {
        network["denyMachLookup"] = json!(KEYCHAIN_SERVICES);
    }
    network
}

/// The harness's model hosts, then the project's allowlist, each once
pub(in crate::sandbox) fn hosts(harness: &Harness, project: &Project<'_>) -> Vec<String> {
    let mut seen = HashSet::new();
    harness
        .hosts
        .iter()
        .chain(project.network_allowlist)
        .filter(|host| seen.insert(host.as_str()))
        .cloned()
        .collect()
}
