//! A ticket's worktree, read the way git reads it
//!
//! Each read is tried in this process first, which spares starting git, and
//! asked of git where that read cannot answer. An in-process read opens the
//! repo with its own config only, so it answers as git does only where a
//! system, global or environment config could not change the answer.
//!
//! Every program here goes through [`ProcessTree::output`], so every git kelpie
//! starts is in the run's process tree (ADR 0001).

use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::process::Command;

use crate::tree::ProcessTree;

mod in_process;
mod loose;

#[cfg(test)]
pub(crate) mod tests;

// `is_ancestor` and `local_config` are public here, where SK keeps them to its
// crate: their callers are outside this one.
pub use in_process::{TreeFile, blobs, is_ancestor, local_config, tree_files};
pub use loose::{commit_of, loose_commit, loose_commit_in, loose_head};

/// Why a worktree read could not be answered
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WorktreeError {
    /// git could not be started, with the OS's reason
    Spawn(String),
    /// A git command exited unsuccessfully
    Git {
        /// Its arguments
        args: String,
        /// What it printed on stderr
        stderr: String,
    },
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "cannot run git: {e}"),
            Self::Git { args, stderr } => write!(f, "git {args} failed: {}", stderr.trim()),
        }
    }
}

impl core::error::Error for WorktreeError {}

/// The variables that choose which repo a git call reads, which git dirs and
/// objects it uses, what namespace its refs sit in and which config it reads
const GIT_ENV: [&str; 11] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_COUNT",
];

/// `git -C cwd`, waiting its turn for a ref another git holds
///
/// The open tickets' worktrees share one repo's refs, and their calls run at
/// once, so a ref or `packed-refs` lock one git holds makes another wait for it
/// rather than fail at once.
///
/// `cwd` is the only thing that says which repo git reads: kelpie's own
/// environment is stripped of [`GIT_ENV`], which a shell or a hook could leave
/// pointing at another repo, and which the in-process reads already ignore.
fn in_repo(cwd: &Path) -> Command {
    let mut git = Command::new("git");
    for name in GIT_ENV {
        git.env_remove(name);
    }
    git.args(["-c", "core.filesRefLockTimeout=2000"])
        .args(["-c", "core.packedRefsTimeout=5000"])
        .arg("-C")
        .arg(cwd);
    git
}

/// Runs `git args` in `cwd`, with the git dir found there, and returns its trimmed stdout
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn git<I, S>(tree: &ProcessTree, cwd: &Path, args: I) -> Result<String, WorktreeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    output(tree, in_repo(cwd), args)
}

// Runs `git` with `args` added, through the tree, and returns its trimmed stdout.
fn output<I, S>(tree: &ProcessTree, mut git: Command, args: I) -> Result<String, WorktreeError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<_> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
    let output = tree
        .output(git.args(&args))
        .map_err(|e| WorktreeError::Spawn(e.to_string()))?;
    if !output.status.success() {
        let args: Vec<_> = args.iter().map(|a| a.to_string_lossy()).collect();
        return Err(WorktreeError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
