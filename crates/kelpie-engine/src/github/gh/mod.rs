//! Calls to GitHub as the maintainer, one `gh` child per call
//!
//! Every read and the draft pull request go out as the login `gh` holds
//! (`docs/design/tickets.md`, GitHub). The login's token never leaves the `gh`
//! child: kelpie never reads it, and it is not in a call's arguments or
//! environment. [`Gh`] runs each call through the process tree (ADR 0001) with
//! `GH_DEBUG=api`, so a failure's stderr carries the response headers
//! a rate limit's [`Cooldown`] is read from. [`Runner`] is the child, which a
//! test scripts. [`GithubSource`] reads a repo's issues and open pull requests through a [`hold::Hold`]
//! over it, and [`GithubQueue`] reads where a pull request stands in its merge queue.
//!
//! [`Cooldown`]: super::Cooldown

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;

use crate::ticket::ForgeError;
use crate::tree::ProcessTree;
use crate::worker::login;

pub mod account;
pub mod conditional;
mod failed;
pub mod hold;
mod listing;
pub mod pulls;
mod pulls_read;
pub mod queue;
mod source;

pub use account::{RateBucket, RateLimits};
pub use pulls::GithubPulls;
pub use queue::GithubQueue;
pub use source::GithubSource;

#[cfg(test)]
mod conditional_tests;
#[cfg(test)]
mod hold_tests;
#[cfg(test)]
mod pulls_read_tests;
#[cfg(test)]
mod pulls_tests;
#[cfg(test)]
mod queue_tests;
#[cfg(test)]
mod source_tests;
#[cfg(test)]
mod tests;

/// The `gh` child of one call
pub trait Runner: Send + Sync + fmt::Debug {
    /// Runs `gh` with `args` to its end
    ///
    /// # Errors
    ///
    /// [`io::Error`] when `gh` cannot be started or its output cannot be read.
    fn run(&self, args: &[String]) -> io::Result<Output>;
}

/// The system's `gh`, started through the process tree
#[derive(Debug, Clone)]
struct Process {
    gh: PathBuf,
    tree: Arc<ProcessTree>,
}

impl Runner for Process {
    fn run(&self, args: &[String]) -> io::Result<Output> {
        self.tree.output(&mut command(&self.gh, args))
    }
}

// `gh` with `args`, printing the API's debug lines, which hold the headers a
// limit's answer carries
fn command(gh: &Path, args: &[String]) -> Command {
    let mut command = Command::new(gh);
    command.args(args).env("GH_DEBUG", "api");
    command
}

/// The calls kelpie makes to GitHub as the maintainer's own `gh` login
#[derive(Debug)]
pub struct Gh {
    runner: Box<dyn Runner>,
}

impl Gh {
    /// Calls through the `gh` at `path`, started through `tree`
    #[must_use]
    pub fn new(path: PathBuf, tree: &Arc<ProcessTree>) -> Self {
        Self::with_runner(Box::new(Process {
            gh: path,
            tree: Arc::clone(tree),
        }))
    }

    /// Calls through the `gh` on this process's `PATH`, or `None` when there is none
    #[must_use]
    pub fn on_path(tree: &Arc<ProcessTree>) -> Option<Self> {
        login::helper_on_path().map(|path| Self::new(path, tree))
    }

    /// Calls answered by `runner`
    #[must_use]
    pub fn with_runner(runner: Box<dyn Runner>) -> Self {
        Self { runner }
    }

    /// What `gh` printed on stdout for a call of `args`
    ///
    /// # Errors
    ///
    /// [`ForgeError::Spawn`] when `gh` cannot be started, and a [`ForgeError::Failed`]
    /// or [`ForgeError::Limited`] made of its stderr when it exits unsuccessfully.
    /// A `gh` with no login is a `Failed` carrying its own line, which says to
    /// run `gh auth login`.
    pub fn call(&self, args: &[&str]) -> Result<Vec<u8>, ForgeError> {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let output = self
            .runner
            .run(&args)
            .map_err(|e| ForgeError::Spawn(e.to_string()))?;
        if !output.status.success() {
            return Err(failed::failed(&String::from_utf8_lossy(&output.stderr)));
        }
        Ok(output.stdout)
    }
}

// What `gh` printed, as the text of an answer kelpie could not read
fn unreadable(stdout: &[u8]) -> ForgeError {
    ForgeError::Unreadable(String::from_utf8_lossy(stdout).into_owned())
}
