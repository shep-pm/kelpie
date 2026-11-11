//! A fix turn: the prompt that sends a worker back to its own session
//!
//! The gate sends red CI and a conflict back as a turn of the same session
//! (`docs/design/gate-and-merge.md`, Red CI and Ports), and a Note answers a ruling the
//! same way ([`Workers::note`]). The turn is a prompt over ACP to the worker's own session,
//! so it touches no adapter and reaches every harness alike.
//!
//! With the session live, [`Workers::fix_turn`] is the engine's [`worker::note`]: a worker
//! whose turn ended takes a slot, or waits in line for one, and is sent the prompt as its
//! next turn. With none, as after a relaunch, the prompt joins the record's notes and
//! [`Workers::start`] resumes the session, whose first prompt carries them. Either way the
//! turn starts from what the worktree holds, and the worker shows Running, so its ticket is
//! in Building until the turn ends.

use std::path::Path;

use kelpie_engine::ticket::Tickets;
use kelpie_engine::tools::Tools;
use kelpie_engine::worker::{self, Seat, Status, Ticket};

use super::tell::NoteOutcome;
use super::{CARRY_ON, Opener, Workers, lock, ticket_id, ticket_of};

impl Workers {
    /// Send the worker on `ticket` back with `prompt`, as the turn that follows its last
    ///
    /// `Sent` is a turn that began, and `Queued` is a prompt kept in the ticket's record
    /// for the turn a slot frees up: the worker is in line, and asking again once one is
    /// free starts it. Asking again keeps the prompt once. `source` is read only for a
    /// worker that never had a turn, as [`Workers::resume`] does, and `open` builds the
    /// session of one that has none live.
    ///
    /// # Errors
    ///
    /// `ticket` is not an issue number, the project has no worker on it, or its records
    /// cannot be written. Or the session cannot be started (see [`Workers::start`]). A
    /// prompt kept before a start that failed stays in the record for the next one.
    #[expect(
        clippy::too_many_arguments,
        reason = "the gate's and a Note's fix turn name the ticket, the prompt, where a ticket's text is read, and how a session is opened"
    )]
    pub fn fix_turn(
        &self,
        project_id: &str,
        folder: &Path,
        tools: Option<&Tools>,
        ticket: &str,
        prompt: &str,
        source: Result<&dyn Tickets, String>,
        open: &Opener<'_>,
    ) -> Result<NoteOutcome, String> {
        let ticket = ticket_id(ticket)?;
        if self.session(project_id, &ticket).is_some() {
            let noted = self.steered(
                project_id,
                folder,
                tools,
                &ticket,
                |project, session, ticket| worker::note(project, session, ticket, prompt),
            )?;
            return Ok(super::tell::outcome(noted));
        }
        self.keep_prompt(project_id, &ticket, prompt)?;
        let ticket = if self.has_session(project_id, &ticket)? {
            Ticket {
                id: ticket,
                text: CARRY_ON.to_owned(),
            }
        } else {
            ticket_of(source?, &ticket)?
        };
        let started = self.start(project_id, folder, tools, &ticket, open)?;
        Ok(if started.waiting {
            NoteOutcome::Queued
        } else {
            NoteOutcome::Sent
        })
    }

    /// Add `prompt` to the notes of the worker on `ticket`, which its next prompt carries
    ///
    /// A worker already in line for a slot holds the prompt from the ask that put it
    /// there, so it is not added twice.
    fn keep_prompt(&self, project_id: &str, ticket: &str, prompt: &str) -> Result<(), String> {
        let records = self.records(project_id)?;
        let mut records = lock(&records);
        let record = records
            .get(ticket)
            .ok_or_else(|| format!("there is no worker on ticket {ticket}"))?;
        let in_line = record.status == Status::Running && record.seat == Seat::Waiting;
        if in_line && record.notes.iter().any(|note| note == prompt) {
            return Ok(());
        }
        records
            .add_note(ticket, prompt)
            .map(drop)
            .map_err(|error| error.to_string())
    }
}
