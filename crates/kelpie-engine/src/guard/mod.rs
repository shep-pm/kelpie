//! Guard: what kelpie reads of a command a worker runs, and what it judges of it
//!
//! The worker's policy (`docs/design/workers.md`, check 2) judges each command
//! a worker runs, so it first reads that command as a shell would. `shell`
//! splits a Bash call into the commands it runs, `wrap` takes off the
//! programs wrapped around one (`env`, `sudo`, `timeout`) and refuses what the
//! guard cannot read, and `script` reads a script the call runs from a file.
//! `judging` then judges each command, and `git` the git calls among them.
//! Kelpie judges in the ACP permission handler, not in a hook, so none of
//! these know a harness's tool JSON.
//!
//! [`judge`] refuses whatever [`LocalPaths`] finds of this machine's in a
//! commit or push, a push to the base branch, a fetch in the ticket's clone
//! (kelpie fetches for the worker), a process stopped by name or pattern, and
//! what the guard cannot read. It names the worktree's git dirs and checks them
//! before it reads them, and never echoes what it matched. `gh` is refused
//! whole until the GitHub slice brings its judge.
//!
//! # Quick start
//!
//! ```
//! use std::path::Path;
//!
//! use kelpie_engine::confine::Verdict;
//! use kelpie_engine::guard::{Checkout, judge};
//! use kelpie_engine::local_paths::LocalPaths;
//! use kelpie_engine::run::RunId;
//! use kelpie_engine::tree::ProcessTree;
//!
//! # let dir = std::env::temp_dir().join(format!("guard-quick-start-{}", RunId::new()?));
//! # std::fs::create_dir_all(&dir)?;
//! # let tree = ProcessTree::new(RunId::new()?, dir.join("run.json"))?;
//! let checkout = Checkout {
//!     tree: &tree,
//!     repo: Path::new("/work/repo"),
//!     worktree: Path::new("/work/ticket-1"),
//!     base: "main",
//! };
//! let local = LocalPaths::new([]);
//!
//! let run = |command| judge(command, Path::new("/work/ticket-1"), None, local.clone(), checkout);
//! assert_eq!(run("ls -l"), Verdict::Allow);
//! assert!(matches!(run("pkill node"), Verdict::Refuse(_)));
//! # std::fs::remove_dir_all(&dir)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod git;
mod judging;
mod script;
mod shell;
mod wrap;

use std::fs;
use std::path::{Path, PathBuf};

use crate::confine::Verdict;
use crate::local_paths::{Leak, LocalPaths, Surface};
use crate::tree::ProcessTree;
use judging::{CallState, Judging};

/// Where the worker's own git lives
#[derive(Debug, Clone, Copy)]
pub struct Checkout<'a> {
    /// The tree every git kelpie starts to read the worktree goes through
    pub tree: &'a ProcessTree,
    /// The project's repo, whose git dirs the worktree's are checked against
    pub repo: &'a Path,
    /// The ticket's worktree
    pub worktree: &'a Path,
    /// The project's base branch, which only the project manager changes
    pub base: &'a str,
}

// A message file larger than this is not a message.
const MESSAGE_FILE_MAX: u64 = 1 << 20;

// How many characters of a word a refusal quotes back to the worker.
pub(crate) const SHOWN_CHARS: usize = 40;

// A shell run inside a shell this many times over is not a worker's command.
const MAX_SHELLS: usize = 4;

// More commands than this in one call are refused rather than each judged:
// a command nested in parentheses is judged once per level around it.
const MAX_COMMANDS: usize = 200;

/// Judges the shell command `command`, run from `cwd`, with `home` the home
/// folder `~` names and `local` what the project keeps off the forge
#[must_use]
pub fn judge(
    command: &str,
    cwd: &Path,
    home: Option<&Path>,
    local: LocalPaths,
    checkout: Checkout<'_>,
) -> Verdict {
    let judging = Judging {
        home: Home::new(home, local),
        checkout,
    };
    let mut call_state = CallState::default();
    judging.line(command, Some(cwd.to_owned()), 0, &mut call_state);
    if call_state.refusals.is_empty() {
        return Verdict::Allow;
    }
    Verdict::Refuse(call_state.refusals.join("\n\n"))
}

// Where `cd` or `git -C` moves from `cwd`: `None` when it cannot be told.
fn moved(cwd: Option<&Path>, to: &str, home: Option<&Path>) -> Option<PathBuf> {
    // A folder the shell works out when it runs.
    if to.contains(['$', '`', '*', '?', '[']) {
        return None;
    }
    match to.strip_prefix('~') {
        Some("") => Some(home?.to_owned()),
        Some(rest) => Some(home?.join(rest.strip_prefix('/')?)),
        None if to == "-" => None,
        None if Path::new(to).is_absolute() => Some(PathBuf::from(to)),
        None => Some(cwd?.join(to)),
    }
}

// The values of a flag, as `--long v`, `--long=v`, `-s v`, `-sv` or `-xs v`.
fn values(args: &[String], long: &[&str], short: &[char]) -> Vec<String> {
    let mut out = Vec::new();
    let mut words = args.iter().take_while(|a| *a != "--");
    while let Some(word) = words.next() {
        if let Some(name) = word.strip_prefix("--") {
            match name.split_once('=') {
                Some((name, value)) if long.contains(&format!("--{name}").as_str()) => {
                    out.push(value.to_owned());
                }
                None if long.contains(&word.as_str()) => {
                    out.extend(words.next().cloned());
                }
                _ => {}
            }
        } else if let Some(cluster) = word.strip_prefix('-')
            && let Some(at) = cluster.find(|c| short.contains(&c))
        {
            let rest = &cluster[at + 1..];
            if rest.is_empty() {
                out.extend(words.next().cloned());
            } else {
                out.push(rest.to_owned());
            }
        }
    }
    out
}

// What the files a flag names hold. `-` is stdin, which a heredoc carries.
fn files(args: &[String], long: &[&str], short: &[char], cwd: Option<&Path>) -> Vec<String> {
    values(args, long, short)
        .into_iter()
        .filter(|f| f != "-")
        .filter_map(|f| {
            // A relative file under a folder the guard could not follow is not read.
            let path = moved(cwd, &f, None)?;
            // A pipe would hold the guard open.
            let small =
                fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MESSAGE_FILE_MAX);
            small.then(|| fs::read_to_string(path).ok()).flatten()
        })
        .collect()
}

/// The home folder, and what else of this machine's a text must not name
struct Home {
    path: Option<PathBuf>,
    local: LocalPaths,
}

impl Home {
    // `/` alone would match every absolute path, and a relative one none.
    fn new(path: Option<&Path>, local: LocalPaths) -> Self {
        let path = path
            .filter(|p| {
                let text = p.to_str().unwrap_or_default().trim_end_matches('/');
                text.len() > 1 && text.starts_with('/')
            })
            .map(Path::to_owned);
        Self { path, local }
    }

    fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    // What the first of `texts` to name this machine names.
    fn find_in_prose(&self, texts: impl IntoIterator<Item = String>) -> Option<Leak> {
        texts
            .into_iter()
            .find_map(|t| self.local.find(&t, Surface::Prose))
    }

    // Each file whose added lines in `patches` name this machine, and what they name.
    fn added(&self, patches: &str) -> Vec<(String, Leak)> {
        let mut file = "";
        let mut out: Vec<(String, Leak)> = Vec::new();
        for line in patches.lines() {
            if let Some(name) = line.strip_prefix("+++ ") {
                file = name.strip_prefix("b/").unwrap_or(name);
            } else if line.starts_with('+')
                && !out.iter().any(|(f, _)| f == file)
                && let Some(leak) = self.local.find(line, Surface::Code)
            {
                out.push((file.to_owned(), leak));
            }
        }
        out.into_iter()
            .map(|(f, leak)| (format!("`{f}`"), leak))
            .collect()
    }

    // An address is taken out; a path is written from the repo's root.
    fn refusal(&self, what: &str, leak: Leak, fix: &str) -> String {
        let advice = match leak {
            Leak::Path => {
                " Write a path in the repo from its root (`src/lib.rs`), and leave out one \
                 outside it."
            }
            Leak::Lan | Leak::Private => "",
        };
        format!("{what} carries {leak}. {fix}{advice}")
    }
}

const WRITE: &str = "Take it out, then try again.";

const REWRITE: &str = "Those commits have not left this machine: take it out and rewrite \
                       them, since a new commit on top would still send the old one.";

#[cfg(test)]
mod tests;
