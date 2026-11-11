//! Telling a worker what to do: a Note, a Stop and tell, a Stop, and an answer
//!
//! A worker's tab steers the worker it shows (`docs/design/tickets.md`, Use case, and
//! `docs/design/system/components/PMPanel`): [`Workers::note`] keeps a Note in the ticket's
//! record while its turn runs, [`Workers::stop_and_tell`] cancels the turn and sends the
//! text as the turn that follows, and [`Workers::stop`] cancels it and leaves the worker
//! Paused. Each is the engine's own move (`crates/kelpie-engine/src/worker/tell.rs`), and
//! each refuses a ticket with no live session by name. kelpie keeps a worker's session
//! while its turn runs and after it ends with the work committed, which is how a Note
//! between turns reaches the engine's idle path and becomes that worker's next turn. A
//! Paused worker, or one waiting on the maintainer, is resumed, not told.
//!
//! [`Workers::answer`] answers the worker's own sandbox request through the session kept for
//! it, and what the answer did is said the way the PM's is ([`crate::pm::pm_answer`]).

use std::io;
use std::path::Path;
use std::sync::Arc;

use kelpie_engine::session::{SandboxAnswer, SandboxOutcome, Session};
use kelpie_engine::tools::Tools;
use kelpie_engine::worker::{self, Noted, Opening};
use serde::Serialize;
use specta::Type;

use crate::pm::answered;
use crate::state::AppState;

use super::{Workers, ticket_id};

/// What a Note, or a Stop and tell, became
///
/// `queued` is the text kept in the ticket's record for the worker's next prompt, and
/// `sent` is the text the worker has been given as a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum NoteOutcome {
    /// The text waits in the ticket's record and goes out with the worker's next prompt
    Queued,

    /// The text went to the worker as a turn
    Sent,
}

/// What the engine made of a telling, in the words the window reads
///
/// [`Noted`] is `#[non_exhaustive]`: a state it grows is read as `queued`, since what the
/// maintainer asks is only whether the worker has the text yet.
fn outcome(noted: Noted) -> NoteOutcome {
    match noted {
        Noted::Sent => NoteOutcome::Sent,
        _ => NoteOutcome::Queued,
    }
}

/// Why a ticket with no live session is refused, naming it
fn no_session(ticket: &str) -> String {
    format!("ticket {ticket} has no live session, so kelpie cannot tell it anything")
}

impl Workers {
    /// Run `then` on the worker on `ticket`, naming it as its record does
    ///
    /// `then` steers the engine's [`worker::Project`] of `project_id` and the session kept
    /// for that worker. A steering move starts no session of its own: the kept one is the
    /// one it tells.
    ///
    /// # Errors
    ///
    /// `ticket` is not an issue number, the project's settings or records cannot be read,
    /// or its worker on `ticket` has no live session. What the engine says comes back as it
    /// says it.
    fn steered<T>(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
        then: impl FnOnce(&worker::Project<'_>, &Arc<Session>, &str) -> io::Result<T>,
    ) -> Result<T, String> {
        let ticket = ticket_id(ticket)?;
        let standing = self.standing(project_id)?;
        let session = self
            .session(project_id, &ticket)
            .ok_or_else(|| no_session(&ticket))?;
        let starts_nothing = |_: &Opening<'_>| -> Result<Session, String> {
            Err("a tell starts no session".to_owned())
        };
        let project = self.project_of(&standing, project_id, folder, tools, &starts_nothing)?;
        then(&project, &session, &ticket).map_err(|error| error.to_string())
    }

    /// Tell the worker on `ticket` `text` without interrupting it
    ///
    /// What the engine makes of it is [`kelpie_engine::worker::note`]: kept in the ticket's
    /// record while its turn runs, and sent as its next turn once its turn has ended.
    ///
    /// # Errors
    ///
    /// As `Workers::steered`, plus what the records write answers.
    pub fn note(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
        text: &str,
    ) -> Result<NoteOutcome, String> {
        let noted = self.steered(
            project_id,
            folder,
            tools,
            ticket,
            |project, session, ticket| worker::note(project, session, ticket, text),
        )?;
        Ok(outcome(noted))
    }

    /// Cancel the worker on `ticket`'s turn and send it `text` as the next prompt
    ///
    /// What the engine makes of it is [`kelpie_engine::worker::stop_and_tell`]: the worker
    /// stays Running and keeps its slot, and the text is the turn that follows the one it
    /// cancelled.
    ///
    /// # Errors
    ///
    /// As [`Workers::note`].
    pub fn stop_and_tell(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
        text: &str,
    ) -> Result<NoteOutcome, String> {
        let noted = self.steered(
            project_id,
            folder,
            tools,
            ticket,
            |project, session, ticket| worker::stop_and_tell(project, session, ticket, text),
        )?;
        Ok(outcome(noted))
    }

    /// Cancel the worker on `ticket`'s turn and leave it Paused
    ///
    /// What the engine does is [`kelpie_engine::worker::stop`]: the slot goes to the next in
    /// line, and the maintainer resumes the worker by hand.
    ///
    /// # Errors
    ///
    /// As `Workers::steered`, plus what the records write answers. The turn is cancelled
    /// whichever way the write answers.
    pub fn stop(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
    ) -> Result<(), String> {
        self.steered(project_id, folder, tools, ticket, worker::stop)
    }

    /// Answer the sandbox request `id` of the worker on `ticket`, and say what it did
    ///
    /// The worker's kept session answers it, which is what lets the host through the sandbox
    /// its turn is still running in. The request goes out of the worker's [`crate::workers::Feed`] as soon
    /// as it is answered, so the board stops marking the ticket as needing the maintainer.
    /// What the answer did is [`SandboxOutcome`], which the app answers as
    /// [`crate::pm::pm_answer`] does.
    ///
    /// # Errors
    ///
    /// `ticket` is not an issue number, or its worker has no live session. No request was
    /// waiting for `id` is not an error here: the session says so with `None`.
    pub fn answer(
        &self,
        project_id: &str,
        ticket: &str,
        id: &str,
        answer: SandboxAnswer,
    ) -> Result<Option<SandboxOutcome>, String> {
        let ticket = ticket_id(ticket)?;
        let session = self
            .session(project_id, &ticket)
            .ok_or_else(|| no_session(&ticket))?;
        let outcome = session.answer(id, answer);
        if outcome.is_some() {
            self.feed(project_id, &ticket).answered(id);
        }
        Ok(outcome)
    }
}

/// Run `then` on the project the window is on, and its worker on `ticket`
///
/// `folder` is the project's checkout, which each worker's worktree is cut from.
///
/// # Errors
///
/// No project is picked, or kelpie has no checkout for it.
fn on_project<T>(
    state: &AppState,
    then: impl FnOnce(&Workers, &str, &Path, Option<&Tools>) -> Result<T, String>,
) -> Result<T, String> {
    let project = state.project_id(None)?;
    let folder = state
        .project_path_of(&project)
        .ok_or_else(|| format!("kelpie has no project {project}"))?;
    then(state.workers(), &project, &folder, state.tools())
}

/// Tell the project's worker on `ticket` `text` without interrupting it
///
/// # Errors
///
/// As [`Workers::note`], plus a `ticket` that is not an issue number and a project with no
/// checkout.
pub fn note_on(state: &AppState, ticket: &str, text: &str) -> Result<NoteOutcome, String> {
    on_project(state, |workers, project, folder, tools| {
        workers.note(project, folder, tools, ticket, text)
    })
}

/// Cancel the project's worker on `ticket`'s turn and send it `text` as its next prompt
///
/// # Errors
///
/// As [`Workers::stop_and_tell`].
pub fn stop_and_tell_on(state: &AppState, ticket: &str, text: &str) -> Result<NoteOutcome, String> {
    on_project(state, |workers, project, folder, tools| {
        workers.stop_and_tell(project, folder, tools, ticket, text)
    })
}

/// Cancel the project's worker on `ticket`'s turn and leave it Paused
///
/// # Errors
///
/// As [`Workers::stop`].
pub fn cancel_on(state: &AppState, ticket: &str) -> Result<(), String> {
    on_project(state, |workers, project, folder, tools| {
        workers.stop(project, folder, tools, ticket)
    })
}

/// Answer the sandbox request `id` of the project's worker on `ticket`
///
/// As [`crate::pm::pm_answer`] answers the PM's, a *Allow for this project* kept to the
/// project's `network-allowlist`, which is what the worker's next start builds into its
/// sandbox.
///
/// # Errors
///
/// No project is picked, the ticket's worker has no live session, no request was waiting for
/// `id`, the running sandbox could not take the host, or the host could not be kept to the
/// project. The text is for the maintainer to read.
pub fn answer_on(
    state: &AppState,
    ticket: &str,
    id: &str,
    answer: SandboxAnswer,
) -> Result<(), String> {
    let project = state.project_id(None)?;
    let outcome = state
        .workers()
        .answer(&project, ticket, id, answer)?
        .ok_or_else(|| "That request is over.".to_owned())?;
    answered(state, outcome, answer)
}
