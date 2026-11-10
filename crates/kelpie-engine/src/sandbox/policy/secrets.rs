//! What is closed, whatever else is read, and which projects are refused

use std::path::{Path, PathBuf};

use super::{Project, home, real};

/// The files in the home folder that hold each harness's login, and whose
/// session may not read any of them: a harness gets its own login through its
/// environment instead
///
/// Lifted from SK's `OwnFiles::credentials`, with each harness named as SK
/// named it.
const LOGINS: [(&str, &[&str]); 4] = [
    (
        "Claude Code",
        // Linux keeps the login in the file; `.claude.json` holds MCP servers' secrets
        &["~/.claude/.credentials.json", "~/.claude.json"],
    ),
    ("Codex", &["~/.codex/**"]),
    ("pi", &["~/.pi/**"]),
    (
        "goose",
        // Its config, secrets and session database sit in four places, by platform
        &[
            "~/.config/goose/**",
            "~/.local/share/goose/**",
            "~/.local/state/goose/**",
            "~/Library/Application Support/Block/goose/**",
        ],
    ),
];

/// The secrets any tool keeps in the home folder
///
/// Lifted from SK's `profile::CREDENTIALS`, less the entries for SK's own old
/// home. `~/.config/gh` is added: the PM never holds a GitHub token (ADR 0005).
/// So is macOS's keychain folder, where `gh` and git's helper keep a login, and
/// where kelpie's own logins are stored (ADR 0008).
const SECRETS: [&str; 11] = [
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

/// Every harness's login and every secret, as paths and globs under `~/`
///
/// The same list for a session on any harness: the session's own harness gets
/// its login through its environment, so none of these is one it needs.
#[must_use]
pub fn denied() -> Vec<&'static str> {
    LOGINS
        .iter()
        .flat_map(|(_, paths)| paths.iter().copied())
        .chain(SECRETS)
        .collect()
}

/// Each entry of [`denied`] as the folder or file it really names
///
/// `~` is the person's home folder, a trailing `/**` is dropped as crabpot drops
/// it, and each path is resolved the way a folder is compared here. An allow at
/// or inside one of these opens it, so no allow lands on one and a project
/// inside one is refused.
#[must_use]
pub fn denied_folders() -> Vec<PathBuf> {
    let Some(home) = home() else {
        return Vec::new();
    };
    denied()
        .into_iter()
        .filter_map(|entry| denied_path(entry, &home))
        .map(|path| real(&path))
        .collect()
}

/// The entry `entry` of [`denied`] as a path under `home`
///
/// None when it names nothing: an empty `home` makes `~/.ssh` the relative
/// `.ssh`, which is a folder of wherever the session was started.
fn denied_path(entry: &str, home: &Path) -> Option<PathBuf> {
    let entry = entry.strip_suffix("/**").unwrap_or(entry);
    let path = match entry.strip_prefix('~') {
        Some(tail) => home.join(tail.strip_prefix('/').unwrap_or(tail)),
        None => PathBuf::from(entry),
    };
    path.is_absolute().then_some(path)
}

/// The project folder when it is the home folder or a folder above it
///
/// Such a folder holds the logins and secrets the policy denies under the home
/// folder, so a sandbox over it would read them along with the project.
/// [`Sandbox::start`](crate::sandbox::Sandbox::start) refuses one before the
/// session starts, and the guard in `reads` keeps any other allow off the home
/// folder. The refusal names the folder as the person chose it, and both compare
/// the folder it really names, as crabpot matches it.
#[must_use]
pub fn holds_home(project: &Project<'_>) -> Option<PathBuf> {
    home()
        .filter(|home| real(home).starts_with(real(project.folder)))
        .map(|_| project.folder.to_path_buf())
}

/// The project folder and the denied folder it is at or inside, when it is
///
/// A project at `~/.codex/skills/<name>` has Codex's own folder as the folder
/// beside it, so its sandbox would be asked to read the login whole.
/// [`Sandbox::start`](crate::sandbox::Sandbox::start) refuses it before the
/// session starts, and the guard in `reads` keeps any other allow off it. The
/// refusal names the folder as the person chose it, and both compare the folder
/// it really names, as crabpot matches it.
#[must_use]
pub fn in_denied(project: &Project<'_>) -> Option<(PathBuf, PathBuf)> {
    let reached = real(project.folder);
    denied_folders()
        .into_iter()
        .find(|held| reached.starts_with(held))
        .map(|held| (project.folder.to_path_buf(), held))
}
