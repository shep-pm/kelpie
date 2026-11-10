//! kelpie's own tools, served to every session over MCP through one door
//!
//! A session's agent calls the tools kelpie itself serves, and reaches them from
//! inside its sandbox by one route (`docs/design/kelpie-tools.md`):
//!
//! ```text
//! kelpie ── ACP over stdio ──> harness            (inside crabpot)
//!   ▲                            └─ kelpie tools  (MCP over stdio)
//!   └──────── the route out ────────┘
//! ```
//!
//! [`Tools::start`] opens the [`Door`] once per run and serves MCP on each
//! connection that brings a live token. [`Tools::session`] hands out a
//! [`SessionTools`] with a fresh token, which a [`Session`](crate::session::Session)
//! gives its harness as a stdio server. The harness starts that server, the
//! binary itself run as `kelpie tools` ([`run_from_args`]), as its own child, so
//! the relay sits in the session's process group with its `KELPIE_RUN` and
//! stops with the session (ADR 0001). Nothing here starts a process. The relay
//! parses no MCP: it carries bytes to the door, and the token says which session
//! is asking.
//!
//! Dropping a [`SessionTools`] ends its token and the connections made with it.
//! Dropping the [`Tools`] closes the door.
//!
//! This is the route with no tools on it yet.
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::Path;
//!
//! use kelpie_engine::tools::{self, Tools};
//!
//! // After the sandbox's own first call in `main`
//! tools::run_from_args();
//!
//! let tools = Tools::start(Path::new("/data/kelpie"))?;
//! let session = tools.session();
//! // `session.door()` goes in the sandbox's policy, `session` in the session
//! # let _ = session;
//! # Ok::<(), std::io::Error>(())
//! ```

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

pub use door::Door;
pub use relay::{DOOR_VAR, TOKEN_VAR, run_from_args};
use tokio_util::sync::CancellationToken;

mod door;
mod relay;
#[cfg(test)]
mod tests;

/// The name the server goes by in a harness's `mcpServers`
pub const SERVER_NAME: &str = "kelpie";

/// Which session is asking, as the token it brought says
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionKey(u64);

impl fmt::Display for SessionKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// What the door and the sessions share
#[derive(Debug, Default)]
struct Shared {
    /// The live tokens, and whose each is
    live: Mutex<HashMap<String, Live>>,
    /// The sessions with a connection being served, one entry per connection
    connected: Mutex<Vec<SessionKey>>,
    next: AtomicU64,
}

#[derive(Debug, Clone)]
struct Live {
    key: SessionKey,
    /// Cancelled when the session's tools are dropped
    ended: CancellationToken,
}

fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 128 random bits as 32 hex characters, which no one can guess
fn hex128() -> io::Result<String> {
    use std::fmt::Write as _;

    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes
        .iter()
        .fold(String::with_capacity(32), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        }))
}

/// kelpie's tools server, with its door open
///
/// One per run.
#[derive(Debug)]
pub struct Tools {
    shared: Arc<Shared>,
    door: Door,
    /// Cancelled when this drops, which ends every connection and the accept
    stop: CancellationToken,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Removes the socket on drop
    _open: door::Open,
}

impl Tools {
    /// Open the door, keeping its socket (macOS) in `data_dir`, and serve MCP on
    /// each connection that brings a live token
    ///
    /// # Errors
    ///
    /// The door could not be opened or its server could not start, or this
    /// platform has no door yet (Windows, ADR 0005).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn start(data_dir: &Path) -> io::Result<Self> {
        let (open, listener) = door::open(data_dir)?;
        let shared = Arc::new(Shared::default());
        let stop = CancellationToken::new();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()?;
        let listener = {
            let _entered = runtime.enter();
            door::Listener::from_std(listener)?
        };
        let thread = std::thread::Builder::new()
            .name("kelpie-tools".to_owned())
            .spawn({
                let (shared, stop) = (Arc::clone(&shared), stop.clone());
                move || runtime.block_on(serve::accept(listener, shared, stop))
            })?;
        Ok(Self {
            shared,
            door: open.door.clone(),
            stop,
            thread: Some(thread),
            _open: open,
        })
    }

    /// The platform has no door yet
    ///
    /// # Errors
    ///
    /// Always: Windows waits with the rest of Windows (ADR 0005).
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn start(_data_dir: &Path) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "kelpie's tools have no door on this platform yet (ADR 0005)",
        ))
    }

    /// The door, which the sandbox's policy names
    #[must_use]
    pub fn door(&self) -> &Door {
        &self.door
    }

    /// The sessions with a connection being served now, one entry per connection
    #[must_use]
    pub fn connected(&self) -> Vec<SessionKey> {
        locked(&self.shared.connected).clone()
    }

    /// Tools for one more session, with a token of its own
    ///
    /// # Panics
    ///
    /// When the system has no random bytes to draw a token from.
    #[must_use]
    pub fn session(&self) -> SessionTools {
        let token = hex128().unwrap_or_else(|error| panic!("cannot draw a token: {error}"));
        let key = SessionKey(self.shared.next.fetch_add(1, Ordering::SeqCst));
        let ended = self.stop.child_token();
        locked(&self.shared.live).insert(
            token.clone(),
            Live {
                key,
                ended: ended.clone(),
            },
        );
        SessionTools {
            inner: Arc::new(Inner {
                key,
                token,
                door: self.door.clone(),
                shared: Arc::clone(&self.shared),
                ended,
            }),
        }
    }
}

impl Drop for Tools {
    fn drop(&mut self) {
        self.stop.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One session's way in: its token, and the door it opens
///
/// Cheap to clone. The token lives until the last clone is dropped.
#[derive(Debug, Clone)]
pub struct SessionTools {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    key: SessionKey,
    token: String,
    door: Door,
    shared: Arc<Shared>,
    ended: CancellationToken,
}

impl SessionTools {
    /// Which session this is, as a connection with its token is served
    #[must_use]
    pub fn key(&self) -> SessionKey {
        self.inner.key
    }

    /// The door this session's relay dials
    #[must_use]
    pub fn door(&self) -> &Door {
        &self.inner.door
    }

    /// The token the relay sends first, which only this session holds
    #[must_use]
    pub fn token(&self) -> &str {
        &self.inner.token
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        locked(&self.shared.live).remove(&self.token);
        self.ended.cancel();
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod serve {
    //! The server end of the door, on the runtime thread

    use std::sync::Arc;
    use std::time::Duration;

    use rmcp::model::{ServerCapabilities, ServerConfig};
    use rmcp::{ServerHandler, ServiceExt};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    use tokio_util::sync::CancellationToken;

    use super::door::{Accepted, Listener};
    use super::{SessionKey, Shared, locked};

    /// How long a connection has to send its token
    const TOKEN_WITHIN: Duration = Duration::from_secs(5);

    /// The longest token line read: a token is 32 characters, so a longer line is no token
    const LONGEST_TOKEN_LINE: u64 = 128;

    /// The MCP server one connection is served by, for the session its token names
    #[derive(Debug)]
    struct Server {
        /// Which session is asking, for the tools that come next to act for
        #[expect(dead_code, reason = "no tool is on the route yet")]
        key: SessionKey,
    }

    impl ServerHandler for Server {
        fn get_info(&self) -> ServerConfig {
            ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        }
    }

    /// Accept connections until `stop`, each served on a task of its own
    pub(super) async fn accept(listener: Listener, shared: Arc<Shared>, stop: CancellationToken) {
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                accepted = listener.accept() => {
                    if let Ok((stream, _)) = accepted {
                        tokio::spawn(connection(stream, Arc::clone(&shared)));
                    }
                }
            }
        }
    }

    /// Answer a connection that brings a live token, and drop one that does not
    async fn connection(stream: Accepted, shared: Arc<Shared>) {
        let (read, write) = tokio::io::split(stream);
        let mut read = BufReader::new(read);
        let Ok(Ok(Some(token))) = tokio::time::timeout(TOKEN_WITHIN, token(&mut read)).await else {
            return;
        };
        let Some(live) = locked(&shared.live).get(&token).cloned() else {
            return;
        };
        locked(&shared.connected).push(live.key);
        let _gone = Gone {
            shared: &shared,
            key: live.key,
        };
        let ended = live.ended;
        let Ok(running) = (Server { key: live.key })
            .serve_with_ct((read, write), ended.child_token())
            .await
        else {
            return;
        };
        tokio::select! {
            () = ended.cancelled() => {}
            _ = running.waiting() => {}
        }
    }

    /// The first line of a connection, without its newline, or `None` if it has none
    async fn token<R: tokio::io::AsyncRead + Unpin>(
        read: &mut BufReader<R>,
    ) -> std::io::Result<Option<String>> {
        let mut line = Vec::new();
        let mut limited = read.take(LONGEST_TOKEN_LINE);
        limited.read_until(b'\n', &mut line).await?;
        if line.pop() != Some(b'\n') {
            return Ok(None);
        }
        Ok(String::from_utf8(line).ok())
    }

    /// Takes the connection off the list of those served when it ends
    struct Gone<'a> {
        shared: &'a Shared,
        key: SessionKey,
    }

    impl Drop for Gone<'_> {
        fn drop(&mut self) {
            let mut connected = locked(&self.shared.connected);
            if let Some(at) = connected.iter().position(|key| *key == self.key) {
                connected.remove(at);
            }
        }
    }
}
