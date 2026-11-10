//! Each harness's own files in a worktree, which no worker writes
//!
//! A harness loads its own files from the worktree: Claude Code its `.claude`
//! folder and `.mcp.json`, Codex its `.codex` folder, goose its `.goose`
//! folder. Settings, hooks and MCP servers there run code, so the sandbox
//! refuses writes to every harness's files, each turn's end checks them
//! against `main`'s, and a pull request that changes them waits on the
//! maintainer. Names match in any case, since macOS reads `.CLAUDE` as `.claude`.
//!
//! Each adapter supplies its own row as an [`OwnFiles`] (`harness::claude::OWN_FILES`
//! and the rest, gathered in [`harness::OWN_FILES`]). Codex's and pi's rows stay
//! here until their adapters exist, so a worker on any harness still cannot
//! plant them. Nothing calls the fence yet: the worker's policy and the turn's
//! end do.
//!
//! Git is asked through the [`worktree`] module and its [`ProcessTree`]
//! (ADR 0001).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use crate::harness;
use crate::tree::ProcessTree;
use crate::worktree::{self, Remote, WorktreeError, git};

#[cfg(test)]
mod clone_tests;
mod commit;
mod nested;

/// The files in a worktree that one harness loads and may run code from, and
/// the files in the home folder that hold its login
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OwnFiles {
    /// The harness, as a refusal names it
    pub harness: &'static str,
    /// Folders it loads, at the root or at any depth below
    pub folders: &'static [&'static str],
    /// Folders it loads that sit in a folder of another name, as `.config/goose`,
    /// at the root or at any depth below. Each is a parent's name and its own.
    pub nested: &'static [[&'static str; 2]],
    /// Files it loads at the root only
    pub root_files: &'static [&'static str],
    /// Its login and secrets, under `~/`, which no other harness's call reads
    pub credentials: &'static [&'static str],
    /// The program that starts it, which no worker runs from its commands
    pub program: &'static str,
}

/// Codex's row, until its adapter supplies it
///
/// Codex lists skills in a repo's `.agents` whether or not it trusts the repo.
const CODEX: OwnFiles = OwnFiles {
    harness: "Codex",
    folders: &[".codex", ".agents"],
    nested: &[],
    root_files: &[],
    credentials: &["~/.codex/**"],
    program: "codex",
};

/// pi, as the fence names it
pub const PI: &str = "pi";

/// pi's row, until its adapter supplies it
///
/// Kelpie runs pi with a home of its own, so the person's is never read.
const PI_ROW: OwnFiles = OwnFiles {
    harness: PI,
    folders: &[".pi"],
    nested: &[],
    root_files: &[],
    credentials: &["~/.pi/**"],
    program: "pi",
};

/// The rows of the harnesses with no adapter yet
const UNBUILT: [OwnFiles; 2] = [CODEX, PI_ROW];

/// Every harness's own files, each fenced whichever harness a call runs on
///
/// A worker on one harness could otherwise plant files that the next call,
/// on another, runs. The harnesses that have an adapter supply their own row,
/// and the rest are listed here.
fn harnesses() -> impl Iterator<Item = &'static OwnFiles> {
    harness::OWN_FILES.iter().chain(UNBUILT.iter())
}

/// Every harness's credentials, Claude Code's included, which a call on a harness
/// that keeps its login elsewhere may not read
pub fn credentials() -> Vec<String> {
    others_credentials("")
}

/// Every other harness's credentials, which a call on `harness` may not read
pub fn others_credentials(harness: &str) -> Vec<String> {
    harnesses()
        .filter(|own| own.harness != harness)
        .flat_map(|own| own.credentials.iter().map(|&path| path.to_owned()))
        .collect()
}

// Every name a harness loads from the worktree's root.
fn root_names() -> impl Iterator<Item = &'static str> {
    harnesses()
        .flat_map(|own| own.folders.iter().chain(own.root_files))
        .copied()
}

// Every folder a harness loads from a folder of another name in the worktree's root.
fn nested_names() -> impl Iterator<Item = [&'static str; 2]> {
    harnesses().flat_map(|own| own.nested).copied()
}

/// The sandbox's `denyWrite` paths that fence these files in `worktree`
///
/// srt skips every glob in `denyWrite` on Linux, and on Windows expands one to
/// every file that exists. On both the harness folders already below the
/// worktree are listed as paths too, and on Windows the globs are left out.
/// One made during the call is caught by [`differ`] after it.
pub fn deny_write(worktree: &Path) -> Vec<PathBuf> {
    deny_write_on(worktree, std::env::consts::OS)
}

// The same, as on the operating system `os` names, which a test runs on any.
fn deny_write_on(worktree: &Path, os: &str) -> Vec<PathBuf> {
    let windows = os == "windows";
    let mut paths = roots(worktree, !windows);
    if matches!(os, "linux" | "windows") {
        for path in nested::existing(worktree) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    paths
}

/// The sandbox's `denyWrite` paths for the package install that runs before a worker's first turn
///
/// Only the harnesses' files at the worktree's root. The install runs the package
/// manager and no agent, and a package may ship a `.claude` folder of its own
/// under `node_modules`, which the `**` globs of [`deny_write`] would refuse.
pub fn install_deny_write(worktree: &Path) -> Vec<PathBuf> {
    roots(worktree, false)
}

// The harnesses' files at the worktree's root, and with `globs` the same folders at any depth.
fn roots(worktree: &Path, globs: bool) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for own in harnesses() {
        for folder in own.folders {
            paths.push(worktree.join(folder));
            if globs {
                paths.push(worktree.join("**").join(folder));
            }
        }
        for [parent, folder] in own.nested {
            let inside = Path::new(parent).join(folder);
            paths.push(worktree.join(&inside));
            if globs {
                paths.push(worktree.join("**").join(inside));
            }
        }
        paths.extend(own.root_files.iter().map(|file| worktree.join(file)));
    }
    paths
}

/// Whether `path`, relative to a worktree, is one of a harness's own files
pub fn fenced(path: &Path) -> bool {
    owner(path).is_some()
}

/// The harness whose own file `path`, relative to a worktree, is
pub fn owner(path: &Path) -> Option<&'static str> {
    let parts: Vec<&OsStr> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part),
            _ => None,
        })
        .collect();
    let any = |names: &[&str], part: &OsStr| names.iter().any(|name| folds_to(part, name));
    harnesses()
        .find(|own| {
            matches!(parts.as_slice(), [only] if any(own.root_files, only))
                || parts.iter().any(|part| any(own.folders, part))
                || (parts.windows(2)).any(|pair| {
                    own.nested.iter().any(|[parent, folder]| {
                        folds_to(pair[0], parent) && folds_to(pair[1], folder)
                    })
                })
        })
        .map(|own| own.harness)
}

// Whether a case-insensitive file system could read `part` as `name`. Case
// is folded the Unicode way (`ſ` reads as `s`), and whatever is still not
// ASCII is dropped, which errs toward refusing a name no one would read.
fn folds_to(part: &OsStr, name: &str) -> bool {
    let folded: String = part
        .to_string_lossy()
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .filter(char::is_ascii)
        .collect();
    folded == name
}

/// Fetches `branch` from `remote`, and names the harnesses' own files its head
/// changes
///
/// A file counts when the branch changes it from `main`, and from `accepted`,
/// a head whose change to them the maintainer accepted, if there is one.
/// Returns the head and those files.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn changed(
    tree: &ProcessTree,
    repo: &Path,
    branch: &str,
    remote: &Remote,
    accepted: Option<&str>,
    base: &str,
) -> Result<(String, Vec<String>), WorktreeError> {
    let head = worktree::head_with_main(tree, repo, branch, remote, base)?;
    let between = |range: &str| -> Result<Vec<String>, WorktreeError> {
        let names = git(
            tree,
            repo,
            ["diff", "--name-only", "--no-renames", "-z", range],
        )?;
        Ok(names
            .split('\0')
            .filter(|name| fenced(Path::new(name)))
            .map(str::to_owned)
            .collect())
    };
    let from_main = between(&format!("origin/{base}...{head}"))?;
    if from_main.is_empty() {
        return Ok((head, from_main));
    }
    if let Some(accepted) = accepted
        && between(&format!("{accepted}..{head}"))?.is_empty()
    {
        return Ok((head, Vec::new()));
    }
    Ok((head, from_main))
}

/// Names the harnesses' own files in `worktree` that differ from `main`'s
///
/// The worktree passes when all of them match one commit: `origin/main`,
/// the commit its branch left `main` at, or `accepted`. A file missing on
/// either side differs. Returns the files that differ from `origin/main`.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command or folder that failed.
pub fn differ(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    accepted: Option<&str>,
    base: &str,
) -> Result<Vec<String>, WorktreeError> {
    let disk = on_disk(worktree)?;
    let hashes = hashed(tree, repo, &disk)?;
    let (head, main) = commit::head_and_main(tree, repo, worktree, base)?;
    let cut_at = commit::cut_at(tree, repo, worktree, &head, &main)?;
    let mut first = None;
    for commit in [Some(main.as_str()), Some(cut_at.as_str()), accepted]
        .into_iter()
        .flatten()
    {
        let trusted = commit::files_in(tree, repo, commit)?;
        let differing = compare(tree, repo, &disk, &hashes, &trusted)?;
        if differing.is_empty() {
            return Ok(differing);
        }
        first.get_or_insert(differing);
    }
    Ok(first.unwrap_or_default())
}

/// Whether the harnesses' own files in `worktree` are exactly `origin/main`'s
///
/// Unlike [`differ`], a commit the branch left `main` at or a head the
/// maintainer accepted does not count: a call in such a worktree loads
/// `main`'s own files and nothing else. A folder that is no worktree matches
/// when `main` holds no such files. Any error counts as a mismatch.
pub fn matches_main(tree: &ProcessTree, repo: &Path, worktree: &Path, base: &str) -> bool {
    let Ok(disk) = on_disk(worktree) else {
        return false;
    };
    let Ok(hashes) = hashed(tree, repo, &disk) else {
        return false;
    };
    commit::main_of(tree, repo, base)
        .and_then(|main| commit::files_in(tree, repo, &main))
        .and_then(|trusted| compare(tree, repo, &disk, &hashes, &trusted))
        .is_ok_and(|differing| differing.is_empty())
}

// The blob id git gives each file on disk, byte for byte as it is.
fn hashed(
    tree: &ProcessTree,
    repo: &Path,
    disk: &BTreeMap<String, Disk>,
) -> Result<BTreeMap<PathBuf, String>, WorktreeError> {
    let files: Vec<&PathBuf> = disk
        .values()
        .filter_map(|entry| match entry {
            Disk::File { path, .. } => Some(path),
            _ => None,
        })
        .collect();
    let mut hashes = BTreeMap::new();
    if !files.is_empty() {
        let mut args: Vec<&OsStr> = ["hash-object", "--no-filters", "--"].map(OsStr::new).into();
        args.extend(files.iter().map(|p| p.as_os_str()));
        let ids = git(tree, repo, args)?;
        hashes.extend(
            files
                .into_iter()
                .cloned()
                .zip(ids.lines().map(str::to_owned)),
        );
    }
    Ok(hashes)
}

// A fenced entry in the worktree, never followed if it is a link.
enum Disk {
    // `executable` is `None` where the file system keeps no mode: see `executable_bit`.
    File {
        path: PathBuf,
        executable: Option<bool>,
    },
    Link(String),
    Other,
}

// Every file a harness would load from the worktree's root, by its path
// from the root. The names are opened as a harness opens them, so the
// file system folds case and Unicode its own way. Folders count only for
// what they hold, as in git.
fn on_disk(worktree: &Path) -> Result<BTreeMap<String, Disk>, WorktreeError> {
    let mut found = BTreeMap::new();
    let mut pending: Vec<PathBuf> = Vec::new();
    let nested = nested_names().map(|[parent, folder]| Path::new(parent).join(folder));
    for name in root_names().map(PathBuf::from).chain(nested) {
        let path = worktree.join(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => pending.push(path),
            Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {}
            Err(e) => return Err(folder(&path, &e)),
        }
    }
    while let Some(path) = pending.pop() {
        let meta = fs::symlink_metadata(&path).map_err(|e| folder(&path, &e))?;
        let relative = git_path(
            path.strip_prefix(worktree)
                .expect("found under the worktree"),
        );
        let kind = meta.file_type();
        if kind.is_dir() {
            pending.extend(read_dir(&path)?.map(|entry| entry.path()));
            continue;
        }
        let disk = if kind.is_symlink() {
            let target = fs::read_link(&path).map_err(|e| folder(&path, &e))?;
            Disk::Link(target.to_string_lossy().into_owned())
        } else if kind.is_file() {
            let executable = executable_bit(&meta);
            Disk::File { path, executable }
        } else {
            Disk::Other
        };
        found.insert(relative, disk);
    }
    Ok(found)
}

// `relative` spelled as git spells a path, with `/` between its parts on
// every platform, so a file on disk finds the same file in a commit.
fn git_path(relative: &Path) -> String {
    let parts: Vec<_> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect();
    parts.join("/")
}

// Whether a file on disk is executable, as git records it: git reads the
// owner's execute bit alone, so a group or other bit leaves the mode 100644.
#[cfg(unix)]
fn executable_bit(meta: &fs::Metadata) -> Option<bool> {
    Some(meta.permissions().mode() & 0o100 != 0)
}

// Windows keeps no executable bit on a file, and git there sets
// `core.fileMode` off, so the disk cannot say. A worker cannot set the bit
// through the file system either, so the commit's regular mode stands.
#[cfg(windows)]
fn executable_bit(_: &fs::Metadata) -> Option<bool> {
    None
}

fn compare(
    tree: &ProcessTree,
    repo: &Path,
    disk: &BTreeMap<String, Disk>,
    hashes: &BTreeMap<PathBuf, String>,
    trusted: &BTreeMap<String, (String, String)>,
) -> Result<Vec<String>, WorktreeError> {
    let mut differing: Vec<String> = trusted
        .keys()
        .filter(|path| !disk.contains_key(*path))
        .cloned()
        .collect();
    for (path, entry) in disk {
        let same = match (entry, trusted.get(path)) {
            (Disk::File { path, executable }, Some((mode, blob))) => {
                let same_mode = match executable {
                    Some(true) => mode == "100755",
                    Some(false) => mode == "100644",
                    None => mode == "100644" || mode == "100755",
                };
                same_mode && hashes.get(path) == Some(blob)
            }
            (Disk::Link(target), Some((mode, blob))) if mode == "120000" => {
                git(tree, repo, ["cat-file", "blob", blob])? == *target
            }
            _ => false,
        };
        if !same {
            differing.push(path.clone());
        }
    }
    differing.sort();
    Ok(differing)
}

fn read_dir(path: &Path) -> Result<impl Iterator<Item = fs::DirEntry>, WorktreeError> {
    let entries = fs::read_dir(path).map_err(|e| folder(path, &e))?;
    let entries: Result<Vec<_>, _> = entries.collect();
    Ok(entries.map_err(|e| folder(path, &e))?.into_iter())
}

fn folder(path: &Path, e: &std::io::Error) -> WorktreeError {
    WorktreeError::Unreadable {
        path: path.to_owned(),
        kind: e.kind(),
    }
}

#[cfg(test)]
mod tests;
