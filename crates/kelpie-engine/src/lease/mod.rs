//! Leases: a capacity-N hold every command asking for one queues behind
//!
//! Two workers' `cargo test` runs would starve the machine, so each waits for a
//! place in a counted lease ([`counted`]) and runs its command while it holds one
//! (`docs/design/workers.md`, Slots, pacing and leases).
//!
//! ```text
//! kelpie ── ACP over stdio ──> harness ──> kelpie lease run ──> cargo test
//!   ▲                                          │
//!   └──── the session's lease socket ─────────┘
//! ```
//!
//! [`Leases`] is one per run and holds every lease's book. The capacity per name
//! is the `lease-capacity` setting ([`crate::settings::Resolved::lease_capacity_for`]),
//! and a lease no layer names holds one command at a time. [`Leases::session`]
//! opens the socket of one session, by default [`crate::sandbox::worker::LEASE_SOCKET`]
//! in its state folder, and serves asks on it until that session's lease is
//! dropped. The worker's policy names whichever socket its own session opened
//! ([`crate::sandbox::worker::Worker::lease`]). One socket per session is what
//! keeps a worker out of another's: its policy names its own and no other.
//!
//! A lease is held by connection liveness, as SK's door held it: the ask is one
//! JSON line, the answers follow it, and the place is given back when the
//! connection closes, however the command ended. A kelpie tool could not be held
//! that way, since a tool lives as long as the session's MCP conversation and not
//! as long as one command, which is why the lease has its own socket
//! (`docs/design/kelpie-tools.md`, The route out).
//!
//! Lifted from SK's `core/lease/door.rs` framing and `core/lease/cli/mod.rs`
//! dispatch at `1dc51c040`. The dog's own book, e2e host, forge, pick, fit and
//! credit asks stay in SK: kelpie has counted leases only, and the rest come with
//! the review bots (`docs/lift-plan.md`, Phase 3).
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::Path;
//!
//! use kelpie_engine::lease::Leases;
//! use kelpie_engine::settings::{self, Settings};
//!
//! let settings = Settings::load(&settings::ConfigDir::from_env()?, "kelpie")?;
//! let leases = Leases::new(&settings);
//! // One per worker, named in its environment as `KELPIE_LEASE_SOCKET`
//! let lease = leases.session(Path::new("/state/projects/kelpie/workers/81"))?;
//! assert!(lease.door().socket().is_some() || lease.door().tcp().is_some());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::pacer::Timestamp;
use crate::settings::Resolved;
use crate::tools::Door;

pub mod counted;
pub mod run;

use counted::{Connection, CountedStatus, Taken, Taker};

/// The variable naming the socket `kelpie lease run` reaches kelpie through
///
/// Its text form is a socket's path, or `host:port`, as [`Door`] reads one.
pub const SOCKET_VAR: &str = "KELPIE_LEASE_SOCKET";

/// The longest a lease name may be, past which it is a typo
const LONGEST_NAME: usize = 64;

/// What a command asks for, its connection's first and only line
// wire format: changing this is a breaking change to the lease socket
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ask {
    /// The counted lease it wants
    pub take: String,
    /// Its process, for the lease status
    pub pid: u32,
    /// What it runs, for the lease status
    pub what: String,
    /// Whether its command already runs under a grant from a kelpie that went
    /// away, so kelpie seats it at once rather than counting it twice.
    /// Kelpie takes it on trust, as it does the rest of the ask.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub running: bool,
}

/// What kelpie answers, one line each
// wire format: changing this is a breaking change to the lease socket
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Answer {
    /// Waiting, with this many ahead
    Queued(usize),
    /// Held until the connection closes
    Granted,
    /// Refused, saying why, and kelpie closes the connection
    Error(String),
}

/// kelpie's leases, each with the capacity its name holds
///
/// One per run, so every project's workers queue behind the same machine.
/// Dropping it closes every session's socket and ends every connection served
/// on it, which gives back every place held.
#[derive(Debug)]
pub struct Leases {
    shared: Arc<Shared>,
    /// The runtime the sockets are served on, kept by the thread that drives it
    handle: tokio::runtime::Handle,
    /// Cancelled when this drops, which ends the runtime thread
    stop: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
}

#[derive(Debug)]
struct Shared {
    /// Where each lease's capacity comes from
    settings: Resolved,
    /// The lease each name has, with the waiters it holds
    leases: Mutex<BTreeMap<String, Book>>,
    next: AtomicU64,
}

#[derive(Debug)]
struct Book {
    counted: counted::CountedLease,
    /// Each waiter's wake, by connection: cancelled when its turn comes
    waiting: HashMap<Connection, CancellationToken>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Leases {
    /// Hold every lease, each with the capacity the settings name for it
    #[must_use]
    pub fn new(settings: &Resolved) -> Self {
        let stop = CancellationToken::new();
        let shared = Arc::new(Shared {
            settings: settings.clone(),
            leases: Mutex::default(),
            next: AtomicU64::new(0),
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime starts");
        let handle = runtime.handle().clone();
        // The thread drives every socket until the leases are dropped
        let thread = std::thread::Builder::new()
            .name("kelpie-leases".to_owned())
            .spawn({
                let stop = stop.clone();
                move || runtime.block_on(stop.cancelled())
            })
            .expect("a thread starts");
        Self {
            shared,
            handle,
            stop,
            thread: Some(thread),
        }
    }

    /// Open the lease socket of a session whose state folder is `state_dir`, and
    /// serve asks on it until the returned lease is dropped
    ///
    /// macOS binds [`crate::sandbox::worker::LEASE_SOCKET`] there, or under the
    /// temporary folder when the state folder is too deep for a socket's path.
    /// Linux binds a port on `127.0.0.1`, which its sandbox policy names in place
    /// of a socket (ADR 0005). Either way [`SessionLease::door`] is the one the
    /// policy names.
    ///
    /// # Errors
    ///
    /// The state folder could not be made, the socket could not be bound, or no
    /// path is short enough for a socket.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn session(&self, state_dir: &Path) -> io::Result<SessionLease> {
        let (door, listener, socket) = bind(state_dir)?;
        let listener = {
            let _entered = self.handle.enter();
            crate::tools::door::Listener::from_std(listener)?
        };
        let stop = self.stop.child_token();
        self.handle.spawn(serve::accept(
            listener,
            Arc::clone(&self.shared),
            stop.clone(),
        ));
        Ok(SessionLease(Arc::new(Socket { door, stop, socket })))
    }

    /// The platform has no lease socket yet
    ///
    /// # Errors
    ///
    /// Always: Windows waits with the rest of Windows (ADR 0005).
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn session(&self, _state_dir: &Path) -> io::Result<SessionLease> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "kelpie's leases have no socket on this platform yet (ADR 0005)",
        ))
    }

    /// Who holds and waits for each lease, by name
    #[must_use]
    pub fn status(&self) -> Vec<CountedStatus> {
        locked(&self.shared.leases)
            .values()
            .map(|book| book.counted.status())
            .collect()
    }
}

impl Drop for Leases {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Shared {
    /// Asks for `name` on behalf of `taker`, seating it at once when `running`
    /// says its command already runs under a grant that went away
    ///
    /// A lease no layer names a capacity for holds one command at a time.
    fn ask(&self, name: &str, taker: Taker, running: bool, now: Timestamp) -> Taken {
        let connection = taker.connection;
        let mut leases = locked(&self.leases);
        let capacity = self.settings.lease_capacity_for(name).count();
        let book = leases.entry(name.to_owned()).or_insert_with(|| Book {
            counted: counted::CountedLease::named(name.to_owned(), capacity),
            waiting: HashMap::new(),
        });
        let taken = if running {
            book.counted.seat(taker, now)
        } else {
            book.counted.ask(taker, now)
        };
        if matches!(taken, Taken::Queued { .. }) {
            // Held across the ask, so a grant made before the waiter listens is
            // not missed
            book.waiting.insert(connection, CancellationToken::new());
        }
        taken
    }

    /// Gives back the place or queue entry `connection` had in `name`, and wakes
    /// each waiter the room it made grants
    fn leave(&self, name: &str, connection: Connection, now: Timestamp) {
        let mut leases = locked(&self.leases);
        let Some(book) = leases.get_mut(name) else {
            return;
        };
        book.waiting.remove(&connection);
        for granted in book.counted.leave(connection, now) {
            if let Some(wake) = book.waiting.remove(&granted) {
                wake.cancel();
            }
        }
    }

    /// The wake that says `connection`'s turn in `name` has come, `None` when it
    /// was granted at once or is gone already
    fn wake(&self, name: &str, connection: Connection) -> Option<CancellationToken> {
        locked(&self.leases)
            .get(name)
            .and_then(|book| book.waiting.get(&connection))
            .cloned()
    }
}

/// Whether `text` is a lease name kelpie can hold: lowercase letters, digits and
/// `-`, no longer than [`LONGEST_NAME`]
fn named(text: &str) -> Option<String> {
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    (!text.is_empty() && text.len() <= LONGEST_NAME && text.chars().all(allowed))
        .then(|| text.to_owned())
}

/// One session's lease socket, held while its worker runs
///
/// Cheap to clone, and every clone holds the same socket: it is the last one
/// dropped that closes it, ends every connection served on it, gives back every
/// place those connections held, and removes its file.
#[derive(Debug, Clone)]
pub struct SessionLease(Arc<Socket>);

/// What every clone of one [`SessionLease`] holds
#[derive(Debug)]
struct Socket {
    door: Door,
    stop: CancellationToken,
    /// The socket file, removed when the last clone is dropped (macOS)
    socket: Option<PathBuf>,
}

impl SessionLease {
    /// The socket this session's `kelpie lease run` reaches kelpie through, for
    /// its sandbox policy and its environment
    #[must_use]
    pub fn door(&self) -> &Door {
        &self.0.door
    }

    /// The pair naming this socket, for a harness's environment
    ///
    /// A harness started with it can run `kelpie lease run`, which reads the
    /// socket from [`SOCKET_VAR`].
    #[must_use]
    pub fn env(&self) -> (OsString, OsString) {
        (
            OsString::from(SOCKET_VAR),
            OsString::from(self.0.door.to_string()),
        )
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(socket) = &self.socket {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// Bind the lease socket of a session whose state folder is `state_dir`
///
/// macOS binds [`crate::sandbox::worker::LEASE_SOCKET`] there, replacing one a
/// killed run left, or under the temporary folder with a name no other session
/// holds when the path is longer than a socket's path may be. Linux binds a port
/// on `127.0.0.1` the OS picks. Each listener is made without a runtime and
/// registered with one by the caller.
#[cfg(target_os = "macos")]
fn bind(state_dir: &Path) -> io::Result<(Door, std::os::unix::net::UnixListener, Option<PathBuf>)> {
    // Both are macOS-only: Linux binds no socket file and draws no run id for a name
    use crate::run::RunId;
    use crate::sandbox::worker::LEASE_SOCKET;

    std::fs::create_dir_all(state_dir)?;
    let mut socket = state_dir.join(LEASE_SOCKET);
    // A state folder too deep for a socket's path gets one under the temporary
    // folder with a name no other session can take, as the tools door does. The
    // worker's policy names whichever path this bound to
    // (`sandbox::worker::policy`).
    if socket.as_os_str().len() > crate::tools::door::LONGEST_SOCKET {
        // 96 random bits is enough that no two sessions take the same name, and
        // short enough that the name fits a socket's path under any temporary
        // folder kelpie runs with
        socket = std::env::temp_dir().join(format!(
            "kelpie-lease-{}.sock",
            &RunId::new()?.to_string()[..24]
        ));
    }
    if socket.as_os_str().len() > crate::tools::door::LONGEST_SOCKET {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is {} bytes, longer than the {} a socket's path may be",
                socket.display(),
                socket.as_os_str().len(),
                crate::tools::door::LONGEST_SOCKET
            ),
        ));
    }
    let _ = std::fs::remove_file(&socket);
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    listener.set_nonblocking(true)?;
    let door = Door::parse(&socket.display().to_string())?;
    Ok((door, listener, Some(socket)))
}

/// [`bind`] on Linux: a port on `127.0.0.1` the OS picks
#[cfg(target_os = "linux")]
fn bind(state_dir: &Path) -> io::Result<(Door, std::net::TcpListener, Option<PathBuf>)> {
    std::fs::create_dir_all(state_dir)?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let door = Door::parse(&listener.local_addr()?.to_string())?;
    Ok((door, listener, None))
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod serve {
    //! The server end of one session's lease socket, on the runtime thread

    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
    use tokio_util::sync::CancellationToken;

    use crate::lease::counted::{Connection, Taker};
    use crate::lease::{Answer, Ask, Shared, Taken, named};
    use crate::pacer::Timestamp;
    use crate::tools::door::{Accepted, Listener};

    /// How long a connection has to send its ask line; a live kelpie is asked at once
    const ASK_WITHIN: Duration = Duration::from_secs(5);

    /// The longest ask line read: an ask is one short JSON line, so a longer one is no ask
    const LONGEST_ASK_LINE: u64 = 4096;

    /// Accept connections on one session's socket until `stop`, each served on a
    /// task of its own
    pub(crate) async fn accept(listener: Listener, shared: Arc<Shared>, stop: CancellationToken) {
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                accepted = listener.accept() => {
                    if let Ok((stream, _)) = accepted {
                        tokio::spawn(connection(stream, Arc::clone(&shared), stop.child_token()));
                    }
                }
            }
        }
    }

    /// Answer a connection that asks for a lease, holding its place while the
    /// connection stays open
    ///
    /// The place is given back when the connection closes, however the command it
    /// wrapped ended, and when this task ends for any other reason.
    async fn connection(stream: Accepted, shared: Arc<Shared>, stop: CancellationToken) {
        let (read, mut write) = tokio::io::split(stream);
        let mut read = BufReader::new(read);
        let Ok(ask) = tokio::time::timeout(ASK_WITHIN, ask_line(&mut read)).await else {
            return;
        };
        let Ok(Some(ask)) = ask else { return };
        let Some(name) = named(&ask.take) else {
            let _ = said(
                &mut write,
                &Answer::Error(format!("{:?} is no lease name", ask.take)),
            )
            .await;
            return;
        };
        let connection = Connection(shared.next.fetch_add(1, Ordering::SeqCst));
        let taker = Taker {
            connection,
            pid: ask.pid,
            what: ask.what,
        };
        // Gives the place back however this task ends, which is how a lease is
        // held by the connection's liveness and not by any command's word
        let _held = Held {
            shared: &shared,
            name: &name,
            connection,
        };
        match shared.ask(&name, taker, ask.running, Timestamp::now()) {
            Taken::Granted => {
                if said(&mut write, &Answer::Granted).await.is_err() {
                    return;
                }
            }
            Taken::Queued { ahead } => {
                if said(&mut write, &Answer::Queued(ahead)).await.is_err() {
                    return;
                }
                // A client that closes while queued withdraws its ask, so it
                // never holds a place it was never granted
                let Some(wake) = shared.wake(&name, connection) else {
                    return;
                };
                tokio::select! {
                    biased;
                    () = stop.cancelled() => return,
                    () = wake.cancelled() => {}
                    // A client that sends nothing and closes is withdrawing
                    () = closed(&mut read) => return,
                }
                if said(&mut write, &Answer::Granted).await.is_err() {
                    return;
                }
            }
        }
        // The lease is held by this connection's liveness: it waits here until
        // the client's end closes, which is how kelpie hears the command it
        // wrapped ended, or until the session's lease is dropped.
        tokio::select! {
            biased;
            () = stop.cancelled() => {}
            () = closed(&mut read) => {}
        }
    }

    /// Ends once the client has closed its end of `read`, and never before
    async fn closed(read: &mut BufReader<tokio::io::ReadHalf<Accepted>>) {
        loop {
            match read.fill_buf().await {
                Ok([]) => return,
                Ok(buf) => {
                    // Whatever a client sends after its ask is ignored, and read
                    // on so a full buffer does not spin here
                    let len = buf.len();
                    read.consume(len);
                }
                Err(_) => return,
            }
        }
    }

    /// The connection's first line, read as an ask, or `None` when it has none,
    /// is longer than an ask may be, or is not one
    async fn ask_line<R: tokio::io::AsyncRead + Unpin>(
        read: &mut BufReader<R>,
    ) -> io::Result<Option<Ask>> {
        let mut line = Vec::new();
        let mut limited = read.take(LONGEST_ASK_LINE);
        limited.read_until(b'\n', &mut line).await?;
        // No newline within the limit: no ask, or a line past what an ask may be
        if line.pop() != Some(b'\n') {
            return Ok(None);
        }
        let Ok(text) = String::from_utf8(line) else {
            return Ok(None);
        };
        Ok(serde_json::from_str::<Ask>(text.trim_end()).ok())
    }

    /// One answer line
    async fn said(write: &mut tokio::io::WriteHalf<Accepted>, answer: &Answer) -> io::Result<()> {
        let mut line = serde_json::to_vec(answer).expect("an answer serializes to JSON");
        line.push(b'\n');
        write.write_all(&line).await?;
        write.flush().await
    }

    /// Gives the place back, and grants the waiters it makes room for, when the
    /// connection's task ends however it ends
    struct Held<'a> {
        shared: &'a Shared,
        name: &'a str,
        connection: Connection,
    }

    impl Drop for Held<'_> {
        fn drop(&mut self) {
            self.shared
                .leave(self.name, self.connection, Timestamp::now());
        }
    }
}

#[cfg(test)]
mod tests;
