//! A ticket's worktree: a clone with a git dir of its own, cut from `origin/main`
//!
//! Each ticket gets a [`clone`] beside its folder, on a branch cut from the
//! latest `origin/main`, borrowing the checkout's objects. [`prepare`] is
//! idempotent, so a resumed worker finds the worktree it left. Kelpie's git
//! reads of a worktree go through [`trusted`], which names the git dirs
//! instead of finding them, since the worker can write its worktree's `.git`.
//! [`remove`] takes a ticket's worktree, its clone's git dir, its branch and its
//! build folder away, ending whatever still runs from the build folder first.
//! [`rebase`] catches a branch up with `origin/main` and pushes it, or leaves
//! the conflict in the worktree for the worker, and [`abort_merge`] gives that
//! merge up. [`merge_conflicts`] and [`runs_on_pull_requests`] ask whether a
//! head would conflict and whether the base runs CI on a pull request.
//!
//! `main` and `origin/main` in these docs are the project's base branch, which
//! each function takes as `base`.
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
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use crate::tree::ProcessTree;

mod cache;
mod catch_up;
#[cfg(test)]
mod catch_up_tests;
pub mod clone;
mod common;
mod conflicts;
mod in_process;
#[cfg(test)]
mod line_endings_tests;
mod loose;
mod remote;
mod stale_lock;
mod stray;
mod workflows;
mod writable;

#[cfg(test)]
pub(crate) mod tests;

// `is_ancestor` and `local_config` are public here, where SK keeps them to its
// crate: their callers are outside this one.
pub use cache::{Pruned, TurnMark, prune};
pub use catch_up::{Rebase, rebase};
pub use conflicts::merge_conflicts;
pub use in_process::{TreeFile, blobs, is_ancestor, local_config, tree_files};
pub use loose::{commit_of, loose_commit, loose_commit_in, loose_head};
pub use remote::Remote;
pub use workflows::runs_on_pull_requests;
pub use writable::remove_tree;

/// Where a new worktree's branch starts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Cut from `origin/main`
    Main,
    /// The branch as its [`Remote`] holds it, whoever pushed to it
    Pushed,
}

/// A ticket's worktree and the project repo it belongs to, whose git dirs
/// kelpie checks it against before running git on it
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Linked<'a> {
    /// The project's repo
    pub repo: &'a Path,
    /// The ticket's worktree, which its worker writes
    pub worktree: &'a Path,
}

/// A prepared worktree, and the git dirs a commit from it writes to
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// The checkout's git dir, whose objects the clone borrows
    pub git_common_dir: PathBuf,
    /// The clone's git dir: its refs, index, HEAD and config
    pub git_dir: PathBuf,
    /// What the fetch of `origin/main` failed with, when it failed
    ///
    /// The worktree keeps the `origin/main` kelpie last fetched, and the
    /// caller notes this, since the worker merges that ref and runs no fetch
    /// of its own.
    pub main_fetch_failed: Option<String>,
}

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
    /// The folder exists but is not a worktree on the ticket's branch
    Foreign(PathBuf),
    /// The branch exists with no worktree, so it is not kelpie's to reuse
    BranchTaken(String),
    /// A folder or file could not be created
    Folder {
        /// The folder
        path: PathBuf,
        /// What creating it failed with
        kind: io::ErrorKind,
    },
    /// A file or folder in the worktree could not be read
    Unreadable {
        /// The file or folder
        path: PathBuf,
        /// What reading it failed with
        kind: io::ErrorKind,
    },
    /// A folder could not be removed
    Remove {
        /// The folder
        path: PathBuf,
        /// What removing it failed with
        kind: io::ErrorKind,
    },
}

impl fmt::Display for WorktreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "cannot run git: {e}"),
            Self::Git { args, stderr } => write!(f, "git {args} failed: {}", stderr.trim()),
            Self::Foreign(path) => write!(
                f,
                "{} exists and is not this ticket's worktree",
                path.display()
            ),
            Self::BranchTaken(branch) => {
                write!(f, "branch {branch} already exists without its worktree")
            }
            Self::Folder { path, kind } => write!(f, "cannot create {}: {kind}", path.display()),
            Self::Unreadable { path, kind } => write!(f, "cannot read {}: {kind}", path.display()),
            Self::Remove { path, kind } => write!(f, "cannot remove {}: {kind}", path.display()),
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

/// Makes sure `worktree` is a clone of `repo` on `branch`, and `build` exists
///
/// A new worktree is a [`clone`] starting where `start` says, read from
/// `remote` for [`Start::Pushed`]. A linked worktree on the branch is turned
/// into one and keeps its files. Every call fetches `origin/main` into the
/// checkout and sets the clone's to it. A failed fetch keeps the last one and
/// [`Worktree::main_fetch_failed`] says why, except for [`Start::Main`] on a
/// new worktree.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command or folder that failed.
// SK's signature, which every caller of it in the runner reads the same way
#[expect(
    clippy::too_many_arguments,
    reason = "each is a different thing a worktree is made from"
)]
pub fn prepare(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    branch: &str,
    start: Start,
    base: &str,
    remote: &Remote,
    build: &Path,
) -> Result<Worktree, WorktreeError> {
    let foreign = || WorktreeError::Foreign(worktree.to_owned());
    stale_lock::clear(&clone::git_dir_of(worktree), SystemTime::now());
    let full_ref = format!("refs/heads/{branch}");
    let mut main_fetch_failed = None;
    if worktree.exists() {
        match listed_branch(tree, repo, worktree)? {
            Some(listed) if listed == full_ref => {
                clone::convert(tree, repo, worktree, branch)?;
                mirror_tracking(tree, repo, worktree, &remote.tracking(branch))?;
            }
            Some(_) => return Err(foreign()),
            // A clone is not in the checkout's list: its own git says
            // which branch it is on.
            None => {
                let on = trusted(tree, repo, worktree)?(&["symbolic-ref", "--quiet", "HEAD"]).ok();
                if on.as_deref() != Some(full_ref.as_str()) {
                    return Err(foreign());
                }
                clone::complete(tree, repo, worktree)?;
            }
        }
        // The worker merges `origin/main` and runs no fetch of its own, so
        // kelpie brings main to it before each of its turns.
        main_fetch_failed = fetch_main(tree, repo, worktree, base);
    } else {
        let local = commit_of(tree, repo, &full_ref).ok();
        if local.is_some() && start == Start::Main {
            return Err(WorktreeError::BranchTaken(branch.to_owned()));
        }
        let (start_ref, tracking) = match start {
            Start::Main => {
                git(tree, repo, ["fetch", "--quiet", "--", "origin", base])?;
                (format!("refs/remotes/origin/{base}"), None)
            }
            Start::Pushed => {
                let tracking = remote.fetch(tree, repo, branch)?;
                (tracking.clone(), Some(tracking))
            }
        };
        if let Some(local) = local {
            // A rework's branch left behind by an earlier attempt is reused
            // when it is at the same commit as the remote's. One ahead holds
            // work that is not kelpie's to throw away, and one behind is
            // not the pull request's branch as the remote holds it.
            if commit_of(tree, repo, &start_ref)? != local {
                return Err(WorktreeError::BranchTaken(branch.to_owned()));
            }
        }
        clone::keep_work_at(tree, repo, worktree, branch, &start_ref)?;
        clone::make(tree, repo, worktree, branch, &start_ref)?;
        match tracking {
            // A branch that is already pushed needs main only for the merge
            // its worker runs, so a failed fetch of it is reported rather
            // than fatal.
            Some(tracking) => {
                mirror_tracking(tree, repo, worktree, &tracking)?;
                main_fetch_failed = fetch_main(tree, repo, worktree, base);
            }
            None => mirror_main(tree, repo, worktree, base)?,
        }
    }
    let git_common_dir = common_dir(tree, repo)?;
    let (git_dir, _) = clone::dirs(&git_common_dir, worktree)?.ok_or_else(foreign)?;
    create(build)?;
    Ok(Worktree {
        git_common_dir,
        git_dir,
        main_fetch_failed,
    })
}

// Fetch `origin/main` into the checkout and set the clone's to it, and say
// what either failed with. A fetch that fails leaves the ref where kelpie last
// put it.
fn fetch_main(tree: &ProcessTree, repo: &Path, worktree: &Path, base: &str) -> Option<String> {
    let fetched = git(tree, repo, ["fetch", "--quiet", "--", "origin", base]).err();
    let mirrored = mirror_main(tree, repo, worktree, base).err();
    fetched.or(mirrored).map(|e| e.to_string())
}

// Set the clone's `origin/main` to the checkout's, when the checkout has one.
fn mirror_main(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    base: &str,
) -> Result<(), WorktreeError> {
    mirror_tracking(tree, repo, worktree, &format!("refs/remotes/origin/{base}"))
}

// Set the clone's `tracking` ref to the checkout's, when the checkout has one.
fn mirror_tracking(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    tracking: &str,
) -> Result<(), WorktreeError> {
    if loose_commit_in(repo, tracking).is_some() || has_ref(tree, repo, tracking)? {
        clone::mirror(tree, repo, worktree, &[tracking])?;
    }
    Ok(())
}

/// Whether `full_ref` is in `repo`, telling a ref that is not there from a
/// git that cannot answer
fn has_ref(tree: &ProcessTree, repo: &Path, full_ref: &str) -> Result<bool, WorktreeError> {
    // `show-ref --verify` answers a missing ref with exit 1.
    exit_zero_or_one(tree, repo, ["show-ref", "--verify", "--quiet", full_ref])
}

// Runs `git args` in `repo` for a command that answers yes with exit 0 and no
// with exit 1, and fails with any other code.
fn exit_zero_or_one<const N: usize>(
    tree: &ProcessTree,
    repo: &Path,
    args: [&str; N],
) -> Result<bool, WorktreeError> {
    let output = tree
        .output(in_repo(repo).args(args))
        .map_err(|e| WorktreeError::Spawn(e.to_string()))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(WorktreeError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        }),
    }
}

/// Removes a ticket's worktree, its branch and its build folder
///
/// A clone's worktree and git dir are removed whole, a linked one through git.
/// Whatever still runs from the build folder is ended before it is deleted.
/// `remote` also deletes the branch on that [`Remote`]. Whatever is already
/// gone, a branch the forge deleted first included, is skipped, so a removal
/// cut short can run again.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command or folder that failed.
pub fn remove(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    branch: &str,
    build: &Path,
    remote: Option<&Remote>,
) -> Result<(), WorktreeError> {
    let git_dir = clone::git_dir_of(worktree);
    if git_dir.exists() {
        // The worktree goes first, so a removal cut short between the two
        // still finds the git dir and takes it.
        remove_tree_of(worktree)?;
        remove_tree_of(&git_dir)?;
    } else if worktree.exists() {
        git(
            tree,
            repo,
            [
                "worktree".as_ref(),
                "remove".as_ref(),
                "--force".as_ref(),
                worktree.as_os_str(),
            ],
        )?;
    }
    git(tree, repo, ["worktree", "prune"])?;
    let full_ref = format!("refs/heads/{branch}");
    if commit_of(tree, repo, &full_ref).is_ok() {
        git(tree, repo, ["branch", "--quiet", "-D", branch])?;
    }
    if let Some(remote) = remote {
        remote.delete(tree, repo, branch)?;
    }
    stray::end_running_from(tree, build);
    remove_tree_of(build)
}

// Removes the tree at `path` when it is there.
fn remove_tree_of(path: &Path) -> Result<(), WorktreeError> {
    match writable::remove_tree(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(WorktreeError::Remove {
            path: path.to_owned(),
            kind: e.kind(),
        }),
        _ => Ok(()),
    }
}

/// The worktree other than `own` that has `branch` checked out, if one has:
/// the project's checkout and a maintainer's own worktree included
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn holder_of(
    tree: &ProcessTree,
    repo: &Path,
    branch: &str,
    own: &Path,
) -> Result<Option<PathBuf>, WorktreeError> {
    let list = git(tree, repo, ["worktree", "list", "--porcelain"])?;
    let (own, want) = (canonical(own), format!("refs/heads/{branch}"));
    let mut at = None;
    for line in list.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            at = Some(PathBuf::from(path));
        } else if line.strip_prefix("branch ") == Some(want.as_str())
            && let Some(path) = at.take().filter(|path| canonical(path) != own)
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Whether a ticket's branch or worktree holds work `origin/main` lacks: a
/// commit on `branch` or at the worktree's head, or a file not committed
///
/// Fetches `origin/main` first. A branch or worktree that is not there holds
/// none.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn holds_work(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    branch: &str,
    base: &str,
) -> Result<bool, WorktreeError> {
    git(tree, repo, ["fetch", "--quiet", "origin", base])?;
    // Read from the checkout: a clone's own `origin/main` is the worker's.
    let main = commit_of(tree, repo, &format!("refs/remotes/origin/{base}"))?;
    let mut tips = Vec::new();
    let full_ref = format!("refs/heads/{branch}");
    // A git that cannot answer is an error, not a branch that is not there.
    match loose_commit_in(repo, &full_ref) {
        Some(tip) => tips.push(tip),
        None if has_ref(tree, repo, &full_ref)? => tips.push(commit_of(tree, repo, &full_ref)?),
        None => {}
    }
    if worktree.exists() {
        if !uncommitted(tree, repo, worktree)?.is_empty() {
            return Ok(true);
        }
        let in_worktree = trusted(tree, repo, worktree)?;
        tips.push(in_worktree(&["rev-parse", "HEAD"])?);
        tips.extend(in_worktree(&["rev-parse", "--verify", "--quiet", &full_ref]).ok());
    }
    // A worktree folder gone leaves the clone's git dir and its commits.
    let left = if worktree.exists() {
        None
    } else {
        clone::leftover(tree, repo, worktree, branch)?
    };
    tips.extend(left.clone());
    if tips.is_empty() {
        return Ok(false);
    }
    let behind = format!("^{main}");
    let mut args = vec!["rev-list", "--count", behind.as_str()];
    args.extend(tips.iter().map(String::as_str));
    // A clone's commits are not in the checkout.
    let count = if worktree.exists() {
        trusted(tree, repo, worktree)?(&args)?
    } else if left.is_some() {
        clone::in_clone(tree, &clone::git_dir_of(worktree), &args)?
    } else {
        git(tree, repo, args)?
    };
    Ok(count != "0")
}

/// Where a pull request's head stands against `origin/main`, just fetched
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// The head is the branch on its remote, and has the latest `main`
    Current,
    /// The head is the branch on its remote, and lacks the latest `main`
    Behind,
    /// The branch on its remote is not the head the forge reported, which
    /// lags a push by a moment
    Lagging,
}

/// Fetches `origin/main` and `remote`'s branch, and says where `head`, the
/// forge's head of `branch`, stands
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn base_of(
    tree: &ProcessTree,
    repo: &Path,
    branch: &str,
    head: &str,
    remote: &Remote,
    base: &str,
) -> Result<Base, WorktreeError> {
    let tracking = remote.fetch_with_main(tree, repo, branch, base)?;
    if commit_of(tree, repo, &tracking)? != head {
        return Ok(Base::Lagging);
    }
    let tip = format!("origin/{base}");
    if let Some(ancestor) = is_ancestor(repo, &tip, head) {
        return Ok(if ancestor {
            Base::Current
        } else {
            Base::Behind
        });
    }
    // `--is-ancestor` answers no with exit 1, and fails with any other code.
    let current = exit_zero_or_one(tree, repo, ["merge-base", "--is-ancestor", &tip, head])?;
    Ok(if current { Base::Current } else { Base::Behind })
}

/// Fetches `branch` from `remote`, and returns its head there
///
/// Ask git rather than the forge: the forge's head lags a push by a moment.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn origin_head(
    tree: &ProcessTree,
    repo: &Path,
    branch: &str,
    remote: &Remote,
) -> Result<String, WorktreeError> {
    let tracking = remote.fetch(tree, repo, branch)?;
    commit_of(tree, repo, &tracking)
}

/// Fetches `origin/main` and `branch` from `remote` together, and returns the
/// branch's head
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn head_with_main(
    tree: &ProcessTree,
    repo: &Path,
    branch: &str,
    remote: &Remote,
    base: &str,
) -> Result<String, WorktreeError> {
    let tracking = remote.fetch_with_main(tree, repo, branch, base)?;
    commit_of(tree, repo, &tracking)
}

/// The commit `worktree` has checked out
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn head(tree: &ProcessTree, repo: &Path, worktree: &Path) -> Result<String, WorktreeError> {
    let dirs = git_dirs(tree, repo, worktree)?;
    match loose_head(&dirs.0, &dirs.1) {
        Some(commit) => Ok(commit),
        None => output(tree, trusted_git(worktree, &dirs), ["rev-parse", "HEAD"]),
    }
}

/// Gives up the merge in progress in `worktree`, which leaves it at its head
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn abort_merge(tree: &ProcessTree, repo: &Path, worktree: &Path) -> Result<(), WorktreeError> {
    trusted(tree, repo, worktree)?(&["merge", "--abort"]).map(drop)
}

/// The files `worktree` holds that its last commit does not: changed, staged
/// or new and not ignored
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn uncommitted(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
) -> Result<Vec<String>, WorktreeError> {
    // One `status` sees the index, the working tree and the new files. Its
    // second format, since the first starts an entry with a space that `git`
    // trims off the first one.
    let status = trusted(tree, repo, worktree)?(&[
        "status",
        "--porcelain=v2",
        "-z",
        "--untracked-files=all",
    ])?;
    let mut files = status_names(&status);
    files.sort();
    files.dedup();
    Ok(files)
}

// The paths in `git status --porcelain=v2 -z`, where an entry is a kind, its
// fields and then the path, and a rename or copy adds the old path as an
// entry of its own.
fn status_names(status: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut entries = status.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        let fields = match entry.chars().next() {
            Some('?') => 2,
            Some('1') => 9,
            Some('u') => 11,
            Some('2') => {
                names.extend(entries.next().map(str::to_owned));
                10
            }
            _ => continue,
        };
        names.extend(entry.splitn(fields, ' ').last().map(str::to_owned));
    }
    names
}

/// The commit a ticket's own change starts from: the merge base of `base`
/// and the worktree's `HEAD`
///
/// For a script that diffs a ref of its own, which a range would not suit.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn change_start(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    base: &str,
) -> Result<String, WorktreeError> {
    trusted(tree, repo, worktree)?(&["merge-base", base, "HEAD"])
}

/// Git for the worktree, with its git dirs named rather than found, since
/// git that finds them reads the config of whatever repo the worker points
/// it at. The worktree's `commondir` is checked against the repo's, hooks
/// are off, and replace refs are ignored. It reads the `config` of the repo's
/// common dir or of a clone, which the worker must not be able to write.
///
/// # Errors
///
/// [`WorktreeError`] when the worktree's git dirs cannot be named, or its
/// `commondir` is not the repo's.
pub fn trusted<'a>(
    tree: &'a ProcessTree,
    repo: &Path,
    worktree: &'a Path,
) -> Result<impl Fn(&[&str]) -> Result<String, WorktreeError> + 'a, WorktreeError> {
    let dirs = git_dirs(tree, repo, worktree)?;
    Ok(move |args: &[&str]| output(tree, trusted_git(worktree, &dirs), args))
}

/// `git` about `worktree`, as [`trusted`] runs it, for a call that reads
/// its exit code or its output whole
///
/// The caller runs it through the [`ProcessTree`].
///
/// # Errors
///
/// [`WorktreeError`] when the worktree's git dirs are not `repo`'s.
pub fn trusted_command(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
) -> Result<Command, WorktreeError> {
    Ok(trusted_git(worktree, &git_dirs(tree, repo, worktree)?))
}

/// Sets the environment that makes the git `program` runs on `worktree` the
/// trusted one, as [`trusted`] runs it
///
/// The caller runs `program` through the [`ProcessTree`].
///
/// # Errors
///
/// [`WorktreeError`] when the worktree's git dirs are not `repo`'s.
pub fn trust_git_of(
    tree: &ProcessTree,
    program: &mut Command,
    repo: &Path,
    worktree: &Path,
) -> Result<(), WorktreeError> {
    let (own, common) = git_dirs(tree, repo, worktree)?;
    // The caller built `program` from kelpie's environment, which a hook could
    // have left naming another index, object store or namespace.
    for name in GIT_ENV {
        program.env_remove(name);
    }
    program
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env("GIT_DIR", own)
        .env("GIT_WORK_TREE", worktree)
        .env("GIT_COMMON_DIR", common)
        .env("GIT_CONFIG_COUNT", TRUSTED_CONFIG.len().to_string())
        .env("GIT_NO_REPLACE_OBJECTS", "1");
    for (n, (key, value)) in TRUSTED_CONFIG.iter().enumerate() {
        program
            .env(format!("GIT_CONFIG_KEY_{n}"), key)
            .env(format!("GIT_CONFIG_VALUE_{n}"), value);
    }
    Ok(())
}

// The config every trusted git runs with.
const TRUSTED_CONFIG: [(&str, &str); 4] = [
    // None of the hooks is kelpie's to run.
    ("core.hooksPath", "/dev/null"),
    // `status`, `diff`, `reset`, `merge` and `rebase` start this program.
    ("core.fsmonitor", "false"),
    // Git for Windows' system config sets `true`, and a file checked out with
    // CRs differs from its blob in a byte comparison.
    ("core.autocrlf", "false"),
    // `text=auto` in `.gitattributes` converts on checkout to `core.eol`,
    // which is CRLF on Windows by default.
    ("core.eol", "lf"),
];

// The worktree's own git dir and the repo's common one, once the own dir's
// `commondir` is checked to name the repo's. A clone's are its git dir twice,
// once its alternates are checked to name the repo's objects alone.
fn git_dirs(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
) -> Result<(PathBuf, PathBuf), WorktreeError> {
    let foreign = || WorktreeError::Foreign(worktree.to_owned());
    let common = common_dir(tree, repo)?;
    if let Some(dirs) = clone::dirs(&common, worktree)? {
        return Ok(dirs);
    }
    let own = own_git_dir(&common, worktree).ok_or_else(foreign)?;
    let named = std::fs::read_to_string(own.join("commondir")).map_err(|_| foreign())?;
    if canonical(&own.join(named.trim())) != common {
        return Err(foreign());
    }
    Ok((own, common))
}

// `git` about `worktree` with its git dirs named. The common dir is named
// too, so git never reads `commondir` again after the check. Config passed
// down from kelpie's own environment is dropped.
fn trusted_git(worktree: &Path, (own, common): &(PathBuf, PathBuf)) -> Command {
    let mut git = in_repo(worktree);
    git.env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_CONFIG_COUNT")
        .env("GIT_COMMON_DIR", common)
        .arg("--git-dir")
        .arg(own)
        .arg("--work-tree")
        .arg(worktree);
    for (key, value) in TRUSTED_CONFIG {
        git.arg("-c").arg(format!("{key}={value}"));
    }
    git.arg("--no-replace-objects");
    git
}

// Everything about the worktree is read from the project's repo, never from
// inside the worktree. The worker can rewrite its worktree's `.git` file,
// and git run there would believe it.

/// The branch `git worktree list` shows checked out at `worktree`
fn listed_branch(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
) -> Result<Option<String>, WorktreeError> {
    let list = git(tree, repo, ["worktree", "list", "--porcelain"])?;
    let want = canonical(worktree);
    let mut here = false;
    for line in list.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            here = canonical(Path::new(path)) == want;
        } else if let Some(branch) = line.strip_prefix("branch ")
            && here
        {
            return Ok(Some(branch.to_owned()));
        }
    }
    Ok(None)
}

/// The folder under `<common>/worktrees/` whose `gitdir` names `worktree`
fn own_git_dir(common: &Path, worktree: &Path) -> Option<PathBuf> {
    let want = canonical(&worktree.join(".git"));
    std::fs::read_dir(common.join("worktrees"))
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|dir| {
            std::fs::read_to_string(dir.join("gitdir"))
                .is_ok_and(|named| canonical(Path::new(named.trim())) == want)
        })
}

fn common_dir(tree: &ProcessTree, repo: &Path) -> Result<PathBuf, WorktreeError> {
    common::of(tree, repo)
}

// `Path::canonicalize`, or `path` as it is where that fails. A Windows verbatim
// prefix is not stripped, as kelpie has no Windows worktree yet.
fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

fn create(path: &Path) -> Result<(), WorktreeError> {
    std::fs::create_dir_all(path).map_err(|e| WorktreeError::Folder {
        path: path.to_owned(),
        kind: e.kind(),
    })
}
