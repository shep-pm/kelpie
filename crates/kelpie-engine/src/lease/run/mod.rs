//! `kelpie lease run <name> -- <command>`: one command run under a counted lease
//!
//! The command's process tree inherits the connection, so the lease stays held
//! until the last of it exits, however this process ends. When the command ends,
//! this process shuts the connection for every holder. A kelpie that is back is
//! asked again: a waiting run afresh, a running one to be counted
//! ([`crate::lease`]).
//!
//! A command that runs past its bound, counted from the grant and not from the
//! wait in the queue, is ended with everything it started, so one hung test
//! binary does not hold a place that others queue for.
//!
//! Lifted from SK's `core/lease/cli/counted_run.rs`, and the `exit_number` and
//! `listen` of `core/lease/cli/mod.rs`, at `1dc51c040`. The command runs through
//! the process tree (ADR 0001), carrying the run this process inherited, so
//! kelpie's sweep finds it wherever it is. The book leases, the e2e host and the
//! `take`/`return`/`status` commands stay in SK.

use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

use crate::run::RunId;
use crate::tools::Door;
use crate::tree::ProcessTree;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use client::run;

/// The subcommand that makes the binary a leased run
const LEASE: &str = "lease";

/// What a run exits with when it could not have the lease, as `EX_TEMPFAIL`
pub const NO_LEASE: u8 = 75;

/// What a run exits with when its command could not start, as a shell does
pub const NOT_RUN: u8 = 127;

/// What a run exits with when its command ran past its bound and was ended, as GNU `timeout` does
pub const TIMED_OUT: u8 = 124;

/// How long a command holds its place before the run ends it: a full suite takes
/// minutes, and the hang SK bounded this for held a place for 28
pub const RUN_LIMIT: Duration = Duration::from_secs(20 * 60);

/// How long a command has to end after the request to end before it is killed
const KILL_AFTER: Duration = Duration::from_secs(5);

/// How often a run whose kelpie went away knocks again; a restart takes seconds
pub const REJOIN_EVERY: Duration = Duration::from_secs(1);

/// How many knocks before a run stops asking again, waiting or running
pub const REJOIN_TRIES: u32 = 60;

/// How long a run keeps asking for a kelpie that is gone
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Patience {
    /// How many knocks before it stops
    pub tries: u32,
    /// How long between knocks
    pub every: Duration,
}

/// What a run has: [`REJOIN_TRIES`] knocks, one every [`REJOIN_EVERY`]
pub const PATIENCE: Patience = Patience {
    tries: REJOIN_TRIES,
    every: REJOIN_EVERY,
};

/// What the run's own lines are prefixed with, which a worker reads
fn say(message: &str) {
    eprintln!("kelpie lease: {message}");
}

fn failed(message: &str, code: u8) -> u8 {
    say(message);
    code
}

/// What the first failed knock says, so a worker reading it knows how long the
/// run holds out before it exits without the lease
fn asking_again(lost: &str, patience: Patience) -> String {
    format!(
        "{lost}: asking again every {:?}, up to {} times while kelpie comes back, then exiting {NO_LEASE}",
        patience.every, patience.tries
    )
}

/// The shell's number for how a command ended: its code, or 128 plus its signal
fn exit_number(status: ExitStatus) -> u8 {
    #[cfg(unix)]
    let number = {
        use std::os::unix::process::ExitStatusExt as _;
        match status.code() {
            Some(code) => code,
            None => 128 + status.signal().unwrap_or(1),
        }
    };
    #[cfg(not(unix))]
    let number = status.code().unwrap_or(1);
    u8::try_from(number).unwrap_or(1)
}

/// Takes over the process when the binary was started as `kelpie lease run`
///
/// A host calls this in `main` after the sandbox's and the tools' own
/// `run_from_args`, and before anything else of its own. It runs the command and
/// exits with its number when the arguments are `lease run <name> -- <command>`,
/// and returns at once for any other run.
///
/// The socket is the one the session's environment names
/// ([`SOCKET_VAR`](crate::lease::SOCKET_VAR)). With none there, the run exits
/// [`NO_LEASE`] and says to go on without the lease.
pub fn run_from_args() {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let Some((first, rest)) = args.split_first() else {
        return;
    };
    if first != LEASE {
        return;
    }
    std::process::exit(i32::from(dispatch(rest)));
}

/// What `kelpie lease <rest>` does, as the number to exit with
fn dispatch(rest: &[OsString]) -> u8 {
    let Some((word, rest)) = rest.split_first() else {
        return usage();
    };
    if word != "run" {
        return usage();
    }
    let Some((name, rest)) = rest.split_first() else {
        return usage();
    };
    let Some((separator, command)) = rest.split_first() else {
        return usage();
    };
    if separator != "--" || command.is_empty() {
        return usage();
    }
    run_at(&name.to_string_lossy(), command)
}

/// Says what `kelpie lease` takes, and exits 2 as a shell does for bad arguments
fn usage() -> u8 {
    eprintln!(
        "usage: kelpie lease run <name> -- <command> [args...]\n\
         <name> is a lease a few commands hold at once, `cargo-test` for a worker's tests. \
         The run waits its turn, runs the command, and gives the lease back when it ends."
    );
    2
}

/// Ask for the lease `name`, run `command` under it, and return the number to exit with
///
/// The command runs through a process tree of its own, under the run this process
/// inherited, so it carries the same mark and kelpie's sweep finds it (ADR 0001).
/// Every failure is said and turned into the number to exit with.
fn run_at(name: &str, command: &[OsString]) -> u8 {
    let Some(text) = env::var_os(crate::lease::SOCKET_VAR).filter(|text| !text.is_empty()) else {
        return failed(
            &format!(
                "{} is not set, so there is no kelpie to ask: run the tests without the lease, and say so in your final message",
                crate::lease::SOCKET_VAR
            ),
            NO_LEASE,
        );
    };
    let Ok(door) = Door::parse(&text.to_string_lossy()) else {
        return failed(&format!("{text:?} is no lease socket"), NO_LEASE);
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (name, command);
        return failed(
            "kelpie's leases have no socket on this platform yet (ADR 0005)",
            NO_LEASE,
        );
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let Some(mark) = inherited_run() else {
            return failed("cannot draw a run id for the command", NO_LEASE);
        };
        let file = run_file();
        let tree = match ProcessTree::new(mark, file.clone()) {
            Ok(tree) => tree,
            Err(error) => {
                return failed(
                    &format!("the process tree could not start: {error}"),
                    NO_LEASE,
                );
            }
        };
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                return failed(
                    &format!("cannot start the async runtime: {error}"),
                    NO_LEASE,
                );
            }
        };
        let words: Vec<&OsStr> = command.iter().map(OsString::as_os_str).collect();
        let code = runtime.block_on(run(&door, name, &words, RUN_LIMIT, PATIENCE, &tree));
        // Whatever the command left running goes with this run (ADR 0001)
        let _ = tree.stop_all(KILL_AFTER);
        let _ = std::fs::remove_file(&file);
        code
    }
}

/// The run this process inherited, or a new one when it carries none
///
/// A worker's `kelpie lease run` is started by its harness, which carries the run
/// kelpie started it under, so the command it runs carries the same mark and
/// kelpie's sweep finds it (ADR 0001).
fn inherited_run() -> Option<RunId> {
    match env::var_os(crate::run::ENV) {
        Some(text) => text
            .to_string_lossy()
            .parse()
            .ok()
            .or_else(|| RunId::new().ok()),
        None => RunId::new().ok(),
    }
}

/// Where the lease run's own tree notes its groups
///
/// The run's mark is what finds the command again wherever it runs, so this file
/// is only what the tree keeps while the run lives: it goes under the temporary
/// folder with this process's id, and is removed when the run ends.
fn run_file() -> PathBuf {
    env::temp_dir().join(format!("kelpie-lease-{}.json", std::process::id()))
}

/// What ends a leased run from outside, and what its number is
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caught {
    /// SIGINT, or Ctrl-C
    Interrupt,
    /// SIGTERM, or Ctrl-Break
    Terminate,
    /// SIGHUP, or the console closing
    Hangup,
}

impl Caught {
    /// The shell's number for dying of it: 128 plus the signal's number
    #[must_use]
    pub fn number(self) -> u8 {
        match self {
            Self::Hangup => 129,
            Self::Interrupt => 130,
            Self::Terminate => 143,
        }
    }

    /// What the maintainer calls it
    #[must_use]
    pub fn name(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Self::Interrupt, _) => "Ctrl-C",
            (Self::Terminate, false) => "SIGTERM",
            (Self::Terminate, true) => "Ctrl-Break",
            (Self::Hangup, false) => "SIGHUP",
            (Self::Hangup, true) => "the console closing",
        }
    }
}

/// Listening for the signals that should end a leased run
///
/// Listening replaces each one's default of ending the run at once, so the
/// command's processes are ended and the lease given back first. A kind not
/// listened for keeps its default.
#[derive(Debug)]
struct Signals {
    #[cfg(unix)]
    interrupt: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    terminate: Option<tokio::signal::unix::Signal>,
    #[cfg(unix)]
    hangup: Option<tokio::signal::unix::Signal>,
}

impl Signals {
    /// Starts listening for each signal that ends a run
    ///
    /// # Errors
    ///
    /// The system refused to listen for one of them.
    #[cfg(unix)]
    fn listen() -> io::Result<Self> {
        use tokio::signal::unix::{SignalKind, signal};
        Ok(Self {
            interrupt: Some(signal(SignalKind::interrupt())?),
            terminate: Some(signal(SignalKind::terminate())?),
            hangup: Some(signal(SignalKind::hangup())?),
        })
    }

    /// Starts listening, on a platform with no signals to listen for
    #[cfg(not(unix))]
    fn listen() -> io::Result<Self> {
        Ok(Self {})
    }

    /// The next one caught, never returning if none is listened for
    #[cfg(unix)]
    async fn recv(&mut self) -> Caught {
        tokio::select! {
            () = next(&mut self.interrupt) => Caught::Interrupt,
            () = next(&mut self.terminate) => Caught::Terminate,
            () = next(&mut self.hangup) => Caught::Hangup,
        }
    }

    /// The next one caught, never returning where no signal is caught
    #[cfg(not(unix))]
    async fn recv(&mut self) -> Caught {
        std::future::pending::<Caught>().await
    }

    /// One that has already come, if one has
    ///
    /// The system may have handed the run a signal its listeners have not been
    /// told of yet, since tokio flags it from its driver, not from the handler.
    /// One yield lets the driver catch up before they are asked.
    async fn already(&mut self) -> Option<Caught> {
        tokio::task::yield_now().await;
        tokio::select! {
            biased;
            caught = self.recv() => Some(caught),
            () = std::future::ready(()) => None,
        }
    }
}

#[cfg(unix)]
async fn next(source: &mut Option<tokio::signal::unix::Signal>) {
    match source {
        Some(source) => while source.recv().await.is_none() {},
        None => std::future::pending().await,
    }
}

/// The command's own text, for the lease status
fn shown(command: &[&OsStr]) -> String {
    command
        .iter()
        .map(|word| word.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod client;

#[cfg(test)]
mod tests;
