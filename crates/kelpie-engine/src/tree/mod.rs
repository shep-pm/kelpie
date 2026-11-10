//! The process tree: the one way the engine starts a process
//!
//! [`ProcessTree::spawn`] puts a child in a group of its own, marks it with the
//! run's id and notes its leader in the run file. [`ProcessTree::output`] runs
//! a command to its end on that same spawn and hands back what it printed,
//! which is how a short-lived `git`, `openssl` or `curl` call reaches the tree.
//! [`ProcessTree::stop_all`] ends every child and then whatever else still
//! carries the mark, which catches a grandchild that left its group.
//! [`sweep_leftovers`] does the same for a run that died without stopping, and
//! belongs before the first spawn of the next one. [`sweep_record`] is that
//! sweep on a record already read, which is what a reaper holds.
//!
//! [`ProcessTree::with_reaper`] starts a reaper first, so a run that dies
//! without quitting still has something that stops what it started.
//!
//! - The run file ([`run_file`]) lists live groups only, each with its leader's
//!   start time, so a pid the OS has reused is never mistaken for a group.
//! - On Linux the process becomes a child subreaper, so orphans come back to
//!   it. The reaper ([`reaper`]) is what stops them when this process cannot.
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
use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use shep_tree::{Signal, SweepReport, Tree};

use crate::run::{ENV, RunId};

pub mod reaper;
pub mod run_file;
mod sweep;

pub use reaper::Reaper;
pub use sweep::{sweep_leftovers, sweep_marked, sweep_record};

use run_file::{Group, RunFile};

/// How often [`Child::wait`] looks at whether its child has exited
const POLL: Duration = Duration::from_millis(10);

/// Starts and stops every process of one run
#[derive(Debug)]
pub struct ProcessTree {
    shared: Arc<Shared>,
    /// The reaper [`ProcessTree::with_reaper`] started, holding the pipe whose
    /// close tells it to sweep. `None` from [`ProcessTree::new`].
    reaper: Option<Reaper>,
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

/// The process itself, behind a lock of its own so stopping it holds nothing else
///
/// A stop waits out its grace, and the state's lock is every caller's.
#[derive(Debug)]
struct Proc {
    tree: Tree,
    child: std::process::Child,
}

#[derive(Debug)]
struct Entry {
    proc: Arc<Mutex<Proc>>,
    /// The leader's pid, which is also its group's id
    pid: u32,
    /// The leader's start, `None` when it exited before it could be read
    started: Option<u64>,
    exited: Option<ExitStatus>,
    /// The [`Child`] handle is gone, so nobody will ask for the status
    released: bool,
}

/// Lock `mutex`, whether or not a thread panicked holding it
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Entry {
    /// How the child has exited, `None` while it runs or while a stop holds it
    ///
    /// A stop in progress records the status itself when it ends.
    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        match self.proc.try_lock() {
            Ok(mut proc) => proc.child.try_wait(),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().child.try_wait()
            }
            Err(std::sync::TryLockError::WouldBlock) => Ok(None),
        }
    }
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
                    pid: entry.pid,
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
                entry.exited = entry.try_wait().ok().flatten();
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
            reaper: None,
        })
    }

    /// Start a tree with its reaper already running
    ///
    /// The reaper at `reaper` is the first process of the run, before any
    /// [`ProcessTree::spawn`] can start another. It is not a child of the tree:
    /// [`ProcessTree::stop_all`] leaves it running, and dropping the
    /// `ProcessTree` closes its pipe, which is the sweep a quit does not need
    /// and a crash does.
    ///
    /// # Errors
    ///
    /// As [`ProcessTree::new`], or the reaper could not be started.
    pub fn with_reaper(reaper: &Path, run: RunId, run_file: PathBuf) -> io::Result<Self> {
        let mut tree = Self::new(run, run_file)?;
        tree.reaper = Some(Reaper::start(
            reaper,
            &tree.shared.run,
            &tree.shared.run_file,
        )?);
        Ok(tree)
    }

    /// The reaper's pid, `None` for a tree started by [`ProcessTree::new`]
    #[must_use]
    pub fn reaper_id(&self) -> Option<u32> {
        self.reaper.as_ref().map(Reaper::id)
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
                proc: Arc::new(Mutex::new(Proc { tree, child })),
                pid,
                started,
                exited: None,
                released: false,
            },
        );
        if let Err(error) = self.shared.write(&state) {
            if let Some(entry) = state.children.remove(&slot) {
                let mut proc = locked(&entry.proc);
                let Proc { tree, child } = &mut *proc;
                let _ = tree.stop(child, Signal::Kill, Duration::ZERO);
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

    /// Run `command` to its end, and return what it printed and how it exited
    ///
    /// stdout and stderr are piped, each read on a thread of its own, so a
    /// command that fills one pipe is never left blocked while the other one
    /// waits to be read. stdin is closed at once, so a command that reads it
    /// sees end of file instead of waiting on the app's own input. The child is
    /// in the tree while it runs, so [`ProcessTree::stop_all`] can end it
    /// mid-run.
    ///
    /// # Errors
    ///
    /// As [`ProcessTree::spawn`], or a pipe cannot be read.
    pub fn output(&self, command: &mut Command) -> io::Result<Output> {
        self.run(command, None)
    }

    /// As [`ProcessTree::output`], with `input` on the command's stdin
    ///
    /// stdin is piped, `input` is written to it, and it is then closed, so a
    /// command that reads its input to the end sees end of file and exits. A
    /// command that exits before reading any of it is not an error here: the
    /// broken pipe is dropped, and the exit status is what the caller is given.
    ///
    /// # Errors
    ///
    /// As [`ProcessTree::output`], or `input` cannot be written.
    pub fn output_with_input(&self, command: &mut Command, input: &[u8]) -> io::Result<Output> {
        self.run(command, Some(input))
    }

    /// Run `command` to its end, with `input` on stdin when it is `Some`
    fn run(&self, command: &mut Command, input: Option<&[u8]>) -> io::Result<Output> {
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(match input {
                Some(_) => Stdio::piped(),
                None => Stdio::null(),
            });
        let mut child = self.spawn(command)?;
        let (mut stdin, mut out, mut err) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take());
        std::thread::scope(|scope| {
            let reading_out = scope.spawn(move || read_pipe(&mut out));
            let reading_err = scope.spawn(move || read_pipe(&mut err));
            let writing = input.map(|input| scope.spawn(move || feed(&mut stdin, input)));
            let status = child.wait()?;
            let stdout = reading_out
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
            let stderr = reading_err
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
            if let Some(writing) = writing {
                writing
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
            }
            Ok(Output {
                status,
                stdout,
                stderr,
            })
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
                        let proc = Arc::clone(&entry.proc);
                        let mut proc = locked(&proc);
                        let Proc { tree, child } = &mut *proc;
                        entry.exited = Some(tree.stop(child, Signal::Term, grace)?);
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

/// Read a pipe to the end; one the command never opened reads as empty
fn read_pipe<T: Read>(pipe: &mut Option<T>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(pipe) = pipe {
        pipe.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}

/// Write `input` to stdin and close it, so a reader sees end of file
///
/// A command that exits before reading its input fails the write with a broken
/// pipe. Its exit status is the failure worth reporting, so that one is not an
/// error here.
fn feed(stdin: &mut Option<ChildStdin>, input: &[u8]) -> io::Result<()> {
    let Some(mut stdin) = stdin.take() else {
        return Ok(());
    };
    match stdin.write_all(input) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
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
            entry.exited = entry.try_wait()?;
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

    /// Stop this child's tree, and return how it exited
    ///
    /// The same ladder as [`ProcessTree::stop_all`] for one child: `SIGTERM` to
    /// the group, then the kill when `grace` runs out. The child stays in the
    /// tree, so [`ProcessTree::stop_all`] still counts it, and the run file is
    /// rewritten without it. A child already known to have exited is not
    /// signalled again. The tree is free to every other caller while this waits
    /// out `grace`.
    ///
    /// # Errors
    ///
    /// The child is no longer in the tree, or the OS could not stop its group.
    pub fn stop(&mut self, grace: Duration) -> io::Result<ExitStatus> {
        let proc = {
            let state = self.shared.lock();
            let Some(entry) = state.children.get(&self.slot) else {
                return Err(io::Error::other("the child is no longer in the tree"));
            };
            if let Some(status) = entry.exited {
                return Ok(status);
            }
            Arc::clone(&entry.proc)
        };
        // The tree's lock is not held while the ladder waits out `grace`, so
        // every other child keeps starting, asking and stopping meanwhile
        let status = {
            let mut proc = locked(&proc);
            let Proc { tree, child } = &mut *proc;
            tree.stop(child, Signal::Term, grace)?
        };
        let mut state = self.shared.lock();
        if let Some(entry) = state.children.get_mut(&self.slot) {
            entry.exited = Some(status);
            let _ = self.shared.write(&state);
        }
        Ok(status)
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
