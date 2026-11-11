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
//! Paused worker is resumed, not told: a Note to it waits in its record for its next start.
//! A Note to a worker waiting on the maintainer answers its ruling and sends it back
//! ([`Workers::fix_turn`]).
//!
//! [`Workers::answer`] answers the worker's own sandbox request through the session kept for
//! it, and what the answer did is said the way the PM's is ([`crate::pm::pm_answer`]).

use std::io;
use std::path::Path;
use std::sync::Arc;

use kelpie_engine::session::{SandboxAnswer, SandboxOutcome, Session};
use kelpie_engine::ticket::Tickets;
use kelpie_engine::tools::Tools;
use kelpie_engine::worker::{self, Noted, Opening, Status};
use serde::Serialize;
use specta::Type;

use crate::pm::answered;
use crate::state::AppState;

use super::{Opener, Workers, lock, open, ticket_id};

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
pub(super) fn outcome(noted: Noted) -> NoteOutcome {
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
    pub(super) fn steered<T>(
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
    /// Two workers have no live session, and a Note reaches each one anyway. A worker
    /// waiting on a ruling is sent back at once: the Note is the maintainer's answer, so
    /// the worker is set Ended, its reason is cleared, and the text goes out as a fix turn
    /// ([`Workers::fix_turn`]). A Paused worker keeps the text in its record for its next
    /// start, which is the maintainer's to say.
    ///
    /// `source` and `open` are for the fix turn's start, as [`Workers::resume`] takes them.
    ///
    /// # Errors
    ///
    /// As `Workers::steered`, plus what the records write answers. A fix turn that cannot
    /// start leaves the worker waiting on its ruling, with the text kept in its record.
    #[expect(
        clippy::too_many_arguments,
        reason = "the gate's and a Note's fix turn name the ticket, the prompt, where a ticket's text is read, and how a session is opened"
    )]
    pub fn note(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
        text: &str,
        source: Result<&dyn Tickets, String>,
        open: &Opener<'_>,
    ) -> Result<NoteOutcome, String> {
        let id = ticket_id(ticket)?;
        match self.status_of(project_id, &id) {
            Ok(Status::WaitingOnMaintainer) => {
                self.send_back(project_id, folder, tools, &id, text, source, open)
            }
            Ok(Status::Paused) => self.keep_text(project_id, &id, text),
            _ => {
                let noted = self.steered(
                    project_id,
                    folder,
                    tools,
                    ticket,
                    |project, session, ticket| worker::note(project, session, ticket, text),
                )?;
                Ok(outcome(noted))
            }
        }
    }

    /// Answer the ruling the worker on `ticket` waits on with `text`, by sending it back
    ///
    /// The ruling is over once the worker is Ended with no reason. A fix turn that fails
    /// puts the ruling back, so the ticket still says what it waits on.
    #[expect(
        clippy::too_many_arguments,
        reason = "the gate's and a Note's fix turn name the ticket, the prompt, where a ticket's text is read, and how a session is opened"
    )]
    fn send_back(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
        text: &str,
        source: Result<&dyn Tickets, String>,
        open: &Opener<'_>,
    ) -> Result<NoteOutcome, String> {
        let records = self.records(project_id)?;
        let reason = {
            let mut records = lock(&records);
            let reason = records.get(ticket).and_then(|record| record.reason.clone());
            records
                .set_status(ticket, Status::Ended)
                .and_then(|()| records.set_reason(ticket, None))
                .map_err(|error| error.to_string())?;
            reason
        };
        let sent = self.fix_turn(project_id, folder, tools, ticket, text, source, open);
        if sent.is_err() {
            let mut records = lock(&records);
            // A start that parked the worker Paused, as a refused sandbox does, has said
            // why in its reason, which a ruling put back would hide
            if records.get(ticket).map(|record| record.status) != Some(Status::Ended) {
                return sent;
            }
            let back = records
                .set_status(ticket, Status::WaitingOnMaintainer)
                .and_then(|()| records.set_reason(ticket, reason));
            if let Err(error) = back {
                eprintln!("kelpie: the ruling of ticket {ticket} was not put back: {error}");
            }
        }
        sent
    }

    /// Keep `text` in the record of the Paused worker on `ticket`, for its next start
    fn keep_text(&self, project_id: &str, ticket: &str, text: &str) -> Result<NoteOutcome, String> {
        let records = self.records(project_id)?;
        lock(&records)
            .add_note(ticket, text)
            .map_err(|error| error.to_string())?;
        Ok(NoteOutcome::Queued)
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
    let project = state.project_id(None)?;
    let source = state.source(&project);
    let source = source.as_ref().map(|source| &**source as &dyn Tickets);
    note_through(
        state,
        ticket,
        text,
        source.map_err(Clone::clone),
        &open::open,
    )
}

/// Tell the project's worker on `ticket` `text`, opening the session of a fix turn with `open`
///
/// What [`note_on`] does with the app's own opener, which a test replaces.
///
/// # Errors
///
/// As [`note_on`].
pub fn note_through(
    state: &AppState,
    ticket: &str,
    text: &str,
    source: Result<&dyn Tickets, String>,
    open: &Opener<'_>,
) -> Result<NoteOutcome, String> {
    on_project(state, |workers, project, folder, tools| {
        workers.note(project, folder, tools, ticket, text, source, open)
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
