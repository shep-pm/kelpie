//! `request_network`: the agent asks the maintainer for a host the sandbox refuses
//!
//! The first of kelpie's tools (`docs/design/kelpie-tools.md`, The sandbox
//! request). A sandbox refusal is a dead end for an agent, so it asks instead:
//! [`Asks::request`] puts a [`SessionEvent::SandboxRequest`] on the turn's sink
//! and waits. [`Asks::answer`] is the maintainer's answer, and `Once` and
//! `Project` widen the running sandbox before the agent hears of it.
//!
//! Anything that ends the wait without an answer, a cancelled turn or a session
//! that stops, is a denial.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crabpot::Config;
use kelpie_protocol::{SandboxAnswer, SessionEvent};
use serde_json::json;
use tokio::sync::oneshot;

/// The name the tool goes by
pub(super) const REQUEST_NETWORK: &str = "request_network";

/// What the agent hears when the maintainer denies a request, or no one is there to answer it
const DENIED: &str = "denied by the maintainer";

/// The hosts a running sandbox lets through, which an answer widens
///
/// [`crate::sandbox::Network`] is the real one.
pub trait Reach: Send + Sync + fmt::Debug {
    /// Whether the sandbox already lets `host` through
    fn allows(&self, host: &str) -> bool;

    /// Let `host` through, for as long as the sandbox runs
    fn allow(&self, host: &str);
}

/// Puts an event on the turn in flight, and says whether a turn took it
pub(crate) type Emit = Arc<dyn Fn(SessionEvent) -> bool + Send + Sync>;

/// One session's waiting requests, and what answers them
///
/// Cheap to clone: the session, its tools and the tool server share one.
#[derive(Clone, Default)]
pub(crate) struct Asks {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    next: u64,
    waiting: HashMap<String, Waiting>,
    turn: Option<Emit>,
    reach: Option<Arc<dyn Reach>>,
    /// The session is gone: nothing more is asked
    closed: bool,
}

struct Waiting {
    host: String,
    answer: oneshot::Sender<SandboxAnswer>,
}

impl fmt::Debug for Asks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Asks")
            .field("waiting", &self.state().waiting.len())
            .finish_non_exhaustive()
    }
}

impl Asks {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The sandbox an answer widens
    pub(crate) fn bind(&self, reach: Arc<dyn Reach>) {
        self.state().reach = Some(reach);
    }

    /// Where a request's event goes, or `None` when no turn can be asked
    pub(crate) fn emit_to(&self, turn: Option<Emit>) {
        self.state().turn = turn;
    }

    /// Answer the request `id`, and say whether one was waiting for it
    ///
    /// `Once` and `Project` let its host through the sandbox first, so the agent
    /// can use it as soon as it hears. Saving a `Project` answer to the
    /// project's allowlist is the app's part.
    pub(crate) fn answer(&self, id: &str, answer: SandboxAnswer) -> bool {
        let (waiting, reach) = {
            let mut state = self.state();
            let Some(waiting) = state.waiting.remove(id) else {
                return false;
            };
            (waiting, state.reach.clone())
        };
        if matches!(answer, SandboxAnswer::Once | SandboxAnswer::Project)
            && let Some(reach) = reach
        {
            reach.allow(&waiting.host);
        }
        // The agent gave up on the call in the meantime, which no one hears of
        let _ = waiting.answer.send(answer);
        true
    }

    /// Answer every waiting request denied
    pub(crate) fn deny_all(&self) {
        // A dropped sender is a denial to the one that waits on it
        self.state().waiting.clear();
    }

    /// The session is over: deny what waits, and ask nothing more
    pub(crate) fn close(&self) {
        let mut state = self.state();
        state.closed = true;
        state.waiting.clear();
    }

    /// Handle one `request_network` call, and give the line the agent hears
    ///
    /// # Errors
    ///
    /// What is wrong with the call, for the agent to read.
    pub(super) async fn request(&self, host: &str, why: &str) -> Result<String, String> {
        let host = host_name(host)?;
        let (id, receiver, emit) = {
            let mut state = self.state();
            let Some(reach) = state.reach.clone() else {
                return Err("this session has no sandbox, so no host is refused".to_owned());
            };
            if reach.allows(&host) {
                return Ok(format!("{host} is already allowed"));
            }
            let (Some(emit), false) = (state.turn.clone(), state.closed) else {
                return Ok(DENIED.to_owned());
            };
            state.next += 1;
            let id = format!("net-{}", state.next);
            let (answer, receiver) = oneshot::channel();
            state.waiting.insert(
                id.clone(),
                Waiting {
                    host: host.clone(),
                    answer,
                },
            );
            (id, receiver, emit)
        };
        let _gone = Forget {
            asks: self,
            id: id.clone(),
        };
        let asked = emit(SessionEvent::SandboxRequest {
            id,
            host: host.clone(),
            why: why.to_owned(),
        });
        if !asked {
            return Ok(DENIED.to_owned());
        }
        Ok(match receiver.await {
            Ok(SandboxAnswer::Once) => format!("{host} is allowed for this session"),
            Ok(SandboxAnswer::Project) => {
                format!("{host} is allowed and saved for this project")
            }
            _ => DENIED.to_owned(),
        })
    }
}

/// Takes a request off the waiting list when its call ends, answered or not
struct Forget<'a> {
    asks: &'a Asks,
    id: String,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        self.asks.state().waiting.remove(&self.id);
    }
}

/// `host` as the sandbox's network lists take it, or what is wrong with it
///
/// A name or an address, no more: a URL, a path and a port are other things, and
/// each is refused by name.
fn host_name(host: &str) -> Result<String, String> {
    let host = host.trim();
    let form = if host.contains("://") {
        Some("a URL")
    } else if host.contains(['/', '?', '#']) {
        Some("a path")
    } else if has_port(host) {
        Some("a port")
    } else {
        None
    };
    if let Some(form) = form {
        return Err(format!(
            "{host:?} is {form}. Give the host alone, a name such as docs.rs or *.docs.rs"
        ));
    }
    let host = host.to_ascii_lowercase();
    let listed = json!({
        "network": {"allowedDomains": [&host], "deniedDomains": []},
        "filesystem": {"denyRead": [], "allowWrite": [], "denyWrite": []},
    });
    Config::from_json(&listed.to_string())
        .map(|_| host.clone())
        .map_err(|error| format!("{host:?} is not a host the sandbox can list: {error}"))
}

/// Whether `host` ends in a `:port`, as a name or a bracketed address does
///
/// A bare IPv6 address has colons and no port.
fn has_port(host: &str) -> bool {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split_once("]:").is_some();
    }
    host.matches(':').count() == 1
}
