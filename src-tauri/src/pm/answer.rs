//! What an answer to a sandbox request did, and what a *Allow for this project* kept
//!
//! The PM's [`crate::pm::pm_answer`] and a worker's [`crate::workers::worker_answer`] both
//! end here, so the same answer is kept, refused and said the same way whichever tab it was
//! clicked in (`docs/design/kelpie-tools.md`, The sandbox request).

use kelpie_engine::session::{SandboxAnswer, SandboxOutcome};
use kelpie_engine::settings::{Field, LayerFile, Settings, SettingsError};
use serde_json::Value;

use crate::state::AppState;

/// What an answer did, said to the maintainer, and what a *Allow for this project* kept
///
/// Every session's answer ends here, whichever session it was for, so a worker's tab and
/// the PM's chat keep, refuse and say the same about the same answer
/// ([`crate::workers::worker_answer`]).
///
/// # Errors
///
/// The running sandbox could not take the host, which saves nothing, or the host could not
/// be kept to the project, in which case it stays allowed for the session alone.
pub(crate) fn answered(
    state: &AppState,
    outcome: SandboxOutcome,
    answer: SandboxAnswer,
) -> Result<(), String> {
    match outcome {
        SandboxOutcome::Allowed { host } => match answer {
            // Only a kept answer has a place outside the session running now
            SandboxAnswer::Project => allow_for_the_project(state, &host),
            _ => Ok(()),
        },
        // A denial has nothing to keep
        SandboxOutcome::Denied { .. } => Ok(()),
        // The maintainer said yes and the sandbox could not take it, so nothing
        // was widened: the project is not told a host its sessions cannot reach
        SandboxOutcome::NotWidened { host } => Err(not_through(&host)),
    }
}

/// Add `host` to the project's own `network-allowlist`
///
/// A project's file replaces the global list rather than adding to it, so what
/// the PM may already reach is written into it along with `host`. The resolve
/// and the write are one step ([`Settings::update`]), since two answers can be
/// on their way at once and each adds one host to the same list.
///
/// # Errors
///
/// No project is picked, or its settings file cannot be read or written.
fn allow_for_the_project(state: &AppState, host: &str) -> Result<(), String> {
    let id = state.project_id(None)?;
    Settings::update(
        state.config(),
        LayerFile::Project(&id),
        Field::NetworkAllowlist,
        |resolved| {
            let mut hosts = resolved.network_allowlist.value.clone();
            if !hosts.iter().any(|allowed| allowed == host) {
                hosts.push(host.to_owned());
            }
            Value::Array(hosts.into_iter().map(Value::String).collect())
        },
    )
    .map_err(|error| not_saved(host, error))
}

/// Why `host` never reached the sandbox at all, said to the maintainer
///
/// The agent was told the same in its own line. Nothing is saved for the project,
/// so the settings page never lists a host its sessions cannot reach.
fn not_through(host: &str) -> String {
    format!("{host} is not through this session's sandbox, so kelpie saved nothing for it")
}

/// Why `host` could not be kept, said to the maintainer
///
/// The host is through the running sandbox already, so the answer took effect;
/// only the saving did not. The chat shows this on the request's own line.
fn not_saved(host: &str, error: SettingsError) -> String {
    format!(
        "{host} is allowed for this session, but kelpie could not save it to the project: {error}"
    )
}
