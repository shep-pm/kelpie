//! The pull request a worker hands over, opened from the file it left
//!
//! A worker pushes with a token that opens no pull request, so it writes the
//! title and body in a file of its own and kelpie opens the draft when a turn
//! ends with the branch pushed ([`crate::worker`], `docs/design/workers.md`,
//! Pushing and the pull request). A copy of the file as kelpie last sent it sits
//! beside it, so a turn that left the file alone asks nothing of the forge.
//!
//! The file is read through [`crate::worker_folder`], so a link the worker planted
//! there is refused and never followed. A body has its soft-wrapped lines joined
//! before it is sent, and ends `Resolves #<ticket>` whether the worker wrote that
//! line or not.

use std::fmt;
use std::num::NonZeroU64;
use std::path::Path;

use crate::ticket::{ForgeError, GithubTickets, Pulls};
use crate::tree::ProcessTree;
use crate::worker_folder;
use crate::worktree;

use super::paragraphs;
use super::start::{PULL_REQUEST_FILE, Paths, branch_of};

/// The copy of the hand-over file as kelpie last put it on the pull request
pub(super) const APPLIED: &str = "pull-request.applied.md";

/// The line that closes the ticket when the worker left it out
const RESOLVES: &str = "Resolves #";

/// A turn's hand-over: the file its worker left, and the sources to open from it
pub struct HandOver<'a> {
    /// Where the worker's folders are
    pub paths: &'a Paths,

    /// The ticket, which names its branch, its worktree and its hand-over file
    pub ticket: &'a str,

    /// The tree kelpie's own git reads go through (ADR 0001)
    pub tree: &'a ProcessTree,

    /// The project's checkout, which holds the remote the branch was cut from
    pub folder: &'a Path,

    /// The branch the pull request opens into
    pub base: &'a str,

    /// The forge that opens the draft and edits its text
    pub pulls: &'a dyn Pulls,

    /// The ticket source that knows which pull requests are open
    pub tickets: &'a dyn GithubTickets,
}

// The sources are trait objects, which no derive can print
impl fmt::Debug for HandOver<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HandOver")
            .field("paths", &self.paths)
            .field("ticket", &self.ticket)
            .field("folder", &self.folder)
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

/// What kelpie made of a turn's hand-over
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// The worker left no file to open from
    NoFile,

    /// The branch on the forge is not the commit the worktree has, so nothing is opened
    NotPushed,

    /// A draft was opened with this number
    Opened(NonZeroU64),

    /// Pull request `number`'s title and body are what the file says now
    Edited(NonZeroU64),

    /// The file is as kelpie last sent it, so nothing was asked of the forge
    Unchanged,

    /// A refusal the worker can fix in its own file, worded for it
    Told(String),

    /// The forge could not be asked, or its answer could not be read
    Failed(ForgeError),
}

/// A pull request's title and body, as its hand-over file holds them
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Text {
    /// The first line, which is a conventional commit subject
    pub(super) title: String,

    /// The rest, its paragraphs joined
    pub(super) body: String,
}

/// Why a file's first line is no title kelpie sends
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Untitled {
    /// The first line is blank, which GitHub would refuse
    Blank,

    /// The first line, which is not a conventional commit subject, such as a
    /// template's first heading
    NotConventional(String),
}

/// Why kelpie opened nothing from the file
#[derive(Debug, Clone, PartialEq, Eq)]
enum Refused {
    /// Its first line holds no title
    Blank,

    /// Its first line is not a conventional commit subject
    Title(String),

    /// It is a link, a folder, or none of the worker's own
    Unreadable(worker_folder::Error),

    /// The forge refused the text
    Forge(ForgeError),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank => f.write_str("has no title on its first line"),
            Self::Title(line) => {
                write!(
                    f,
                    "its first line, `{line}`, is not a conventional commit subject"
                )
            }
            Self::Unreadable(why) => write!(f, "cannot be read: {why}"),
            Self::Forge(why) => why.fmt(f),
        }
    }
}

impl From<Untitled> for Refused {
    fn from(untitled: Untitled) -> Self {
        match untitled {
            Untitled::Blank => Self::Blank,
            Untitled::NotConventional(line) => Self::Title(line),
        }
    }
}

/// Open or edit the pull request of a turn that ended, from the file its worker left
///
/// In order: the file, its title, the branch, and the pull request already open on
/// it. A draft is opened when the branch is pushed and none is open; an open one
/// is edited only when the file differs from the copy kelpie last sent.
///
/// A file that is a link, or a title that is not a conventional commit subject, is
/// [`Outcome::Told`] with what the worker can change, and opens nothing: the file
/// stays unapplied, so the next turn's end tries it again. So is a text the forge
/// refuses for naming this machine, whose wording never repeats what the check
/// found. A branch kelpie cannot read is not believed pushed.
#[must_use]
pub fn after_turn(ctx: &HandOver<'_>) -> Outcome {
    let state = ctx.paths.state_dir(ctx.ticket);
    let file = match worker_folder::read(&state, Path::new(PULL_REQUEST_FILE)) {
        Ok(Some(file)) => file,
        Ok(None) => return Outcome::NoFile,
        Err(why) => return told(&state, &Refused::Unreadable(why)),
    };
    let text = match parse(&file) {
        Ok(text) => text,
        Err(untitled) => return told(&state, &Refused::from(untitled)),
    };
    let branch = branch_of(ctx.ticket);
    if !pushed(ctx, &branch) {
        return Outcome::NotPushed;
    }
    let body = closes(&text.body, ctx.ticket);
    let open = match ctx.tickets.open_pulls() {
        Ok(pulls) => pulls.into_iter().find(|pull| pull.branch == branch),
        Err(why) => return Outcome::Failed(why),
    };
    match open {
        None => match ctx
            .pulls
            .create_draft(&branch, ctx.base, &text.title, &body)
        {
            Ok(number) => {
                keep_applied(&state, &file);
                Outcome::Opened(number)
            }
            Err(why) => refused(&state, why),
        },
        Some(pull) => {
            if applied(&state).is_some_and(|applied| applied == file) {
                return Outcome::Unchanged;
            }
            match ctx.pulls.edit(pull.number, &text.title, &body) {
                Ok(()) => {
                    keep_applied(&state, &file);
                    Outcome::Edited(pull.number)
                }
                Err(why) => refused(&state, why),
            }
        }
    }
}

/// The title and body in `file`: its first line is the title, its leading `#`s
/// left off, and the rest is the body, blank lines dropped from each end and its
/// soft-wrapped lines joined by [`paragraphs::join`]
///
/// [`Untitled`] when the first line is blank or is not a conventional commit
/// subject. Any lowercase word is a type, since kelpie knows no project's list.
pub(super) fn parse(file: &str) -> Result<Text, Untitled> {
    let file = file.trim_start();
    let (first, rest) = file.split_once('\n').unwrap_or((file, ""));
    let title = first.trim_start_matches('#').trim();
    if title.is_empty() {
        return Err(Untitled::Blank);
    }
    if !conventional(title) {
        return Err(Untitled::NotConventional(title.to_owned()));
    }
    Ok(Text {
        title: title.to_owned(),
        body: paragraphs::join(&blank_ends(rest)),
    })
}

/// `text` with its blank lines dropped from each end, every indent kept
///
/// [`str::trim`] would shave the four spaces off a body that opens with an
/// indented code block, and [`paragraphs::join`] would then read its lines as an
/// ordinary paragraph and join them into one line.
fn blank_ends(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(first) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let last = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .unwrap_or(first);
    lines[first..=last].join("\n")
}

/// `type(scope)!: summary`, with any lowercase word as the type
fn conventional(title: &str) -> bool {
    let Some((head, summary)) = title.split_once(':') else {
        return false;
    };
    let head = head.strip_suffix('!').unwrap_or(head);
    let (kind, scope) = match head.split_once('(') {
        Some((kind, scope)) => (kind, Some(scope)),
        None => (head, None),
    };
    let typed = !kind.is_empty() && kind.chars().all(|c| c.is_ascii_lowercase());
    let scoped = scope.is_none_or(|scope| scope.ends_with(')'));
    typed && scoped && !summary.trim().is_empty()
}

/// `body`, ending the line that closes `ticket` when it holds no such line
fn closes(body: &str, ticket: &str) -> String {
    let line = format!("{RESOLVES}{ticket}");
    if (body.lines()).any(|held| held.trim() == line) {
        return body.to_owned();
    }
    if body.is_empty() {
        return line;
    }
    format!("{body}\n\n{line}")
}

/// Whether the branch on the forge is the commit the worktree has
///
/// A worktree kelpie cannot read, or a forge that cannot be asked, is answered
/// here as not pushed: nothing is believed about a branch either leaves.
fn pushed(ctx: &HandOver<'_>, branch: &str) -> bool {
    let Ok(head) = worktree::head(ctx.tree, ctx.folder, &ctx.paths.worktree(ctx.ticket)) else {
        return false;
    };
    match ctx.pulls.branch_head(branch) {
        Ok(on_forge) => on_forge.is_some_and(|sha| sha == head),
        Err(why) => {
            eprintln!("kelpie: cannot read the branch {branch} on the forge: {why}");
            false
        }
    }
}

/// A forge's refusal, told to the worker where it can fix it, and failed otherwise
fn refused(state: &Path, why: ForgeError) -> Outcome {
    match why {
        ForgeError::LocalPath => told(state, &Refused::Forge(why)),
        _ => Outcome::Failed(why),
    }
}

/// A refusal the worker can fix in its own file, worded with the file and what to
/// change in it
///
/// A refusal for naming this machine never repeats what the check found, so the
/// text can reach the worker and the log with nothing of this machine in it.
fn told(state: &Path, refused: &Refused) -> Outcome {
    let fix = match refused {
        Refused::Blank => "Put the title on the file's first line",
        Refused::Title(_) => {
            "Write the title as the file's first line, a conventional commit \
                             subject such as `fix(parser): keep the last line`, above any \
                             template's text: a template's heading is never the title"
        }
        Refused::Unreadable(_) => "Write the text in a plain file of that name",
        Refused::Forge(ForgeError::LocalPath) => {
            "Rewrite the file without the path or address it names"
        }
        Refused::Forge(why) => return Outcome::Failed(why.clone()),
    };
    Outcome::Told(format!(
        "{}: {refused}. {fix}",
        state.join(PULL_REQUEST_FILE).display()
    ))
}

/// The copy kelpie last sent, or none when it sent nothing
fn applied(state: &Path) -> Option<String> {
    worker_folder::read(state, Path::new(APPLIED))
        .ok()
        .flatten()
}

/// Keep `file` as what kelpie just sent, so a later turn that leaves it alone edits
/// nothing
fn keep_applied(state: &Path, file: &str) {
    if let Err(why) = worker_folder::write(state, Path::new(APPLIED), file) {
        eprintln!(
            "kelpie: cannot write {}: {why}",
            state.join(APPLIED).display()
        );
    }
}
