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

/// The run itself, on the platform's lease socket
///
/// macOS and Linux only, as every kelpie socket is (ADR 0005).
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod client {
    use std::ffi::OsStr;
    use std::io;
    use std::process::Command;
    use std::time::Duration;

    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
    use tokio::time::Instant;

    use super::{
        Caught, KILL_AFTER, NO_LEASE, NOT_RUN, Patience, Signals, TIMED_OUT, asking_again,
        exit_number, failed, say, shown,
    };
    use crate::lease::{Answer, Ask};
    use crate::tools::Door;
    use crate::tree::{Child, ProcessTree};

    /// How long a kelpie has to answer an ask; a live one answers at once
    const ANSWER_WITHIN: Duration = Duration::from_secs(5);

    /// How often a run looks at whether its command has ended
    const POLL: Duration = Duration::from_millis(50);

    /// The connection one command holds its place on: a Unix socket (macOS)
    #[cfg(target_os = "macos")]
    type Conn = tokio::net::UnixStream;
    /// The connection one command holds its place on: loopback TCP (Linux)
    #[cfg(target_os = "linux")]
    type Conn = tokio::net::TcpStream;

    /// Runs `command` once kelpie grants the lease `name` on `door`, and returns
    /// the number to exit with: the command's own, [`NO_LEASE`], [`NOT_RUN`],
    /// [`TIMED_OUT`] if it ran past `limit` from the grant, or a caught signal's
    pub async fn run(
        door: &Door,
        name: &str,
        command: &[&OsStr],
        limit: Duration,
        patience: Patience,
        tree: &ProcessTree,
    ) -> u8 {
        let Some((program, args)) = command.split_first() else {
            return NOT_RUN;
        };
        let mut signals = match Signals::listen() {
            Ok(signals) => signals,
            Err(error) => return failed(&format!("cannot listen for signals: {error}"), NO_LEASE),
        };
        let ask = Ask {
            take: name.to_owned(),
            pid: std::process::id(),
            what: shown(command),
            running: false,
        };
        let visit = match wait_for_turn(door, &ask, &mut signals, patience).await {
            Ok(Ok(visit)) => visit,
            Ok(Err(caught)) => return caught.number(),
            Err(why) => return failed(&unreached(&why), NO_LEASE),
        };
        let granted = Instant::now();
        // A signal before the start reached nobody else, so it ends the run.
        if let Some(caught) = signals.already().await {
            visit.give_back().await;
            return caught.number();
        }
        // The command inherits the connection, so the lease outlives this process
        // if it is killed while the command runs, and is let go when the last of
        // the command's processes closes it.
        if let Err(why) = visit.inheritable(true) {
            say(&format!(
                "{why}: the lease is held by this process alone, and ends if it is killed while the command runs"
            ));
        }
        let mut child = match started(tree, program, args) {
            Ok(child) => child,
            Err(error) => {
                visit.give_back().await;
                return failed(
                    &format!("cannot run {}: {error}", program.to_string_lossy()),
                    NOT_RUN,
                );
            }
        };
        let _ = visit.inheritable(false);
        let rejoin = Rejoin {
            door,
            ask: Ask {
                running: true,
                ..ask
            },
            patience,
        };
        tend(
            &mut child,
            Some(visit),
            &rejoin,
            &mut signals,
            (granted, limit),
        )
        .await
    }

    /// What a run needs to be counted again when its kelpie went away
    struct Rejoin<'a> {
        door: &'a Door,
        ask: Ask,
        patience: Patience,
    }

    /// Start `program` through the tree, in a group of its own and with this
    /// run's mark, keeping this run's own stdio
    fn started(tree: &ProcessTree, program: &OsStr, args: &[&OsStr]) -> io::Result<Child> {
        let mut command = Command::new(program);
        command.args(args);
        tree.spawn(&mut command)
    }

    /// Waits for `child` until it ends or `limit` has passed since `granted`, and
    /// returns the number to exit with
    ///
    /// A signal to the run is passed on to the command's whole tree, and so is an
    /// interrupt, which the terminal sends to the run's group and not to the
    /// command's. A kelpie that went away is asked to count the run again.
    async fn tend(
        child: &mut Child,
        mut held: Option<Visit>,
        rejoin: &Rejoin<'_>,
        signals: &mut Signals,
        (granted, limit): (Instant, Duration),
    ) -> u8 {
        let deadline = granted + limit;
        let patience = rejoin.patience;
        let mut tries = 0;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if let Some(visit) = held {
                        visit.give_back().await;
                    }
                    return exit_number(status);
                }
                Ok(None) => {}
                Err(error) => {
                    if let Some(visit) = held {
                        visit.give_back().await;
                    }
                    return failed(&format!("cannot tell how the command ended: {error}"), 1);
                }
            }
            let rejoining = held.is_none() && tries < patience.tries;
            let event = tokio::select! {
                caught = signals.recv() => Event::Caught(caught),
                () = tokio::time::sleep_until(deadline) => Event::Overran,
                () = gone(&mut held) => Event::KelpieGone,
                () = tokio::time::sleep(patience.every), if rejoining => Event::Rejoin,
                () = tokio::time::sleep(POLL) => Event::Nap,
            };
            match event {
                Event::Nap => {}
                Event::Caught(caught) => {
                    say(&format!(
                        "{}: ending the command and everything it started",
                        caught.name()
                    ));
                    end(child, held).await;
                    return caught.number();
                }
                Event::KelpieGone => {
                    held = None;
                    tries = 0;
                    say("kelpie went away: asking it to count this run again once it is back");
                }
                Event::Rejoin => {
                    tries += 1;
                    match seat(rejoin.door, &rejoin.ask).await {
                        Ok(visit) => {
                            held = Some(visit);
                            say("kelpie counts this run again");
                        }
                        Err(_) if tries == patience.tries => {
                            say("kelpie did not come back, so this run is not counted");
                        }
                        Err(_) => {}
                    }
                }
                Event::Overran => {
                    end(child, held).await;
                    say(&format!(
                        "the command ran past its bound of {:?} and was ended, exiting {TIMED_OUT}: its output above shows where it was",
                        limit
                    ));
                    return TIMED_OUT;
                }
            }
        }
    }

    enum Event {
        Nap,
        Caught(Caught),
        KelpieGone,
        Rejoin,
        Overran,
    }

    /// Ends the command's whole tree, and gives the lease back
    ///
    /// The leader going does not spare what it started: the tree's ladder kills
    /// what ignores the request to end.
    async fn end(child: &mut Child, held: Option<Visit>) {
        if let Err(error) = child.stop(KILL_AFTER) {
            say(&format!("cannot end the command's processes: {error}"));
        }
        if let Some(visit) = held {
            visit.give_back().await;
        }
    }

    /// Knocks and waits for the grant, knocking again for a while if kelpie is
    /// gone or goes away first
    ///
    /// A signal while waiting closes the connection, which withdraws the ask, and
    /// is handed back.
    async fn wait_for_turn(
        door: &Door,
        ask: &Ask,
        signals: &mut Signals,
        patience: Patience,
    ) -> Result<Result<Visit, Caught>, String> {
        let mut tries = 0;
        loop {
            let lost = match one_turn(door, ask, signals).await {
                Turn::Granted(visit) => return Ok(Ok(visit)),
                Turn::Caught(caught) => return Ok(Err(caught)),
                Turn::Refused(why) => return Err(why),
                Turn::Lost { why, answered } => {
                    // A kelpie that answered was there; the next one is a fresh
                    // wait. A run queued behind hours of work may outlast many
                    // restarts, so only a kelpie that never answers wears the
                    // tries down.
                    if answered {
                        tries = 0;
                    }
                    why
                }
            };
            tries += 1;
            if tries > patience.tries {
                return Err(lost);
            }
            if tries == 1 {
                say(&asking_again(&lost, patience));
            }
            tokio::select! {
                () = tokio::time::sleep(patience.every) => {}
                caught = signals.recv() => return Ok(Err(caught)),
            }
        }
    }

    enum Turn {
        Granted(Visit),
        Caught(Caught),
        // Kelpie said no, which asking again would not change.
        Refused(String),
        // No kelpie, or it went away before granting, after answering or not.
        Lost { why: String, answered: bool },
    }

    async fn one_turn(door: &Door, ask: &Ask, signals: &mut Signals) -> Turn {
        let mut visit = match Visit::knock(door, ask).await {
            Ok(visit) => visit,
            Err(why) => {
                return Turn::Lost {
                    why,
                    answered: false,
                };
            }
        };
        let mut first = true;
        let mut answered = false;
        loop {
            // The first answer comes at once from a live kelpie; a queue's grant can take hours.
            let next = async {
                if !first {
                    return visit.answer().await;
                }
                tokio::time::timeout(ANSWER_WITHIN, visit.answer())
                    .await
                    .map_err(|_| "kelpie took the ask and never answered".to_owned())?
            };
            let answer = tokio::select! {
                answer = next => answer,
                caught = signals.recv() => return Turn::Caught(caught),
            };
            first = false;
            match answer {
                Ok(Some(Answer::Granted)) => return Turn::Granted(visit),
                Ok(Some(Answer::Queued(ahead))) => {
                    answered = true;
                    say(&format!("waiting for {}, {ahead} ahead", ask.take));
                }
                Ok(Some(Answer::Error(why))) => {
                    return Turn::Refused(format!("kelpie refused: {why}"));
                }
                Ok(None) => {
                    return Turn::Lost {
                        why: format!("kelpie closed the socket before granting {}", ask.take),
                        answered,
                    };
                }
                Err(why) => return Turn::Lost { why, answered },
            }
        }
    }

    /// A kelpie asked to count a running command seats it at once
    async fn seat(door: &Door, rejoin: &Ask) -> Result<Visit, String> {
        let mut visit = Visit::knock(door, rejoin).await?;
        match tokio::time::timeout(ANSWER_WITHIN, visit.answer()).await {
            Ok(Ok(Some(Answer::Granted))) => Ok(visit),
            other => Err(format!("kelpie answered {other:?}")),
        }
    }

    /// Ends when kelpie closes the connection; never without one
    async fn gone(held: &mut Option<Visit>) {
        let Some(visit) = held else {
            return std::future::pending().await;
        };
        while let Ok(Some(_)) = visit.answer().await {}
    }

    /// A worker cannot start kelpie, so it is told to go on without the lease
    fn unreached(why: &str) -> String {
        format!("{why}: run the tests without the lease, and say so in your final message")
    }

    /// One command's connection to the lease socket, holding its place while open
    #[derive(Debug)]
    struct Visit {
        // Kept open: closing it is how kelpie hears the lease given back.
        stream: BufReader<Conn>,
    }

    impl Visit {
        /// Connects to the socket at `door` and asks
        ///
        /// # Errors
        ///
        /// A message naming the socket when kelpie cannot be reached.
        async fn knock(door: &Door, ask: &Ask) -> Result<Self, String> {
            let stream = match connect(door).await {
                Ok(stream) => stream,
                Err(error) => return Err(format!("cannot reach kelpie at {door}: {error}")),
            };
            Self::over(stream, ask).await
        }

        /// Asks over `stream`, which is already connected to kelpie
        ///
        /// # Errors
        ///
        /// A message when the ask cannot be sent.
        async fn over(mut stream: Conn, ask: &Ask) -> Result<Self, String> {
            let mut line = serde_json::to_vec(ask).expect("an ask serializes to JSON");
            line.push(b'\n');
            stream
                .write_all(&line)
                .await
                .map_err(|error| format!("cannot ask kelpie: {error}"))?;
            Ok(Self {
                stream: BufReader::new(stream),
            })
        }

        /// Kelpie's next answer, or `None` once it has closed the connection
        ///
        /// # Errors
        ///
        /// A message when the answer cannot be read or is not one.
        async fn answer(&mut self) -> Result<Option<Answer>, String> {
            let mut line = String::new();
            let read = self
                .stream
                .read_line(&mut line)
                .await
                .map_err(|error| format!("cannot hear kelpie: {error}"))?;
            if read == 0 {
                return Ok(None);
            }
            let line = line.trim_end();
            serde_json::from_str(line)
                .map(Some)
                .map_err(|error| format!("kelpie answered {line:?} ({error})"))
        }

        /// Lets processes started from now on inherit the connection, or not
        ///
        /// A command started while it is inheritable holds the lease with its
        /// whole process tree, however the process that asked ends.
        ///
        /// # Errors
        ///
        /// A message when the connection's flags cannot be changed.
        #[cfg(unix)]
        fn inheritable(&self, inherit: bool) -> Result<(), String> {
            use std::os::fd::AsFd as _;

            use rustix::io::{FdFlags, fcntl_setfd};
            let flags = if inherit {
                FdFlags::empty()
            } else {
                FdFlags::CLOEXEC
            };
            fcntl_setfd(self.stream.get_ref().as_fd(), flags)
                .map_err(|error| format!("cannot pass the lease to the command: {error}"))
        }

        /// The lease cannot be passed to the command yet
        #[cfg(not(unix))]
        fn inheritable(&self, _inherit: bool) -> Result<(), String> {
            Err("cannot pass the lease to the command: this platform is not supported yet (ADR 0005)".to_owned())
        }

        /// Gives the lease back, whatever else still holds the connection
        ///
        /// Shutting the socket ends it for every process holding a copy, such as
        /// one the command left running, so kelpie hears the end at once.
        async fn give_back(self) {
            let mut stream = self.stream.into_inner();
            let _ = stream.shutdown().await;
        }
    }

    /// Dial `door`, as a leased run does: the Unix socket on macOS, the loopback
    /// address on Linux, which is all each platform's sandbox lets a worker reach
    #[cfg(target_os = "macos")]
    async fn connect(door: &Door) -> io::Result<Conn> {
        let Some(path) = door.socket() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "{door} names no Unix socket",
            ));
        };
        Conn::connect(path).await
    }

    /// [`connect`] on Linux
    #[cfg(target_os = "linux")]
    async fn connect(door: &Door) -> io::Result<Conn> {
        let Some(address) = door.tcp() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "{door} names no address",
            ));
        };
        Conn::connect(address).await
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::Instant;

    use super::*;
    use crate::lease::{Leases, SessionLease};
    use crate::settings::{CARGO_TEST, ConfigDir, Settings};
    use crate::tree::sweep;

    /// How long a test waits for something it forced to happen
    const WAIT: Duration = Duration::from_secs(10);

    #[test]
    fn the_first_failed_knock_says_how_long_it_will_ask() {
        assert_eq!(
            asking_again("cannot reach kelpie", PATIENCE),
            "cannot reach kelpie: asking again every 1s, up to 60 times while kelpie comes back, then exiting 75"
        );
    }

    #[test]
    fn a_run_that_names_no_command_is_usage() {
        for args in [
            &["run", "cargo-test", "--"][..],
            &["run", "cargo-test"][..],
            &["run"][..],
            &["status"][..],
            &[][..],
        ] {
            assert_eq!(dispatch(&os(args)), 2, "{args:?}");
        }
    }

    fn os(words: &[&str]) -> Vec<OsString> {
        words.iter().map(|word| OsString::from(*word)).collect()
    }

    /// A folder of its own under the temporary folder, removed when it drops
    ///
    /// Named short, since on macOS a socket's path may be 103 bytes at most.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let folder =
                env::temp_dir().join(format!("kelpie-lease-run-{}", RunId::new().unwrap()));
            let _ = fs::remove_dir_all(&folder);
            fs::create_dir_all(&folder).unwrap();
            Self(folder)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The leases of a test, with `capacity` places for `cargo-test`
    fn leases(dir: &Scratch, capacity: u32) -> Leases {
        let config = dir.path().join("config");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("settings.json"),
            format!(r#"{{"lease-capacity": {{"cargo-test": {capacity}}}}}"#),
        )
        .unwrap();
        Leases::new(&Settings::load(&ConfigDir::at(&config), "web").unwrap())
    }

    /// The tree a test's command runs through
    fn tree(dir: &Scratch) -> ProcessTree {
        ProcessTree::new(RunId::new().unwrap(), dir.path().join("run.json")).unwrap()
    }

    fn words<'a>(words: &'a [&'a str]) -> Vec<&'a OsStr> {
        words.iter().map(|word| OsStr::new(*word)).collect()
    }

    /// How many places of `name` are held, as kelpie's lease status says
    fn held(leases: &Leases, name: &str) -> usize {
        leases
            .status()
            .iter()
            .find(|status| status.kind == name)
            .map_or(0, |status| status.holders.len())
    }

    /// The pid a script wrote to `file`, once it has written it
    fn pid_in(file: &Path) -> u32 {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Ok(text) = fs::read_to_string(file)
                && let Ok(pid) = text.trim().parse()
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "{file:?} holds no pid");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait until none of `pids` runs
    fn assert_gone(pids: &[u32]) {
        let deadline = Instant::now() + WAIT;
        while let Some(pid) = pids
            .iter()
            .find(|pid| sweep::read(**pid).is_some_and(|reading| !reading.zombie))
        {
            assert!(Instant::now() < deadline, "pid {pid} still runs");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[tokio::test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    async fn the_lease_is_let_go_when_the_command_ends() {
        let dir = Scratch::new();
        let leases = leases(&dir, 1);
        let session: SessionLease = leases.session(&dir.path().join("w")).unwrap();
        let tree = tree(&dir);

        assert_eq!(
            run(
                session.door(),
                CARGO_TEST,
                &words(&["sh", "-c", "exit 3"]),
                RUN_LIMIT,
                PATIENCE,
                &tree,
            )
            .await,
            3
        );
        assert_eq!(
            held(&leases, CARGO_TEST),
            0,
            "the command ended, so the lease is let go"
        );
        // The next command is granted at once, with no kelpie in the way
        assert_eq!(
            run(
                session.door(),
                CARGO_TEST,
                &words(&["sh", "-c", "true"]),
                RUN_LIMIT,
                PATIENCE,
                &tree,
            )
            .await,
            0
        );
        let _ = tree.stop_all(KILL_AFTER);
    }

    #[tokio::test]
    #[cfg(all(unix, any(target_os = "macos", target_os = "linux")))]
    async fn a_command_past_its_bound_is_ended_with_its_children_and_exits_124() {
        let dir = Scratch::new();
        let leases = leases(&dir, 1);
        let session = leases.session(&dir.path().join("w")).unwrap();
        let tree = tree(&dir);
        let pidfile = dir.path().join("sleep.pid");
        let script = format!("sleep 300 & echo $! > {}; wait", pidfile.display());
        let began = Instant::now();

        let code = run(
            session.door(),
            CARGO_TEST,
            &words(&["sh", "-c", &script]),
            Duration::from_millis(300),
            PATIENCE,
            &tree,
        )
        .await;

        assert_eq!(code, TIMED_OUT);
        assert!(
            began.elapsed() < WAIT,
            "it waited for the sleep instead of ending it"
        );
        assert_gone(&[pid_in(&pidfile)]);
        assert_eq!(held(&leases, CARGO_TEST), 0, "its place came back with it");
        let _ = tree.stop_all(KILL_AFTER);
    }

    #[tokio::test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    async fn a_command_that_cannot_start_gives_the_lease_back() {
        let dir = Scratch::new();
        let leases = leases(&dir, 1);
        let session = leases.session(&dir.path().join("w")).unwrap();
        let tree = tree(&dir);

        assert_eq!(
            run(
                session.door(),
                CARGO_TEST,
                &words(&["kelpie-no-such-program"]),
                RUN_LIMIT,
                PATIENCE,
                &tree,
            )
            .await,
            NOT_RUN
        );
        assert_eq!(held(&leases, CARGO_TEST), 0);
        let _ = tree.stop_all(KILL_AFTER);
    }

    #[tokio::test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    async fn a_run_with_no_kelpie_behind_the_socket_exits_75() {
        let dir = Scratch::new();
        let tree = tree(&dir);
        #[cfg(target_os = "macos")]
        let door = Door::parse(dir.path().join("nothing.sock").to_str().unwrap()).unwrap();
        #[cfg(target_os = "linux")]
        let door = Door::parse("127.0.0.1:1").unwrap();
        let patience = Patience {
            tries: 2,
            every: Duration::from_millis(10),
        };

        assert_eq!(
            run(
                &door,
                CARGO_TEST,
                &words(&["true"]),
                RUN_LIMIT,
                patience,
                &tree,
            )
            .await,
            NO_LEASE
        );
        let _ = tree.stop_all(KILL_AFTER);
    }

    #[test]
    fn a_signal_ending_a_run_carries_the_shell_number() {
        assert_eq!(Caught::Interrupt.number(), 130);
        assert_eq!(Caught::Hangup.number(), 129);
        assert_eq!(Caught::Terminate.number(), 143);
    }

    #[test]
    #[cfg(unix)]
    fn a_command_killed_by_a_signal_exits_as_the_shell_says() {
        use std::os::unix::process::ExitStatusExt as _;

        let killed = std::process::Command::new("sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .unwrap();
        assert_eq!(exit_number(killed), 143);
        assert_eq!(exit_number(ExitStatus::from_raw(0)), 0);
        assert_eq!(exit_number(ExitStatus::from_raw(0x0100)), 1);
    }
}
