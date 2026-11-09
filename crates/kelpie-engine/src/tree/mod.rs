//! The process tree: the one way the engine starts a process
//!
//! [`ProcessTree::spawn`] puts a child in a group of its own, marks it with the
//! run's id and notes its leader in the run file. [`ProcessTree::stop_all`]
//! ends every child and then whatever else still carries the mark, which
//! catches a grandchild that left its group. [`sweep_leftovers`] does the same
//! for a run that died without stopping, and belongs before the first spawn
//! of the next one.
//!
//! - The run file ([`run_file`]) lists live groups only, each with its leader's
//!   start time, so a pid the OS has reused is never mistaken for a group.
//! - On Linux the process becomes a child subreaper, so orphans come back to
//!   it. Nothing reaps them yet; the reaper sidecar will.
//! - Windows has no job object here yet, so a child's group is not stopped.
//!
//! # Quick start
//!
//! ```no_run
//! use std::process::Command;
//! use std::time::Duration;
//!
//! use kelpie_engine::run::RunId;
//! use kelpie_engine::tree::{ProcessTree, sweep_leftovers};
//!
//! let run_file = std::path::PathBuf::from("run.json");
//! sweep_leftovers(&run_file)?;
//! let tree = ProcessTree::new(RunId::new()?, run_file)?;
//! let mut child = tree.spawn(Command::new("sleep").arg("60"))?;
//! tree.stop_all(Duration::from_secs(5))?;
//! assert!(!child.wait()?.success());
//! # Ok::<(), std::io::Error>(())
//! ```

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use shep_tree::{Signal, SweepReport, Tree};

use crate::run::{ENV, RunId};

pub mod run_file;
mod sweep;

pub use sweep::{sweep_leftovers, sweep_marked};

use run_file::{Group, RunFile};

/// How often [`Child::wait`] looks at whether its child has exited
const POLL: Duration = Duration::from_millis(10);

/// Starts and stops every process of one run
#[derive(Debug)]
pub struct ProcessTree {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    run: RunId,
    run_file: PathBuf,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// Set by `stop_all`: nothing may start after the tree was stopped
    closed: bool,
    next: u64,
    children: BTreeMap<u64, Entry>,
}

#[derive(Debug)]
struct Entry {
    tree: Tree,
    child: std::process::Child,
    /// The leader's start, `None` when it exited before it could be read
    started: Option<u64>,
    exited: Option<ExitStatus>,
    /// The [`Child`] handle is gone, so nobody will ask for the status
    released: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Rewrite the run file with the groups that are still alive
    fn write(&self, state: &State) -> io::Result<()> {
        let groups = state
            .children
            .values()
            .filter(|entry| entry.exited.is_none())
            .filter_map(|entry| {
                Some(Group {
                    pid: entry.child.id(),
                    started: entry.started?,
                })
            })
            .collect();
        RunFile {
            run: self.run.to_string(),
            groups,
        }
        .write(&self.run_file)
    }
}

impl State {
    /// Note every child that has exited, and forget those nobody will ask
    /// about. Returns whether a live child was found to have exited.
    fn note_exits(&mut self) -> bool {
        let mut found = false;
        for entry in self.children.values_mut() {
            if entry.exited.is_none() {
                entry.exited = entry.child.try_wait().ok().flatten();
                found |= entry.exited.is_some();
            }
        }
        self.children
            .retain(|_, entry| !(entry.released && entry.exited.is_some()));
        found
    }
}

impl ProcessTree {
    /// Start a tree for `run`, recording its groups in `run_file`
    ///
    /// Writes an empty run file, replacing a previous run's, so call
    /// [`sweep_leftovers`] on the path first. On Linux the process becomes a
    /// child subreaper.
    ///
    /// # Errors
    ///
    /// The run file cannot be written, or the subreaper flag cannot be set.
    pub fn new(run: RunId, run_file: PathBuf) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        rustix::process::set_child_subreaper(Some(rustix::process::getpid()))?;
        let shared = Shared {
            run,
            run_file,
            state: Mutex::new(State::default()),
        };
        shared.write(&State::default())?;
        Ok(Self {
            shared: Arc::new(shared),
        })
    }

    /// Start `command` in a group of its own, carrying this run's mark
    ///
    /// Sets `KELPIE_RUN` on `command`, replacing a value it had, and rewrites
    /// the run file. Pipes `command` asked for are on the returned [`Child`].
    ///
    /// # Errors
    ///
    /// The tree was stopped by [`ProcessTree::stop_all`], the child cannot be
    /// started, or the run file cannot be rewritten; in the last case the child
    /// is killed first.
    pub fn spawn(&self, command: &mut Command) -> io::Result<Child> {
        command.env(ENV, self.shared.run.to_string());
        // Held across the spawn, so `stop_all` cannot sweep between the two
        let mut state = self.shared.lock();
        if state.closed {
            return Err(io::Error::other("the process tree was stopped"));
        }
        let (tree, mut child) = Tree::spawn(command)?;
        let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let pid = child.id();
        let started = sweep::read(pid).map(|reading| reading.started);
        state.note_exits();
        let slot = state.next;
        state.next += 1;
        state.children.insert(
            slot,
            Entry {
                tree,
                child,
                started,
                exited: None,
                released: false,
            },
        );
        if let Err(error) = self.shared.write(&state) {
            if let Some(mut entry) = state.children.remove(&slot) {
                let _ = entry
                    .tree
                    .stop(&mut entry.child, Signal::Kill, Duration::ZERO);
            }
            return Err(error);
        }
        let (stdin, stdout, stderr) = pipes;
        Ok(Child {
            stdin,
            stdout,
            stderr,
            pid,
            slot,
            shared: Arc::clone(&self.shared),
        })
    }

    /// Stop every child and everything marked with this run
    ///
    /// Each live child's tree is signalled with `SIGTERM` at once and killed
    /// when `grace` runs out. Then [`sweep_marked`] ends what is left, such as a
    /// grandchild that left its group. A child that already exited is not
    /// signalled again. Afterwards [`ProcessTree::spawn`] refuses to start
    /// anything.
    ///
    /// # Errors
    ///
    /// The first child that could not be stopped, after the others were tried
    /// and the sweep ran.
    pub fn stop_all(&self, grace: Duration) -> io::Result<SweepReport> {
        let mut state = self.shared.lock();
        state.closed = true;
        state.note_exits();
        let results: Vec<io::Result<()>> = std::thread::scope(|scope| {
            let stops: Vec<_> = state
                .children
                .values_mut()
                .filter(|entry| entry.exited.is_none())
                .map(|entry| {
                    scope.spawn(move || {
                        entry.exited =
                            Some(entry.tree.stop(&mut entry.child, Signal::Term, grace)?);
                        Ok(())
                    })
                })
                .collect();
            stops
                .into_iter()
                .map(|stop| {
                    stop.join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        });
        let written = self.shared.write(&state);
        drop(state);
        let report = sweep_marked(&self.shared.run);
        results.into_iter().collect::<io::Result<()>>()?;
        written?;
        Ok(report)
    }
}

/// A child started by a [`ProcessTree`]
///
/// Dropping it leaves the child running; the tree still stops it.
#[derive(Debug)]
pub struct Child {
    /// The child's standard input, if it was piped
    pub stdin: Option<ChildStdin>,
    /// The child's standard output, if it was piped
    pub stdout: Option<ChildStdout>,
    /// The child's standard error, if it was piped
    pub stderr: Option<ChildStderr>,
    pid: u32,
    slot: u64,
    shared: Arc<Shared>,
}

impl Child {
    /// The child's pid, which is also its group's id
    #[must_use]
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// The child's exit status, or `None` while it runs
    ///
    /// The first call that sees the exit takes the child out of the run file.
    ///
    /// # Errors
    ///
    /// The OS cannot say whether the child has exited.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let mut state = self.shared.lock();
        let Some(entry) = state.children.get_mut(&self.slot) else {
            return Ok(None);
        };
        if entry.exited.is_none() {
            entry.exited = entry.child.try_wait()?;
            if entry.exited.is_some() {
                // A listed group that is gone is found gone at the next launch
                let _ = self.shared.write(&state);
            }
        }
        Ok(state.children[&self.slot].exited)
    }

    /// Wait for the child to exit
    ///
    /// Polls instead of blocking in the OS, so [`ProcessTree::stop_all`] can
    /// take the child meanwhile.
    ///
    /// # Errors
    ///
    /// The OS cannot say whether the child has exited.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        if let Some(entry) = state.children.get_mut(&self.slot) {
            entry.released = true;
        }
        if state.note_exits() {
            // A listed group that is gone is found gone at the next launch
            let _ = self.shared.write(&state);
        }
    }
}

// The tests start `sh` and `cat`, which Windows lacks
#[cfg(all(test, unix))]
mod tests;
