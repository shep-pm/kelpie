//! The run itself, on the platform's lease socket
//!
//! macOS and Linux only, as every kelpie socket is (ADR 0005).

use std::ffi::OsStr;
use std::io;
use std::process::Command;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::time::Instant;

use super::{
    Caught, KILL_AFTER, NO_LEASE, NOT_RUN, Patience, Signals, TIMED_OUT, asking_again, exit_number,
    failed, say, shown,
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
        Err(
            "cannot pass the lease to the command: this platform is not supported yet (ADR 0005)"
                .to_owned(),
        )
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
            format!("{door} names no Unix socket"),
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
            format!("{door} names no address"),
        ));
    };
    Conn::connect(address).await
}
