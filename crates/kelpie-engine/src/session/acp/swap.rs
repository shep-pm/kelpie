//! Which bridge version one try of a session runs on, and who is answerable
//! when it goes wrong
//!
//! A session on a bridge asks [`crate::session::Bridge::current`] which version
//! to start on, so a newer one takes over at the session's next idle point. Only
//! a version the session just swapped to is the bridge's failure: a harness that
//! is missing, has no login, or is too old for the bridge is the harness's, and
//! ends the turn with what it said, marking nothing bad
//! (`docs/design/bridge-updates.md`, Flow and Scope).

use agent_client_protocol::Error;
use semver::Version;

use crate::harness::HarnessError;
use crate::session::{Harness, Launch, Link, LogEntry, Setup, Source, Started, Swap, lock};

use super::describe;

/// How one try at a session ended
pub(super) enum Done {
    /// The harness ended the connection, or the session was dropped
    Over,
    /// A message waits for the newer bridge version `swap` names, which the next start runs on
    Moved {
        /// What the idle look answered, so the next start does not ask again and
        /// still knows which version it swapped from
        swap: Swap,
    },
    /// The session could not be opened on the version it started
    OpenFailed {
        /// Why, and what to tell the maintainer
        fault: Fault,
        /// What the version failed to do, which the divider says
        why: &'static str,
    },
}

/// One try at a session, on one bridge version
pub(super) struct Attempt {
    /// What to run, and what its harness offers
    pub(super) harness: Harness,
    /// The bridge version it runs on, `None` for a harness kelpie does not move
    pub(super) version: Option<Version>,
    /// The version this try swapped from, which the divider names and whose
    /// failure a failed start or open is
    pub(super) from: Option<Version>,
    /// The value of the harness's `mode` option to set, `None` to leave it
    ///
    /// The session's own, or the one the started harness names when what
    /// launched it runs in a sandbox, so each bridge version is judged by its
    /// own launcher.
    pub(super) mode: Option<String>,
}

/// Start the agent for one try, and say what its harness offers
///
/// A bridge session runs on `swap` when its last idle look already offered one,
/// and otherwise asks [`crate::session::Bridge::current`], which is how a newer
/// version takes over at the session's next idle point.
///
/// # Errors
///
/// [`DidNotStart`] when the harness for that version could not be built or would
/// not spawn.
pub(super) fn start(setup: &Setup, swap: Option<Swap>) -> Result<(Attempt, Link), DidNotStart> {
    match &setup.source {
        Source::Fixed { harness, launcher } => {
            let mut launcher = lock(launcher);
            bind_network(setup, &**launcher);
            let link = launcher.launch().map_err(|error| DidNotStart {
                version: None,
                from: None,
                message: format!("could not start the harness: {error}"),
            })?;
            Ok((
                Attempt {
                    harness: (**harness).clone(),
                    version: None,
                    from: None,
                    mode: mode_of(setup, &**launcher, harness),
                },
                link,
            ))
        }
        Source::Bridge { bridge } => {
            let Swap { version, from } = swap.unwrap_or_else(|| (bridge.current)());
            let not_started = |message| DidNotStart {
                version: Some(version.clone()),
                from: from.clone(),
                message,
            };
            let Started { harness, launch } = (bridge.start)(&version).map_err(not_started)?;
            let mut launch = launch;
            bind_network(setup, &*launch);
            let mode = mode_of(setup, &*launch, &harness);
            let Link {
                transport,
                explain,
                stop,
            } = launch
                .launch()
                .map_err(|error| not_started(format!("could not start the harness: {error}")))?;
            // What launched the agent owns what confines it, such as a sandbox
            // whose proxy stops with it, so it stays until the agent has been
            // stopped, not until this function returns (ADR 0005)
            let link = Link {
                transport,
                explain,
                stop: Box::new(move |grace| {
                    let how = stop(grace);
                    drop(launch);
                    how
                }),
            };
            Ok((
                Attempt {
                    harness,
                    from,
                    version: Some(version),
                    mode,
                },
                link,
            ))
        }
    }
}

/// Hand the session's `request_network` calls the sandbox `launch` runs the agent in
fn bind_network(setup: &Setup, launch: &dyn Launch) {
    if let Some(network) = launch.network() {
        setup.asks.bind(network);
    }
}

/// The `mode` option to set on `harness`
///
/// The session's own, else the mode that makes a judged session's harness ask,
/// else the one that stops its prompts when `launch` runs it in a sandbox. A
/// judged session never stops them: the judge would see nothing.
fn mode_of(setup: &Setup, launch: &dyn Launch, harness: &Harness) -> Option<String> {
    setup.mode.clone().or_else(|| {
        if setup.judge.is_some() {
            harness.prompting.clone()
        } else {
            unprompted(launch, harness)
        }
    })
}

/// The mode that stops `harness`'s own prompts, when `launch` runs it in a sandbox
fn unprompted(launch: &dyn Launch, harness: &Harness) -> Option<String> {
    launch
        .sandboxed()
        .then(|| harness.unprompted.clone())
        .flatten()
}

/// A try whose harness could not be started at all
pub(super) struct DidNotStart {
    /// The bridge version it tried, `None` for a harness kelpie does not move
    pub(super) version: Option<Version>,
    /// The version it swapped from, `None` when nothing swapped, which is what
    /// makes this the harness's problem rather than the bridge version's
    pub(super) from: Option<Version>,
    /// What to tell the maintainer
    pub(super) message: String,
}

/// The version to go back to after the session failed on `failed` with `message`
///
/// `None` when there is no version before this one. Marks the version bad, puts
/// the one before it back in use, and logs the divider that says so. Called only
/// for a version the session just swapped to, which is the only kind of failure
/// a bridge is answerable for (`docs/design/bridge-updates.md`, Flow).
pub(super) fn fell_back(
    setup: &Setup,
    failed: &Version,
    why: &str,
    message: &str,
) -> Option<Version> {
    let Source::Bridge { bridge } = &setup.source else {
        return None;
    };
    let back = (bridge.failed)(failed, message)?;
    *lock(&setup.running) = Some(back.clone());
    if let Some(log) = &setup.log {
        let text = format!("{bridge} {failed} failed to {why}, back on {back}");
        let _ = log.append(&LogEntry::Divider { text });
    }
    Some(back)
}

/// The bridge version this session should swap to, `None` when the one it runs on stands
pub(super) fn moves(setup: &Setup, attempt: &Attempt) -> Option<Swap> {
    let Source::Bridge { bridge } = &setup.source else {
        return None;
    };
    let now = attempt.version.as_ref()?;
    let swap = (bridge.current)();
    (swap.version != *now).then_some(swap)
}

/// The divider that says the session moved to a newer bridge version
pub(super) fn moved(setup: &Setup, from: &Version, to: &Version) {
    let (Some(log), Source::Bridge { bridge }) = (&setup.log, &setup.source) else {
        return;
    };
    let text = format!("Updated {bridge} from {from} to {to}");
    let _ = log.append(&LogEntry::Divider { text });
}

/// Why a try went wrong, and whether the bridge version it ran on is answerable for it
#[derive(Debug)]
pub(super) enum Fault {
    /// This bridge version is what failed, which is what a swap goes back from
    Bridge {
        /// What to tell the maintainer
        message: String,
    },

    /// What failed is not the bridge version's doing: the harness behind it is
    /// too old, which only the person can fix
    /// (`docs/design/bridge-updates.md`, Scope), or a request kelpie itself got
    /// wrong. No version is marked bad for it.
    NotTheBridge {
        /// What to tell the maintainer, naming the harness's own update command
        /// when that is the problem
        message: String,
    },
}

impl Fault {
    /// What the maintainer reads
    pub(super) fn message(&self) -> &str {
        match self {
            Self::Bridge { message } | Self::NotTheBridge { message } => message,
        }
    }
}

/// Why `error` went wrong, saying so as [`HarnessError::TooOld`] with the
/// harness's own update command when its bridge says the harness behind it is
/// too old
pub(super) fn fault(attempt: &Attempt, error: &Error) -> Fault {
    let said = describe(error);
    let Some(outdated) = &attempt.harness.outdated else {
        return Fault::Bridge { message: said };
    };
    let lower = said.to_lowercase();
    if !outdated
        .says
        .iter()
        .any(|phrase| lower.contains(phrase.to_lowercase().as_str()))
    {
        return Fault::Bridge { message: said };
    }
    Fault::NotTheBridge {
        message: HarnessError::TooOld {
            harness: outdated.harness.clone(),
            bridge: said,
            update: outdated.update.clone(),
        }
        .to_string(),
    }
}
