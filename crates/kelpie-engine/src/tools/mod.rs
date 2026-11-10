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
//! The tools on the route are `network`'s `request_network`, which asks the
//! maintainer for a host the sandbox refuses.
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
pub(crate) use network::Asks;
pub use network::Reach;
pub use relay::{DOOR_VAR, TOKEN_VAR, run_from_args};
use tokio_util::sync::CancellationToken;

// `pub(crate)` for the lease socket, which binds and accepts on the same door
pub(crate) mod door;
mod network;
mod relay;
#[cfg(test)]
mod tests;

/// How long a harness has to start the conversation (`initialize`) once its
/// relay is through, before kelpie lets the connection go
const HELLO_WITHIN: std::time::Duration = std::time::Duration::from_secs(30);

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
#[derive(Debug)]
struct Shared {
    /// The live tokens, and whose each is
    live: Mutex<HashMap<String, Live>>,
    /// The sessions with a connection being served, one entry per connection
    connected: Mutex<Vec<SessionKey>>,
    next: AtomicU64,
    /// How long a connection has to open the MCP conversation
    hello_within: std::time::Duration,
}

#[derive(Debug, Clone)]
struct Live {
    key: SessionKey,
    /// The session's waiting requests
    asks: Asks,
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
        Self::start_within(data_dir, HELLO_WITHIN)
    }

    /// [`Tools::start`], letting a connection go when its harness has not opened
    /// the MCP conversation within `hello_within`
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub(crate) fn start_within(
        data_dir: &Path,
        hello_within: std::time::Duration,
    ) -> io::Result<Self> {
        let (open, listener) = door::open(data_dir)?;
        let shared = Arc::new(Shared {
            live: Mutex::default(),
            connected: Mutex::default(),
            next: AtomicU64::new(0),
            hello_within,
        });
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
        let asks = Asks::default();
        locked(&self.shared.live).insert(
            token.clone(),
            Live {
                key,
                asks: asks.clone(),
                ended: ended.clone(),
            },
        );
        SessionTools {
            inner: Arc::new(Inner {
                key,
                token,
                door: self.door.clone(),
                shared: Arc::clone(&self.shared),
                asks,
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
    asks: Asks,
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

    /// The session's waiting requests
    pub(crate) fn asks(&self) -> &Asks {
        &self.inner.asks
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

    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    };
    use rmcp::service::RequestContext;
    use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    use tokio_util::sync::CancellationToken;

    use super::door::{Accepted, Listener};
    use super::network::REQUEST_NETWORK;
    use super::{Asks, SessionKey, Shared, locked};

    /// How long a connection has to send its token
    const TOKEN_WITHIN: Duration = Duration::from_secs(5);

    /// The longest token line read: a token is 32 characters, so a longer line is no token
    const LONGEST_TOKEN_LINE: u64 = 128;

    /// The MCP server one connection is served by, for the session its token names
    #[derive(Debug)]
    struct Server {
        /// The session's waiting requests, which its tools act for
        asks: Asks,
    }

    impl ServerHandler for Server {
        fn get_info(&self) -> ServerConfig {
            ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        }

        async fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, ErrorData> {
            Ok(ListToolsResult::with_all_items(vec![request_network()]))
        }

        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, ErrorData> {
            if request.name != REQUEST_NETWORK {
                return Err(ErrorData::invalid_params(
                    format!("kelpie has no tool named {}", request.name),
                    None,
                ));
            }
            let argument = |name: &str| {
                request
                    .arguments
                    .as_ref()
                    .and_then(|arguments| arguments.get(name))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let (Some(host), Some(why)) = (argument("host"), argument("why")) else {
                return Ok(said(Err(
                    "request_network takes a `host` and a `why`, both text".to_owned(),
                )));
            };
            Ok(said(self.asks.request(&host, &why).await))
        }
    }

    /// A tool's answer, which the agent reads as one line
    fn said(line: Result<String, String>) -> CallToolResponse {
        let (line, failed) = match line {
            Ok(line) => (line, false),
            Err(line) => (line, true),
        };
        let content = vec![ContentBlock::text(line)];
        CallToolResponse::from(if failed {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        })
    }

    fn request_network() -> Tool {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "host": {
                    "type": "string",
                    "description": "The host to reach, a name such as docs.rs or *.docs.rs. Not a URL, a path or a port.",
                },
                "why": {
                    "type": "string",
                    "description": "Why you need it, in a sentence the maintainer reads when they decide.",
                },
            },
            "required": ["host", "why"],
        });
        let serde_json::Value::Object(schema) = schema else {
            unreachable!("a JSON object literal")
        };
        Tool::new(
            REQUEST_NETWORK,
            "Ask the maintainer to let the sandbox reach a host it refused. Waits for their answer, and says in one line whether the host is now allowed.",
            schema,
        )
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
        // A harness that connects and never says hello holds nothing open: the
        // handshake ends with the session's tools, and with its deadline
        let hello = (Server { asks: live.asks }).serve_with_ct((read, write), ended.child_token());
        let running = tokio::select! {
            () = ended.cancelled() => return,
            started = tokio::time::timeout(shared.hello_within, hello) => match started {
                Ok(Ok(running)) => running,
                _ => return,
            },
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
