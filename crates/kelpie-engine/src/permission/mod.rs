//! Permission: each tool call a worker asks to make is judged before it runs
//!
//! A harness asks over ACP before it runs a tool, and kelpie answers in process
//! (`docs/design/workers.md`, The worker's policy). Where SK spawned a hook per
//! call, the session's handler reads the request into a [`Call`] and asks a
//! [`Judge`], which allows it or refuses with a reason the refused step keeps.
//! Every harness asks the same way, but each names the file it writes and the
//! command it runs in its own words, so each adapter reads its own tool input
//! ([`crate::harness`], `read_call`).
//!
//! A harness only asks when it is not set to stop asking, so a judged session
//! runs the harness in its prompting mode ([`crate::session::Harness::prompting`]).
//!
//! # Quick start
//!
//! ```
//! use std::path::PathBuf;
//!
//! use kelpie_engine::confine::Verdict;
//! use kelpie_engine::permission::{Call, Judge};
//!
//! let worktree = PathBuf::from("/work/ticket-1");
//! let judge = Judge::new(&worktree, vec![worktree.clone()]);
//!
//! let write = Call::Write { path: "src/lib.rs".into() };
//! assert_eq!(judge.judge(&write), Verdict::Allow);
//!
//! let shell = Call::Shell { command: "ls".to_owned() };
//! assert!(matches!(judge.judge(&shell), Verdict::Refuse(_)));
//! ```

use std::fmt;
use std::path::{Path, PathBuf};

use agent_client_protocol::v1::schema::{ToolCallUpdate, ToolKind};

use crate::confine::{self, Verdict};

/// What a tool call does, as far as the judge needs to know
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// The call writes one file
    ///
    /// An empty `path` is a write the adapter could not read the file of, which
    /// the judge refuses.
    Write {
        /// The file, absolute or from the session's folder
        path: PathBuf,
    },

    /// The call runs a shell command
    Shell {
        /// The command line, as the harness gives it
        command: String,
    },

    /// Anything else, such as a read or a search, which writes no file
    Other,
}

/// How a harness's adapter reads a permission request's tool call into a [`Call`]
///
/// A function pointer of the adapter's own, so a [`crate::session::Harness`]
/// stays plain data.
#[derive(Clone, Copy)]
pub struct ReadCall(fn(&ToolCallUpdate) -> Call);

impl ReadCall {
    /// The reader `read`
    #[must_use]
    pub const fn new(read: fn(&ToolCallUpdate) -> Call) -> Self {
        Self(read)
    }

    /// The reader for a harness that names its tool input in no words of its own
    ///
    /// Reads what ACP itself says: the kind of the call, and the files it names
    /// as its locations.
    #[must_use]
    pub const fn standard() -> Self {
        Self(|call| read_acp(call, &[]))
    }

    /// What `tool_call` does
    #[must_use]
    pub fn read(&self, tool_call: &ToolCallUpdate) -> Call {
        (self.0)(tool_call)
    }
}

impl PartialEq for ReadCall {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::fn_addr_eq(self.0, other.0)
    }
}

impl Eq for ReadCall {}

impl fmt::Debug for ReadCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReadCall").finish_non_exhaustive()
    }
}

/// Reads a tool call by what ACP says of it, and by the fields of its raw input
/// that name a file
///
/// A call of kind edit, delete or move is a [`Call::Write`]: its file is the
/// first of `path_fields` its raw input holds, else its one location. A call
/// that names two files, or none, is a write of no file, which the judge
/// refuses. A call of kind execute is a [`Call::Shell`], with its raw input's
/// `command`. Every other kind is [`Call::Other`].
///
/// An adapter calls this with the fields its own harness names a file by.
#[must_use]
pub fn read_acp(tool_call: &ToolCallUpdate, path_fields: &[&str]) -> Call {
    let fields = &tool_call.fields;
    match fields.kind {
        Some(ToolKind::Edit | ToolKind::Delete | ToolKind::Move) => {
            let locations = fields.locations.as_deref().unwrap_or_default();
            let path = match locations {
                [] | [_] => path_fields
                    .iter()
                    .find_map(|field| {
                        fields
                            .raw_input
                            .as_ref()?
                            .get(field)?
                            .as_str()
                            .map(PathBuf::from)
                    })
                    .or_else(|| locations.first().map(|location| location.path.clone())),
                // One path to judge: a call that names more is not read
                _ => None,
            };
            Call::Write {
                path: path.unwrap_or_default(),
            }
        }
        Some(ToolKind::Execute) => Call::Shell {
            command: fields
                .raw_input
                .as_ref()
                .and_then(|input| input.get("command")?.as_str())
                .unwrap_or_default()
                .to_owned(),
        },
        _ => Call::Other,
    }
}

/// What judges a worker's tool calls: the folder it works in, and the folders it may write
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Judge {
    /// Where a relative path is read from
    cwd: PathBuf,
    /// The folders a write must stay in
    folders: Vec<PathBuf>,
}

impl Judge {
    /// A judge that lets `cwd`'s worker write inside `folders`
    ///
    /// `cwd` is the worktree, and a relative path in a call is read from it.
    #[must_use]
    pub fn new(cwd: impl AsRef<Path>, folders: Vec<PathBuf>) -> Self {
        Self {
            cwd: cwd.as_ref().to_owned(),
            folders,
        }
    }

    /// Whether `call` goes ahead, and if not, why
    #[must_use]
    pub fn judge(&self, call: &Call) -> Verdict {
        match call {
            Call::Write { path } if path.as_os_str().is_empty() => {
                Verdict::Refuse("kelpie cannot tell which one file this call writes".to_owned())
            }
            Call::Write { path } => {
                confine::judge(&self.cwd, std::slice::from_ref(path), &self.folders)
            }
            // `guard` reads a command and judges its git calls. Until its issue
            // lands in this handler, no command is run
            Call::Shell { .. } => Verdict::Refuse(
                "kelpie runs no shell command for a worker until guard judges it: \
                 guard is not yet part of this check"
                    .to_owned(),
            ),
            Call::Other => Verdict::Allow,
        }
    }
}

#[cfg(test)]
mod tests;
