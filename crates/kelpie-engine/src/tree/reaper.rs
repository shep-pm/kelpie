//! The reaper: the process that stops the tree when kelpie dies without quitting
//!
//! A crash or a `SIGKILL` runs no exit hook, so the stopping cannot live in
//! kelpie (ADR 0001). [`Reaper::start`] spawns the reaper binary and holds the
//! write end of its standard input open: the reaper blocks reading that pipe,
//! and the read ending is the news that kelpie is gone.
//!
//! The reaper is not part of the tree. It carries no [`ENV`], is not listed in
//! the run file, and [`ProcessTree::stop_all`] leaves it running so it can do
//! its own sweep. A quit runs [`ProcessTree::stop_all`] first and then closes
//! the pipe, so the reaper sweeps and finds nothing left.

use std::io;
use std::path::Path;
use std::process::{ChildStdin, Command, ExitStatus, Stdio};

use shep_tree::Tree;

use crate::run::{ENV, RunId};

// Link only: `ProcessTree` is in the parent module, and rustdoc needs it in
// scope to resolve the `stop_all` links above and below
#[cfg(doc)]
use super::ProcessTree;

/// The reaper process, and the pipe whose close tells it to sweep
///
/// Dropping it closes the pipe and leaves the reaper running to finish its
/// sweep.
#[derive(Debug)]
pub struct Reaper {
    child: std::process::Child,
    /// The write end of the pipe the reaper blocks reading
    pipe: ChildStdin,
}

impl Reaper {
    /// Start the reaper at `path` for `run`, and hold its pipe open
    ///
    /// Where the binary lives is the caller's business: the app passes its
    /// sidecar path, a test passes `CARGO_BIN_EXE_kelpie-reaper`. The reaper
    /// starts through [`Tree::spawn`], so it leads a group of its own and a
    /// group signal aimed at this process misses it, and without [`ENV`], so
    /// no sweep of this run matches it.
    ///
    /// # Errors
    ///
    /// The reaper could not be started, or its standard input could not be
    /// piped; a reaper started without a pipe is killed before the error.
    pub fn start(path: &Path, run: &RunId, run_file: &Path) -> io::Result<Self> {
        let mut command = Command::new(path);
        command
            .arg("--run")
            .arg(run.to_string())
            .arg("--run-file")
            .arg(run_file)
            .env_remove(ENV)
            .stdin(Stdio::piped());
        // Held for nothing: the reaper starts no child of its own, and its
        // group is its own once spawned, on Windows as much as on unix
        let (_tree, mut child) = Tree::spawn(&mut command)?;
        let Some(pipe) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("the reaper has no piped stdin"));
        };
        Ok(Self { child, pipe })
    }

    /// The reaper's pid
    #[must_use]
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Close the pipe, and wait for the reaper to sweep and exit
    ///
    /// The normal quit: [`ProcessTree::stop_all`] first, then this. The reaper
    /// reads the end of the pipe, sweeps the run file, and finds nothing left.
    ///
    /// # Errors
    ///
    /// The OS could not wait on the reaper.
    pub fn quit(self) -> io::Result<ExitStatus> {
        let Self { mut child, pipe } = self;
        drop(pipe);
        child.wait()
    }
}
