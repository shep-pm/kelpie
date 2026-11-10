//! `git commit`, `git tag` and `git push` in a worker's Bash call
//!
//! A `git fetch`, `git pull`, `git remote update` or `git remote prune` in the
//! ticket's own repo is answered before it runs: each writes every
//! `refs/remotes/origin/*`, which a worker may not, and kelpie fetches
//! `origin/main` for it. One git is pointed at another repo is left to git.
//!
//! Messages are read from the command, and what a commit adds or a push sends
//! from the worktree's own git. What the guard cannot vouch for is refused, as
//! is a push to the base branch.

mod front;
mod runs;

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use super::shell::Quoting;
use super::wrap::Reach;
use super::{Checkout, Home, REWRITE, SHOWN_CHARS, WRITE, files, moved, values};
use crate::worktree::{self, WorktreeError};
use front::{BUILTINS, front};
pub(super) use runs::{bisect_run, makes_repo, scripts};

/// The worktree reads one call has made, so none runs twice
#[derive(Debug, Default)]
pub(super) struct Reads(HashMap<Read, Result<Vec<String>, String>>);

/// A git command, its wrappers off
#[derive(Debug, Clone, Copy)]
pub(super) struct Git<'a> {
    /// Its words, `git` first
    pub words: &'a [String],
    /// How each of its words was written, aligned to `words`
    pub quoting: &'a [Quoting],
    /// The heredoc bodies it reads
    pub heredocs: &'a [String],
    /// Whether an assignment in front points it at another repo
    pub redirected: bool,
    /// Whether an assignment gives it config from the call's text
    pub configured: bool,
    /// Whether it surely runs, or may only be named
    pub reach: Reach,
    /// Whether the call made a repo before it
    pub after_new_repo: bool,
}

// `words` without the redirections a shell reads: `2>&1`, `>out`, `&>out`, and the
// target word after a bare `>` or `2>`. A quoted `>` is text. A heredoc's `<<` stays,
// since `git commit -F -` reads it. The quoting of the words left comes back with them.
fn without_redirects(words: &[String], quoting: &[Quoting]) -> (Vec<String>, Vec<Quoting>) {
    let mut kept = Vec::with_capacity(words.len());
    let mut kept_quoting = Vec::with_capacity(quoting.len());
    let mut target = false;
    for (at, word) in words.iter().enumerate() {
        if std::mem::take(&mut target) {
            continue;
        }
        let held = |i: usize| {
            quoting
                .get(at)
                .is_some_and(|q| q.quoted.get(i) == Some(&true))
        };
        match (at > 0).then(|| redirect(word, held)).flatten() {
            Some(bare) => target = bare,
            None => {
                kept.push(word.clone());
                kept_quoting.extend(quoting.get(at).cloned());
            }
        }
    }
    (kept, kept_quoting)
}

// Whether `word` is a redirection to an output file, and if so, whether it is bare, so
// that the next word is its target: an optional run of digits or a `&`, then `>` or `>>`,
// and a `&` for a descriptor, as `2>&1` and `>&out` have.
fn redirect(word: &str, held: impl Fn(usize) -> bool) -> Option<bool> {
    let digits = word.chars().take_while(char::is_ascii_digit).count();
    let lead = if word.starts_with('&') { 1 } else { digits };
    let rest = word.get(lead..)?.strip_prefix('>')?;
    let marks = 1 + usize::from(rest.starts_with('>'));
    let dup = usize::from(word.get(lead + marks..)?.starts_with('&'));
    let operator = lead + marks + dup;
    (!(0..operator).any(held)).then_some(word.len() == operator)
}

/// Judges one git command run in `cwd`
pub(super) fn judge(
    git: Git<'_>,
    cwd: Option<&Path>,
    home: &Home,
    checkout: Checkout<'_>,
    reads: &mut Reads,
) -> Vec<String> {
    // A redirect is the shell's, and git is never given it.
    let (words, quoting) = without_redirects(git.words, git.quoting);
    let git = Git {
        words: &words,
        quoting: &quoting,
        ..git
    };
    let runs = git.reach == Reach::Runs;
    // A program kelpie does not know may only name git, as `rg git` does.
    let front = match front(git.words) {
        Ok(front) => front,
        Err(refusal) if runs => return vec![refusal],
        Err(_) => return Vec::new(),
    };
    let Some(at) = front.sub else {
        return Vec::new();
    };
    let sub = git.words[at].as_str();
    let mut out = Vec::new();
    if git.configured {
        out.push(
            "kelpie cannot check git given config in its environment (`GIT_CONFIG_*` or \
             `GIT_ALLOW_PROTOCOL`), which can name a program git runs: run it without."
                .to_owned(),
        );
    }
    if front.exec_path {
        out.push(
            "kelpie cannot check git run with `--exec-path`, which has it run the programs in \
             another folder, and a worker may have written them: run it without."
                .to_owned(),
        );
    }
    if let Some(key) = front
        .config
        .iter()
        .map(|c| runs::key(c))
        .find(|k| !runs::safe(k))
    {
        out.push(format!(
            "kelpie cannot check git run with the config `{}`, which can name a program git \
             runs: run it without.",
            key.chars().take(SHOWN_CHARS).collect::<String>()
        ));
    }
    // An alias or an external command could be anything, and no alias
    // shares a built-in's name. Some built-ins run a command of their own,
    // as `submodule foreach` does, so they are left out too.
    if !BUILTINS.contains(&sub) {
        if runs {
            out.push(format!(
                "kelpie runs only the git commands it knows, and `{}` is not one: run the plain \
                 git command you need, or the one an alias stands for.",
                sub.chars().take(SHOWN_CHARS).collect::<String>()
            ));
        }
        return out;
    }
    let dir = front.moves.iter().fold(cwd.map(Path::to_owned), |dir, to| {
        moved(dir.as_deref(), to, home.path())
    });
    let args = &git.words[at + 1..];
    // A fetch of the shared clone is answered rather than run. One in a repo
    // the worker made, or pointed at another repo by `--git-dir`, `--work-tree`,
    // `--bare` or a `GIT_` assignment, is left to git: kelpie cannot tell which
    // repo it names, and it is no write to the ticket's branch.
    if runs
        && writes_remote_refs(sub, args)
        && !(front.redirected || git.redirected)
        && !git.after_new_repo
        && fetch_reaches_clone(&clone_roots(checkout), dir.as_deref())
    {
        out.push(fetch_answered(checkout.base));
        return out;
    }
    if sub == "config" {
        if runs::config_edits(args) {
            out.push(
                "kelpie cannot check git config opened in an editor, which can write any key, \
                 and one that names a program for git to run: set a key with `git config \
                 <key> <value>`."
                    .to_owned(),
            );
        }
        if let Some(key) = runs::config_set(args)
            && !runs::safe(&key)
        {
            out.push(format!(
                "kelpie cannot check git once its config names a program for it to run, and \
                 `{}` can: set only keys that run nothing, such as `color.*`, `user.*` or \
                 `core.quotepath`.",
                key.chars().take(SHOWN_CHARS).collect::<String>()
            ));
        }
    }
    // Where a push goes is judged wherever it runs from.
    let pushed = if sub == "push" {
        match refspecs(args) {
            Ok(specs) if specs.iter().any(|spec| spec.to_base(checkout.base)) => {
                out.push(to_base(checkout.base));
                return out;
            }
            Ok(specs) => specs,
            Err(refusal) => {
                // Behind a program kelpie does not know, a push is refused below.
                if runs {
                    out.push(refusal);
                    return out;
                }
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    if sub == "commit" || sub == "tag" {
        let messages: Vec<String> = values(args, &cut_short_names(&["--message"]), &['m'])
            .into_iter()
            .chain(files(
                args,
                &cut_short_names(&["--file"]),
                &['F'],
                dir.as_deref(),
            ))
            .chain(git.heredocs.iter().cloned())
            .collect();
        if let Some(leak) = home.find_in_prose(messages.iter().cloned()) {
            out.push(home.refusal(&format!("this {sub}'s message"), leak, WRITE));
        }
    }
    // A commit is read for what names this machine; a push, for its branch too.
    if !matches!(sub, "commit" | "push") {
        return out;
    }
    if !runs {
        out.push(format!(
            "kelpie checks a {sub} only when git is the command's program, or runs behind a \
             wrapper it knows such as `env` or `sudo`: run git directly."
        ));
        return out;
    }
    if git.redirected || front.redirected || front.configured {
        out.push(format!(
            "kelpie checks a {sub} only in this worktree's own git, as its config stands: run \
             it without `--git-dir`, `--work-tree`, `-c`, `--config-env` or a `GIT_` variable."
        ));
        return out;
    }
    if !in_own_repo(checkout.worktree, dir.as_deref()) {
        out.push(format!(
            "this {sub} runs outside this worktree's own repo, which kelpie does not check: \
             run it from the worktree."
        ));
        return out;
    }
    // A folder there before the call may hold a repo the call makes.
    if git.after_new_repo {
        out.push(format!(
            "this {sub} follows a `git init`, `git clone` or `git worktree add` in the same \
             call, which makes a repo kelpie does not check: run it in a call of its own."
        ));
        return out;
    }
    let keys = if sub == "push" {
        let current = pushed.iter().any(Refspec::sends_current_branch);
        let sources: Vec<String> = (pushed.iter())
            .filter(|spec| !spec.source.is_empty())
            .map(|spec| spec.source.clone())
            .collect();
        let sent = sources.iter().cloned().map(Read::Push);
        std::iter::once(Read::PushConfig)
            .chain(current.then_some(Read::Branch))
            .chain(sent)
            .collect()
    } else {
        let (all, paths) = match commit_paths(args) {
            _ if all_tracked(args) => (true, Vec::new()),
            // A path list kept in a file, or paths from a folder the read cannot
            // follow them from, are every tracked change.
            None => (true, Vec::new()),
            Some(paths) if !paths.is_empty() && !at_root(checkout.worktree, dir.as_deref()) => {
                (true, Vec::new())
            }
            Some(paths) => (false, paths),
        };
        vec![Read::Commit { all, paths }]
    };
    for key in keys {
        let found = reads
            .0
            .entry(key.clone())
            .or_insert_with(|| {
                worktree::trusted(checkout.tree, checkout.repo, checkout.worktree)
                    .and_then(|run| read(&key, home, checkout.base, run))
                    .map_err(|e| e.to_string())
            })
            .clone();
        match found {
            Ok(found) => out.extend(found),
            // Git that cannot be read is refused, not let through unchecked.
            Err(e) => out.push(format!(
                "kelpie cannot read this worktree's git to check this {sub}: {e}"
            )),
        }
    }
    out
}

/// One read of the worktree's git, keyed so a call makes it once
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Read {
    /// What a commit adds: staged, plus the content of the `paths` it names, or
    /// every tracked change with `all`
    Commit { all: bool, paths: Vec<String> },
    /// What a push of this source sends past `origin`'s base branch
    Push(String),
    /// The branch checked out, where a push of `HEAD` alone goes
    Branch,
    /// Config that has a push send more than the refs it names
    PushConfig,
}

// Push options that take no value. Git takes an abbreviated long option, so
// only these exact spellings pass.
const PUSH_FLAGS: [&str; 30] = [
    "-u",
    "--set-upstream",
    "-q",
    "--quiet",
    "-v",
    "--verbose",
    "--progress",
    "--no-progress",
    "-n",
    "--dry-run",
    "--porcelain",
    "--no-verify",
    "--verify",
    "--atomic",
    "--no-atomic",
    "--thin",
    "--no-thin",
    "--signed",
    "--no-signed",
    "--force-with-lease",
    "--no-force-with-lease",
    "--force-if-includes",
    "--no-force-if-includes",
    "-4",
    "-6",
    "--ipv4",
    "--ipv6",
    "-f",
    "--force",
    "--no-recurse-submodules",
];

// Push options that take the next word, or an `=` value.
const PUSH_VALUED: [&str; 4] = ["-o", "--push-option", "--repo", "--force-with-lease"];

/// One refspec of a push
#[derive(Debug, Clone, PartialEq, Eq)]
struct Refspec {
    /// What it sends from, empty for a delete
    source: String,
    /// The branch it names on the remote, `None` when git takes the source's
    destination: Option<String>,
}

impl Refspec {
    // Git matches `main`, `heads/main` and `refs/heads/main` to the remote's
    // branch. A source alone goes to the branch of its own name.
    fn to_base(&self, base: &str) -> bool {
        let named = self.destination.as_deref().unwrap_or(&self.source);
        let full = format!("refs/heads/{base}");
        [base, &full["refs/".len()..], &full].contains(&named)
    }

    // `HEAD` or `@` alone goes to the branch checked out.
    fn sends_current_branch(&self) -> bool {
        self.destination.is_none() && matches!(self.source.as_str(), "HEAD" | "@")
    }
}

// What a worker is told instead of a fetch of the shared clone
fn fetch_answered(base: &str) -> String {
    format!(
        "kelpie fetches `origin/{base}` in this repo before each of your turns, and the \
         clone's other refs are not yours to write, so a fetch here fails. To take {base} \
         into your branch, run `git merge origin/{base}`."
    )
}

// What to do about a tag whose message names this machine
const RETAG: &str = "That tag has not left this machine: delete it with `git tag -d <name>` and \
                     make it again without that.";

/// The refusal for a push to the base branch
fn to_base(base: &str) -> String {
    format!(
        "only the project manager changes `{base}`, on the maintainer's ruling: push your own \
         branch with `git push origin HEAD`."
    )
}

// A push's refspecs; `HEAD` alone when it names none.
fn refspecs(args: &[String]) -> Result<Vec<Refspec>, String> {
    let unknown = |word: &str| {
        Err(format!(
            "kelpie checks a push only in the forms it knows, and not with `{}`: push HEAD \
             to a branch, with no more than `-u`.",
            word.chars().take(SHOWN_CHARS).collect::<String>()
        ))
    };
    let mut positional = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        let word = word.as_str();
        let name = word.split_once('=').map_or(word, |(name, _)| name);
        if word == "--" {
            positional.extend(words.by_ref().cloned());
        } else if PUSH_FLAGS.contains(&word) || word.contains('=') && PUSH_VALUED.contains(&name) {
        } else if PUSH_VALUED.contains(&word) {
            words.next();
        } else if matches!(
            word,
            "--recurse-submodules=no" | "--recurse-submodules=check"
        ) {
        } else if word.starts_with('-') {
            return unknown(word);
        } else {
            positional.push(word.to_owned());
        }
    }
    // The first is the remote.
    let mut out = Vec::new();
    for refspec in positional.iter().skip(1) {
        let refspec = refspec.strip_prefix('+').unwrap_or(refspec);
        let (source, destination) = match refspec.split_once(':') {
            Some((source, destination)) => (source, Some(destination)),
            None => (refspec, None),
        };
        if source.starts_with('-') {
            return Err("kelpie cannot read a push source that starts with `-`.".into());
        }
        // `:` alone pushes every branch that matches one on the remote, and
        // `:branch` deletes.
        if source.is_empty() && destination.is_none_or(str::is_empty) {
            return unknown(refspec);
        }
        out.push(Refspec {
            source: source.to_owned(),
            destination: destination.map(str::to_owned),
        });
    }
    if positional.len() < 2 {
        out.push(Refspec {
            source: "HEAD".into(),
            destination: None,
        });
    }
    Ok(out)
}

// `-a` or `--all`, which commit every tracked change, staged or not.
fn all_tracked(args: &[String]) -> bool {
    args.iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "--all" || a.starts_with('-') && !a.starts_with("--") && a.contains('a'))
}

// Each annotated tag `source` names, through tags of tags, and its message.
fn tag_messages(
    source: &str,
    run: &impl Fn(&[&str]) -> Result<String, WorktreeError>,
) -> Result<Vec<String>, WorktreeError> {
    // A chain longer than this is no tag a worker made by hand.
    const DEPTH: usize = 16;
    let mut messages = Vec::new();
    let mut at = source.to_owned();
    for _ in 0..DEPTH {
        if run(&["cat-file", "-t", &at])?.trim() != "tag" {
            break;
        }
        let tag = run(&["cat-file", "tag", &at])?;
        let (head, message) = tag.split_once("\n\n").unwrap_or((&tag, ""));
        messages.push(message.to_owned());
        match head.lines().find_map(|line| line.strip_prefix("object ")) {
            Some(next) => at = next.to_owned(),
            None => break,
        }
    }
    Ok(messages)
}

// Git takes any start of a long option that names only one, as `--mess`.
fn cut_short_names<'a>(names: &[&'a str]) -> Vec<&'a str> {
    names
        .iter()
        .flat_map(|name| (3..=name.len()).map(|n| &name[..n]))
        .collect()
}

// Whether `word` is `option`, or a start of it git takes for it, with an `=` value
// or without. One that could be another option too is an error to git.
fn cut_short(word: &str, option: &str) -> bool {
    let name = word.split_once('=').map_or(word, |(name, _)| name);
    name.len() >= 3 && option.starts_with(name)
}

// The paths a `git commit` names, which it commits as they are in the working tree.
// `None` when they come from a file.
fn commit_paths(args: &[String]) -> Option<Vec<String>> {
    // Options that take the next word as their value, or an `=` one.
    const VALUED: [&str; 11] = [
        "--message",
        "--file",
        "--reuse-message",
        "--reedit-message",
        "--template",
        "--author",
        "--date",
        "--cleanup",
        "--trailer",
        "--fixup",
        "--squash",
    ];
    // Short options whose value is the rest of their word, or the next word.
    const SHORT_VALUED: [char; 5] = ['m', 'F', 'C', 'c', 't'];
    let mut paths = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if word == "--" {
            paths.extend(words.by_ref().cloned());
        } else if let Some(long) = word.strip_prefix("--") {
            if cut_short(word, "--pathspec-from-file") {
                return None;
            }
            let valued = VALUED.iter().any(|option| cut_short(word, option));
            if valued && !long.contains('=') {
                words.next();
            }
        } else if let Some(cluster) = word.strip_prefix('-').filter(|c| !c.is_empty()) {
            if let Some(at) = cluster.find(SHORT_VALUED)
                && cluster[at + 1..].is_empty()
            {
                words.next();
            }
        } else {
            paths.push(word.clone());
        }
    }
    Some(paths)
}

// Whether `dir` is the worktree's own folder, which the pathspecs of a read run from.
fn at_root(worktree: &Path, dir: Option<&Path>) -> bool {
    matches!(
        (worktree.canonicalize(), dir.map(Path::canonicalize)),
        (Ok(worktree), Some(Ok(dir))) if worktree == dir
    )
}

// The folders whose repo is the ticket's shared clone: its own worktree, the
// project's repo, and every other worktree the repo lists.
fn clone_roots(checkout: Checkout<'_>) -> Vec<PathBuf> {
    let mut roots = vec![checkout.worktree.to_owned(), checkout.repo.to_owned()];
    let listed = checkout.repo.join(".git").join("worktrees");
    for entry in fs::read_dir(listed).into_iter().flatten().flatten() {
        // Each lists the `.git` file of its worktree.
        let Ok(gitdir) = fs::read_to_string(entry.path().join("gitdir")) else {
            continue;
        };
        roots.extend(Path::new(gitdir.trim()).parent().map(Path::to_owned));
    }
    roots.iter().filter_map(|r| r.canonicalize().ok()).collect()
}

// Plain patches: no colour, and no program the repo's config names.
const PLAIN: [&str; 4] = [
    "--unified=0",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
];

// What a commit adds, or a push sends, that names this machine, and a
// push of `HEAD` from the base branch.
fn read(
    key: &Read,
    home: &Home,
    base: &str,
    run: impl Fn(&[&str]) -> Result<String, WorktreeError>,
) -> Result<Vec<String>, WorktreeError> {
    // `--unified` alone makes `git log` print patches, so only patch reads take these.
    let patches = |args: &[&str]| run(&[args, &PLAIN].concat());
    let mut out = Vec::new();
    match key {
        Read::Branch => {
            let branch = run(&["rev-parse", "--symbolic-full-name", "HEAD"])?;
            if branch.trim() == format!("refs/heads/{base}") {
                out.push(to_base(base));
            }
        }
        Read::PushConfig => {
            let config = run(&["config", "--list", "-z"])?;
            for entry in config.split('\0') {
                let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
                let key = key.to_lowercase();
                if runs::pushes_more(&key, &value.to_lowercase()) {
                    out.push(format!(
                        "this repo's git config sets `{key}`, which has a push send more than \
                         the refs it names, and kelpie checks only those: take it back with \
                         `{}`, adding `--global` if it is set there, then push.",
                        runs::push_fix(&key)
                    ));
                }
            }
        }
        Read::Push(source) => {
            // A file written and committed in one call is not staged when the
            // commit is judged, so the push reads what it sends. The base is
            // `origin`'s, which the worker cannot move, unlike its own branch's.
            let tip = format!("refs/remotes/origin/{base}");
            let log = run(&["log", "--format=%B", source, "--not", &tip])?;
            if let Some(leak) = home.find_in_prose([log]) {
                let what = "a message in the commits this push sends";
                out.push(home.refusal(what, leak, REWRITE));
            }
            // A tag's own message goes with it, and `log` never reads one.
            for message in tag_messages(source, &run)? {
                if let Some(leak) = home.find_in_prose([message]) {
                    let what = "the message of a tag this push sends";
                    out.push(home.refusal(what, leak, RETAG));
                }
            }
            let sent = patches(&["log", "-p", "--format=", source, "--not", &tip])?;
            for (file, leak) in home.added(&sent) {
                let what = format!("{file} in the commits this push sends");
                out.push(home.refusal(&what, leak, REWRITE));
            }
        }
        Read::Commit { all, paths } => {
            let mut diffs = Vec::new();
            if *all {
                diffs.push(patches(&["diff", "HEAD"])?);
            } else {
                diffs.push(patches(&["diff", "--cached"])?);
                // `git commit <path>` takes the path's working-tree content.
                if !paths.is_empty() {
                    let mut args = vec!["diff", "HEAD"];
                    args.extend(PLAIN);
                    args.push("--");
                    args.extend(paths.iter().map(String::as_str));
                    diffs.push(run(&args)?);
                }
            }
            for diff in diffs {
                for (file, leak) in home.added(&diff) {
                    let refusal = home.refusal(&format!("this commit's {file}"), leak, WRITE);
                    if !out.contains(&refusal) {
                        out.push(refusal);
                    }
                }
            }
        }
    }
    Ok(out)
}

// Whether `dir` is the worker's own repo: in the worktree and in no repo it
// made there, bare ones too. A folder the guard cannot follow (`None`), or
// one that is not there when the hook runs, such as one the call will make, is not.
fn in_own_repo(worktree: &Path, dir: Option<&Path>) -> bool {
    let (Ok(worktree), Some(Ok(dir))) = (worktree.canonicalize(), dir.map(Path::canonicalize))
    else {
        return false;
    };
    dir.starts_with(&worktree)
        && dir
            .ancestors()
            .take_while(|a| *a != worktree)
            .all(no_repo_between)
}

// Whether a fetch reaches the ticket's shared clone: from its repo or any of its
// worktrees. git searches upward for a repo, so a folder kelpie cannot follow, or
// one the call made inside a worktree, reaches it too. Only a repo of the
// worker's own, in the folder or between it and the clone's root, keeps a fetch
// out of the answer.
fn fetch_reaches_clone(roots: &[PathBuf], dir: Option<&Path>) -> bool {
    // A folder kelpie cannot follow: the only repo it can name is the ticket's.
    let (false, Some(dir)) = (roots.is_empty(), dir) else {
        return true;
    };
    // The deepest part of the path that is there, with its links resolved, and
    // the names after it, which are not there yet.
    let Some(at) = dir.ancestors().position(Path::exists) else {
        return true;
    };
    let Some(base) = dir.ancestors().nth(at).and_then(|a| a.canonicalize().ok()) else {
        return true;
    };
    let folder = {
        let mut missing: Vec<&OsStr> = dir
            .ancestors()
            .take(at)
            .filter_map(Path::file_name)
            .collect();
        missing.reverse();
        missing
            .into_iter()
            .fold(base, |folder, name| folder.join(name))
    };
    roots.iter().any(|root| {
        folder.starts_with(root)
            && folder
                .ancestors()
                .take_while(|a| a != root)
                .all(no_repo_between)
    })
}

// Whether one folder of the worktree holds no repo of its own
fn no_repo_between(folder: &Path) -> bool {
    folder.join(".git").symlink_metadata().is_err()
        && !["HEAD", "objects", "refs"]
            .iter()
            .all(|name| folder.join(name).symlink_metadata().is_ok())
}

// Whether the command writes the clone's remote refs. `fetch` and `pull` do,
// and so do `remote update`, which fetches every remote, and `remote prune`,
// which deletes the same refs.
fn writes_remote_refs(sub: &str, args: &[String]) -> bool {
    match sub {
        "fetch" | "pull" => true,
        "remote" => args
            .iter()
            .find(|w| !w.starts_with('-'))
            .is_some_and(|w| matches!(w.as_str(), "update" | "prune")),
        _ => false,
    }
}
