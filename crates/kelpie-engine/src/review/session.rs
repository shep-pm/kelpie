//! A read as an ACP session: the real side of [`Reviewer`]
//!
//! [`SessionReviewer`] is a ticket's reviewer on one binding. Each read is a
//! fresh session of the reviewer's harness, never the worker's and never a
//! resumed one, started in a sandbox of its own ([`Sandbox::start_reviewer`])
//! through [`crate::session::sandboxed_launcher`] by [`open_on`]. It is sent the
//! read's prompt, its events go to the read's sink, and its last reply is read
//! with [`read_review`]. A second look is one more read, so one more session
//! (`docs/design/review.md`, Reviewers).
//!
//! The session gets a [`Judge`] for the worktree that lets it write the tests
//! folder when the project has a Test command, and no folder otherwise. Its
//! shell is refused whole, since the judge has no project to read a command
//! against. It has no kelpie tools, no lease and no instructions file: its
//! prompt carries everything it is told.
//!
//! Kelpie ships no harness (ADR 0006): like a worker's, a read's session is built
//! by the caller's [`Open`], which holds the logins and the installed bridges,
//! and [`open_on`] is the one for a plain harness program.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;

use kelpie_protocol::StopReason;

use super::{Finding, Read, Reviewer, ReviewerError, read_review};
use crate::agents::Binding;
use crate::permission::Judge;
use crate::sandbox::{self, Reads, Sandbox, SandboxError};
use crate::session::log::Log;
use crate::session::{Choice, Harness, Session, SessionEvent, sandboxed_launcher};
use crate::settings;
use crate::tree::ProcessTree;
use crate::worker::Paths;
use crate::worktree::{self, WorktreeError};

/// What a read's session is built from
pub struct Opening<'a> {
    /// What the reviewer runs on, each value with the layer that set it
    pub binding: &'a Binding,

    /// The model and effort to ask the harness for, from the binding
    pub choice: Choice,

    /// The worktree, which is the harness's working folder
    pub worktree: &'a Path,

    /// The reviewer's state folder, which holds its harness's config folder
    pub state_dir: &'a Path,

    /// Start the reviewer's sandbox for `harness`, as its policy has it
    pub sandbox: &'a dyn Fn(&Harness) -> Result<Sandbox, SandboxError>,
}

// The sandbox is a closure, which no derive can print
impl std::fmt::Debug for Opening<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opening")
            .field("binding", &self.binding)
            .field("choice", &self.choice)
            .field("worktree", &self.worktree)
            .field("state_dir", &self.state_dir)
            .finish_non_exhaustive()
    }
}

/// Builds a read's session on the harness its binding names, inside the sandbox
/// [`Opening::sandbox`] starts. The text of an error is for the maintainer.
pub type Open = dyn Fn(&Opening<'_>) -> Result<Session, String> + Send + Sync;

/// The opener for `harness`, a program kelpie starts through `tree` inside the
/// reviewer's sandbox
///
/// Each session it builds is new: it resumes nothing.
pub fn open_on(tree: Arc<ProcessTree>, harness: Harness) -> Box<Open> {
    Box::new(move |opening| {
        let sandbox = (opening.sandbox)(&harness).map_err(|error| error.to_string())?;
        let launch = sandboxed_launcher(
            Arc::clone(&tree),
            harness.clone(),
            sandbox,
            opening.worktree.to_owned(),
        );
        Ok(Session::with_launcher(
            launch,
            harness.clone(),
            opening.worktree.to_owned(),
            None,
            opening.choice.clone(),
        ))
    })
}

/// The project a reviewer reads in, and what its sandbox is told of it
#[derive(Debug, Clone)]
pub struct ReviewProject {
    /// The project's checkout
    pub folder: PathBuf,

    /// The checkout's git dir, whose objects each worktree's clone borrows
    pub git_common_dir: PathBuf,

    /// Domains the reviewer's sandbox lets through, besides its harness's model
    /// hosts (`network-allowlist`)
    pub network_allowlist: Vec<String>,

    /// How far the reviewer reads (`read-anywhere`)
    pub reads: Reads,
}

/// The name of the tool a harness writes a file with, which the reviewer's prompt
/// names for a failing test ([`super::TestsAsked::tool`])
#[must_use]
pub fn write_tool(harness: Option<settings::Harness>) -> &'static str {
    match harness {
        // parity: harness.reviewer
        Some(settings::Harness::Goose) => "text_editor",
        // The tool Claude Code writes with, and the name a harness that names no
        // other is asked for
        _ => "Write",
    }
}

/// The [`Reviewer`] of one ticket on one binding, which reads in fresh ACP sessions
pub struct SessionReviewer {
    paths: Paths,
    ticket: String,
    binding: Binding,
    project: ReviewProject,
    open: Box<Open>,
}

// The opener is a closure, which no derive can print
impl std::fmt::Debug for SessionReviewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionReviewer")
            .field("ticket", &self.ticket)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl SessionReviewer {
    /// A reviewer of `ticket`, whose reads run on `binding` and whose sessions `open` builds
    ///
    /// Each read keeps its log in the ticket's review state folder
    /// ([`Paths::review_log_file`]), numbered from 1 in the order the reads begin,
    /// and a read never writes a log an earlier read kept.
    #[must_use]
    pub fn new(
        paths: Paths,
        ticket: impl Into<String>,
        binding: Binding,
        project: ReviewProject,
        open: Box<Open>,
    ) -> Self {
        Self {
            paths,
            ticket: ticket.into(),
            binding,
            project,
            open,
        }
    }

    /// The log of the next read: the first numbered file nothing has written
    fn next_log(&self) -> Result<Log, ReviewerError> {
        let round = (1..)
            .find(|round| !self.paths.review_log_file(&self.ticket, *round).exists())
            .unwrap_or(u32::MAX);
        Log::open(self.paths.review_log_file(&self.ticket, round))
            .map_err(|error| ReviewerError::Start(format!("the read's log: {error}")))
    }

    /// The session of a read, with the judge that refuses it every shell and every write outside `read.tests`
    fn session(&self, read: &Read<'_>) -> Result<Session, ReviewerError> {
        let state_dir = self.paths.review_state_dir(&self.ticket);
        if let Some(tests) = read.tests {
            fs::create_dir_all(tests)
                .map_err(|error| ReviewerError::Start(format!("the tests folder: {error}")))?;
        }
        let sessions_root = self.paths.sessions_root();
        let reach = sandbox::Project {
            folder: &self.project.folder,
            network_allowlist: &self.project.network_allowlist,
            reads: self.project.reads,
            sessions_root: &sessions_root,
        };
        let folders = sandbox::Reviewer {
            worktree: read.worktree,
            git_common_dir: &self.project.git_common_dir,
            state_dir: &state_dir,
            tests: read.tests,
            kelpie_home: &self.paths.data_dir,
        };
        let start = |harness: &Harness| Sandbox::start_reviewer(harness, &reach, &folders);
        let opening = Opening {
            binding: &self.binding,
            choice: Choice {
                model: self.binding.model.value.clone(),
                effort: self.binding.effort.value.clone(),
            },
            worktree: read.worktree,
            state_dir: &state_dir,
            sandbox: &start,
        };
        let session = (self.open)(&opening).map_err(ReviewerError::Start)?;
        // No folder but the tests folder, and no project for the judge to read a
        // shell command against, so every command is refused
        let judge = Judge::new(
            read.worktree,
            read.tests.map(Path::to_owned).into_iter().collect(),
        );
        Ok(session.with_judge(judge).with_log(self.next_log()?))
    }
}

impl Reviewer for SessionReviewer {
    fn read(&self, read: &Read<'_>) -> Result<Vec<Finding>, ReviewerError> {
        let session = self.session(read)?;
        let (events, heard) = mpsc::channel();
        // The session's thread holds the sender, which hangs up when that thread
        // ends: the read's sink runs here, where it may borrow
        session.send(read.prompt, move |event| {
            let _ = events.send(event);
        });
        let mut reply = String::new();
        let mut failed = None;
        let ended = loop {
            let Ok(event) = heard.recv() else {
                break None;
            };
            (read.sink)(event.clone());
            match event {
                SessionEvent::Text { delta } => reply.push_str(&delta),
                // The reply is the text after the last tool call
                SessionEvent::ToolStep { .. } => reply.clear(),
                SessionEvent::TurnEnded { stop_reason, .. } => break Some(stop_reason),
                SessionEvent::Failed { message } => failed = Some(message),
                _ => {}
            }
        };
        session.stop();
        match ended {
            Some(StopReason::EndTurn) => read_review(&reply).map_err(ReviewerError::Unreadable),
            Some(StopReason::MaxTokens) => Err(ReviewerError::CutOff(
                "the model stopped at its token limit".to_owned(),
            )),
            Some(StopReason::MaxTurnRequests) => Err(ReviewerError::CutOff(
                "the model stopped at its limit of requests in one turn".to_owned(),
            )),
            Some(StopReason::Cancelled) => Err(ReviewerError::Stopped),
            Some(StopReason::Refusal) => Err(ReviewerError::Failed(
                "the model refused to answer".to_owned(),
            )),
            Some(_) => Err(ReviewerError::Failed(
                "the turn ended for a reason kelpie does not know".to_owned(),
            )),
            None => {
                Err(ReviewerError::Failed(failed.unwrap_or_else(|| {
                    "the session ended with no reply".to_owned()
                })))
            }
        }
    }
}

/// The change `worktree` holds against `base`, as `git diff base...HEAD` shows it
///
/// Read through the worktree's own git dirs, named and checked against `repo`'s
/// ([`worktree::trusted_command`]), so a `.git` file the worker pointed at
/// another repo is never followed. No external diff driver and no text
/// conversion run: both are settings a repo's config names, and a command in
/// a config is a command run.
///
/// # Errors
///
/// [`WorktreeError`]: the worktree is not `repo`'s, or git failed.
pub fn diff_against(
    tree: &ProcessTree,
    repo: &Path,
    worktree: &Path,
    base: &str,
) -> Result<String, WorktreeError> {
    let mut git = worktree::trusted_command(tree, repo, worktree)?;
    let range = format!("{base}...HEAD");
    git.args(["diff", "--no-ext-diff", "--no-textconv", "--no-color"])
        .arg(&range)
        .arg("--");
    let output = tree
        .output(&mut git)
        .map_err(|error| WorktreeError::Spawn(error.to_string()))?;
    if !output.status.success() {
        return Err(WorktreeError::Git {
            args: format!("diff {range}"),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
