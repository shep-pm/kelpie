//! Starting a command from a process the `kelpie` profile confines
//!
//! AppArmor attaches a profile when a program is exec'd, so a kelpie that was
//! already running when Install loaded the profile stays `unconfined`, and only
//! the profile's own `Px` rule moves bwrap into `kelpie_bwrap`. [`wrap`] runs
//! the command through a fresh exec of kelpie's binary, which the loaded
//! profile confines, so the next session starts sandboxed without a relaunch.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;
use std::process::Command;

#[cfg(test)]
mod tests;

/// The argument that makes kelpie's binary exec the rest of its arguments
const FLAG: &str = "--sandbox-hop";

const ATTR: &str = "/proc/self/attr/current";
const PROFILES: &str = "/sys/kernel/security/apparmor/policy/profiles";

/// `command` run through a fresh exec of kelpie's binary, when that moves it
/// into the `kelpie` profile, and `None` when it should run as it is
pub(super) fn through_profile(command: &Command) -> Option<Command> {
    let attr = fs::read_to_string(ATTR).ok()?;
    let profiles = fs::read_dir(PROFILES).ok()?;
    let names = profiles
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned());
    if !needed(&attr, names) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    Some(wrap(&exe, command))
}

/// Whether a `kelpie.<n>` profile is loaded on this machine
///
/// Install loads it, and so does each command the sandbox item asks the
/// maintainer to run in a terminal. It says nothing about this process: a kelpie
/// already running stays `unconfined` whatever is loaded. False where AppArmor
/// has no profiles folder to read, as on macOS and Windows.
#[must_use]
pub fn profile_loaded() -> bool {
    loaded_in(Path::new(PROFILES))
}

/// Whether `folder` holds a `kelpie.<n>` entry
fn loaded_in(folder: &Path) -> bool {
    let Ok(entries) = fs::read_dir(folder) else {
        return false;
    };
    entries
        .filter_map(Result::ok)
        .any(|entry| named_kelpie(entry.file_name()))
}

/// Whether a profile entry `name` is kelpie's, which AppArmor writes as `kelpie.<n>`
fn named_kelpie(name: impl AsRef<OsStr>) -> bool {
    name.as_ref()
        .to_string_lossy()
        .strip_prefix("kelpie.")
        .is_some_and(|number| {
            !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// Whether kelpie runs unconfined while a `kelpie.<n>` profile is loaded
///
/// It does not check that the binary's path is the one the profile names: an
/// AppImage or a build run beside the package hops and stays unconfined.
fn needed(attr: &str, profiles: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    attr.trim() == "unconfined" && profiles.into_iter().any(|name| named_kelpie(name.as_ref()))
}

/// `command`, run by `exe --sandbox-hop`, with its directory and environment
fn wrap(exe: &Path, command: &Command) -> Command {
    let mut hopped = Command::new(exe);
    hopped
        .arg(FLAG)
        .arg("--")
        .arg(command.get_program())
        .args(command.get_args());
    if let Some(dir) = command.get_current_dir() {
        hopped.current_dir(dir);
    }
    for (name, value) in command.get_envs() {
        match value {
            Some(value) => hopped.env(name, value),
            None => hopped.env_remove(name),
        };
    }
    hopped
}

#[cfg(any(unix, test))]
/// The program and arguments after `--sandbox-hop --` in `args`, which start
/// with the binary's own name
fn target(args: &[OsString]) -> Option<(&OsString, &[OsString])> {
    match args {
        [_, flag, dashes, program, rest @ ..] if flag == FLAG && dashes == "--" => {
            Some((program, rest))
        }
        _ => None,
    }
}

/// Execs the command after `--sandbox-hop --` in place of this process, or
/// comes back when this run has none
///
/// The exec keeps the pid, process group and environment the process tree set
/// (ADR 0001). A failed exec ends the process with 127.
#[cfg(unix)]
pub(super) fn run_from_args() {
    use std::os::unix::process::CommandExt;

    let args: Vec<OsString> = std::env::args_os().collect();
    let Some((program, rest)) = target(&args) else {
        return;
    };
    let error = Command::new(program).args(rest).exec();
    eprintln!("kelpie: the sandbox hop could not run {program:?}: {error}");
    std::process::exit(127);
}

/// Windows has no profile to hop into
#[cfg(not(unix))]
pub(super) fn run_from_args() {}
