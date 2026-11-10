use std::path::{Path, PathBuf};

use crabpot::Config;
use serde_json::json;

use super::*;
use crate::sandbox::policy::text;
use crate::session::Harness;

// The policy's tests, a file for each of its modules, and the Harness global folder
// setting, which has its own
mod global_folder;
mod network;
mod reads;
mod secrets;

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
fn installed(mut harness: Harness, folders: &[impl AsRef<Path>]) -> Harness {
    harness.installs = folders
        .iter()
        .map(|folder| folder.as_ref().to_path_buf())
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

/// The folder or file `path` really names, as the policy resolves one to compare it
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
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
