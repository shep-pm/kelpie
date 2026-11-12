//! What a pass does about the [`ReviewStep`] the order found
//!
//! The order only says what to do ([`kelpie_engine::review::step`]). Each step is carried
//! out here, and what it came to is folded back into the ticket's [`Kept`] with the
//! engine's own methods, as the gate's `act` does for its steps.
//!
//! - [`ReviewStep::Read`] and [`ReviewStep::SecondLook`] set the record to
//!   [`Status::Reviewing`] and seat it, so the read holds a slot like any turn, run the
//!   read off the poll thread, fold its outcome in with [`after_read`], and set the record
//!   back to [`Status::Ended`]. A record that cannot be seated leaves the read for the next
//!   read of GitHub, which steps a Reviewing ticket too.
//! - [`ReviewStep::FixTurn`] writes the round's findings file in the build folder and sends
//!   the fix prompt through [`crate::workers::Workers::fix_turn`], so the findings are the
//!   worker's next message and its ticket shows Building (`docs/design/review.md`, Findings).
//! - [`ReviewStep::Push`] sends the push prompt the same way.
//! - [`ReviewStep::RunTests`] runs the round's reviewer tests through the project's
//!   [`Testing`](super::Testing): at the head the reviewer read once the round's findings
//!   are in, and at the fix's head once the fix moved it. The runs are the engine's
//!   ([`review::tests_before`], [`review::tests_after`]) and are kept as the ticket's checks.
//!   A run that could not be made says so and blocks nothing; the send-back and the ruling
//!   on a test that still fails are the engine's order ([`review::after_fix`]).
//! - [`ReviewStep::Rule`] parks the worker waiting on the maintainer with the ruling's
//!   reason, as the gate's rulings are.
//! - [`ReviewStep::Ended`] hands the ticket back to the gate, which gives the merge yes.

use std::path::Path;

use kelpie_engine::agents::Binding;
use kelpie_engine::lease::SessionLease;
use kelpie_engine::review::{
    self, Finding, Folders, Kept, Lineup, ReviewStage, ReviewStep, ReviewerError, Setup, TestCheck,
    TestOutcome, TestsAsked,
};
use kelpie_engine::session::SessionEvent;
use kelpie_engine::ticket::TicketId;
use kelpie_engine::worker::{Seat, Status};
use kelpie_engine::worker_folder;

use super::{At, KEPT, Ran, Review, Standing, lock};

/// What a test the pass could not run says, when the worktree is not at the head
const NOT_AT_THE_HEAD: &str = "the worktree is not at the head the fix pushed";

/// The hand-over file the worker wrote the pull request's title and body in
///
/// [`kelpie_engine::worker::Paths::pull_request_file`] names it and keeps the name private.
const PULL_REQUEST: &str = "pull-request.md";

impl Review {
    /// Carry out `step`, folding what it came to into `kept`
    pub(super) fn carry_out(
        &self,
        at: &At<'_>,
        look: &review::Look,
        kept: &mut Kept,
        standing: &Standing,
        step: ReviewStep,
    ) -> Ran {
        match step {
            ReviewStep::Read { reviewer, .. } => {
                self.reading(at, look, kept, standing, &reviewer, None)
            }
            ReviewStep::SecondLook { reviewer, first } => {
                self.reading(at, look, kept, standing, &reviewer, Some(&first))
            }
            ReviewStep::FixTurn { prompt, findings } => self.sent(at, kept, &prompt, &findings),
            ReviewStep::Push(prompt) => self.sent(at, kept, &prompt, &[]),
            ReviewStep::RunTests => self.tests(at, look, kept, standing),
            ReviewStep::Rule(ruling) => self.ruled(at, &ruling),
            ReviewStep::Ended { unread } => self.ended(at, unread),
        }
    }

    /// One read of the ticket's pull request, in a fresh session of the reviewer `name`
    ///
    /// `first` is what the first look found, which a second look is shown.
    fn reading(
        &self,
        at: &At<'_>,
        look: &review::Look,
        kept: &mut Kept,
        standing: &Standing,
        name: &str,
        first: Option<&[Finding]>,
    ) -> Ran {
        let slots = standing.plan.slots;
        if !self.holds_slot(at.ticket, slots) {
            return Ran::Held;
        }
        let Some(binding) = self.binding(at.ticket, name, standing) else {
            self.idle(at.ticket, slots);
            return Ran::Held;
        };
        let reviewer = match (self.make)(at.ticket, &binding, &standing.plan) {
            Ok(reviewer) => reviewer,
            Err(why) => {
                eprintln!(
                    "kelpie: ticket {}'s reviewer was not started: {why}",
                    at.ticket
                );
                self.idle(at.ticket, slots);
                return Ran::Held;
            }
        };
        let prompt = self.prompt(at, look, kept, standing, &binding, first);
        let worktree = self.paths.worktree(at.ticket);
        let tests = standing
            .test_command
            .is_some()
            .then(|| review::tests_folder(&look.build));
        // The read's events go to its ticket's feed, so a tab open on the card follows it,
        // and the read is seated on the round its log will be, which is the line naming it
        // there (`docs/design/review.md`, What the maintainer sees)
        let feed = self.seated(at.ticket, name, first.is_some());
        let sink = |event: SessionEvent| feed.forward(event);
        let outcome = reviewer.read(&review::Read {
            prompt: &prompt,
            worktree: &worktree,
            tests: tests.as_deref(),
            sink: &sink,
        });
        let stopped = matches!(outcome, Err(ReviewerError::Stopped));
        let outcome = outcome.map(|found| self.checked(at, look, standing, found));
        review::after_read(kept, &standing.lineup, outcome);
        self.idle(at.ticket, slots);
        match stopped {
            true => Ran::Held,
            false => Ran::Again,
        }
    }

    /// Send the worker on `ticket` this turn: the round's findings, or the push it owes
    ///
    /// The findings file is written first, so the prompt the turn carries names a file that
    /// is already there. A turn that could not be sent at all is [`Ran::Undone`], which
    /// leaves the pass at the findings it could not send rather than at the fix turn it is
    /// waiting on: the worker is not blamed for a turn it never got (`Review::step`).
    fn sent(&self, at: &At<'_>, kept: &mut Kept, prompt: &str, findings: &[Finding]) -> Ran {
        if !findings.is_empty() {
            let round = kept.review.as_ref().map_or(1, |r| r.round);
            let build = self.paths.build_dir(at.ticket);
            if let Err(why) = review::write_findings_file(&build, round, findings) {
                eprintln!(
                    "kelpie: ticket {}'s findings were not written: {why}",
                    at.ticket
                );
                return Ran::Undone;
            }
        }
        match (self.fix)(at.ticket, prompt) {
            Ok(()) => Ran::Held,
            Err(why) => {
                eprintln!(
                    "kelpie: ticket {}'s worker was not sent its review turn: {why}",
                    at.ticket
                );
                Ran::Undone
            }
        }
    }

    /// `found` with each attached test checked and kept, for the worker's fix turn and the runs
    ///
    /// A test the reviewer attached that is not a regular file of its own folder, or that the
    /// worktree already holds a file for, loses its test and the note says why
    /// ([`review::checked_tests`]). With no Test command set no test was asked for.
    fn checked(
        &self,
        at: &At<'_>,
        look: &review::Look,
        standing: &Standing,
        found: Vec<Finding>,
    ) -> Vec<Finding> {
        let Ok(ticket) = at.ticket.parse::<TicketId>() else {
            return found;
        };
        let (worktree, worker) = (
            self.paths.worktree(at.ticket),
            self.paths.project().join(KEPT),
        );
        let mut notes = Vec::new();
        let checked = review::checked_tests(
            standing.test_command.is_some(),
            &Folders {
                ticket,
                worktree: &worktree,
                build: &look.build,
                worker: &worker,
            },
            found,
            &mut notes,
        );
        self.said(at, &notes);
        checked
    }

    /// Run the tests of the round's findings, before the fix at the head the reviewer read and
    /// after it at the fix's head, and keep each run as the ticket's check
    ///
    /// The runs are the engine's: each puts the kept test in the worktree, runs the project's
    /// Test command on it in the worker's sandbox under the `cargo-test` lease
    /// ([`super::Testing`]), and takes it out again. A run that could not be made, because the
    /// sandbox would not start or the worktree is not at the head, is kept as one that did not
    /// run, which blocks nothing.
    fn tests(&self, at: &At<'_>, look: &review::Look, kept: &mut Kept, standing: &Standing) -> Ran {
        let Some((round, stage, reading)) = kept
            .review
            .as_ref()
            .map(|r| (r.round, r.stage.clone(), r.reading.clone()))
        else {
            return Ran::Again;
        };
        let (found, after) = match stage {
            ReviewStage::Found { findings } => (findings, false),
            ReviewStage::Fixing { sent, .. } => (sent, true),
            _ => return Ran::Again,
        };
        let (Some(command), Ok(ticket)) = (
            standing.test_command.as_ref(),
            at.ticket.parse::<TicketId>(),
        ) else {
            Self::not_run(kept, round, &found, "the project has no Test command set");
            return Ran::Again;
        };
        let kit = match (self.testing.gear)(at.ticket, &standing.plan) {
            Ok(kit) => kit,
            Err(why) => {
                eprintln!(
                    "kelpie: ticket {}'s test could not be run: {why}",
                    at.ticket
                );
                Self::not_run(kept, round, &found, &why);
                return Ran::Again;
            }
        };
        let (worktree, worker) = (
            self.paths.worktree(at.ticket),
            self.paths.project().join(KEPT),
        );
        if let Err(why) = std::fs::create_dir_all(&worker) {
            let why = format!("could not make {}: {why}", worker.display());
            Self::not_run(kept, round, &found, &why);
            return Ran::Again;
        }
        let setup = Setup {
            runner: &*self.testing.runner,
            command,
            ticket,
            worktree: &worktree,
            worker: &worker,
            reach: &kit.reach,
            door: kit.lease.as_ref().map(SessionLease::door),
        };
        let mut notes = Vec::new();
        match after {
            // The fix's head is the worktree's, or there is nothing the run could say of it
            true if look.worktree.head != look.head => {
                Self::not_run(kept, round, &found, NOT_AT_THE_HEAD);
            }
            true => {
                review::tests_after(&setup, &mut kept.checks, round, &found, &mut notes);
            }
            false => {
                let head = reading.unwrap_or_else(|| look.head.clone());
                let unrun: Vec<Finding> = found
                    .iter()
                    .filter(|f| !kept.checks.iter().any(|c| c.is_for(round, f)))
                    .cloned()
                    .collect();
                let ran = review::tests_before(
                    &setup,
                    round,
                    (&head, Some(&look.worktree.head)),
                    &unrun,
                    &mut notes,
                );
                kept.checks.extend(ran);
            }
        }
        self.said(at, &notes);
        Ran::Again
    }

    /// Record each test the round's findings carry as a run that never ran, for `reason`
    ///
    /// A test with no run yet gets its run at the head the reviewer read; one whose fix moved
    /// the head gets the run after it. Each reads [`review::Reading::Unknown`], which holds
    /// nothing.
    fn not_run(kept: &mut Kept, round: u32, sent: &[Finding], reason: &str) {
        for finding in sent.iter().filter(|f| f.test.is_some()) {
            let not_run = || TestOutcome::NotRun {
                reason: reason.to_owned(),
            };
            // `rposition` is the last match, counted where the checks stand: an earlier
            // round's check is a different finding's, and setting its run again would leave
            // this round's waiting forever
            match (kept.checks.iter()).rposition(|c| c.is_for(round, finding)) {
                None => kept.checks.push(TestCheck {
                    file: finding.file.clone(),
                    line: finding.line,
                    what: finding.what.clone(),
                    round,
                    before: not_run(),
                    after: None,
                    sent_back: false,
                }),
                Some(at) if kept.checks[at].after.is_none() => {
                    kept.checks[at].after = Some(not_run());
                }
                Some(_) => {}
            }
        }
    }

    /// What the engine's test functions had to say, named to the maintainer on stderr
    fn said(&self, at: &At<'_>, notes: &[String]) {
        for note in notes {
            eprintln!("kelpie: ticket {}: {note}", at.ticket);
        }
    }

    /// Park the worker waiting on the maintainer with the ruling's reason, which holds no slot
    ///
    /// The gate records the ruling too, so the ticket's line is the reason, it survives a
    /// relaunch, and the ticket gets no pass until its head moves (`gate::act`).
    fn ruled(&self, at: &At<'_>, ruling: &review::Ruling) -> Ran {
        let Some(records) = self.records() else {
            return Ran::Held;
        };
        let reason = ruling.reason();
        let mut records = lock(&records);
        let written = records
            .set_status(at.ticket, Status::WaitingOnMaintainer)
            .and_then(|()| records.set_reason(at.ticket, Some(reason.clone())));
        drop(records);
        if let Err(why) = written {
            eprintln!(
                "kelpie: project {}: the record of ticket {} did not keep the review's ruling: {why}",
                self.project_id, at.ticket
            );
            return Ran::Held;
        }
        eprintln!("kelpie: ticket {}'s review ruled: {reason}", at.ticket);
        Ran::Ruled(ruling.clone())
    }

    /// The pass is over: the gate's order runs again, which gives the merge yes
    ///
    /// What the yes warns of, a head no reviewer read or a pass that ended short, is named
    /// on stderr until the merge yes can show it (`docs/design/review.md`, The unread head).
    fn ended(&self, at: &At<'_>, unread: Option<review::Unread>) -> Ran {
        if let Some(unread) = unread {
            eprintln!("kelpie: ticket {}: {}", at.ticket, unread.text());
        }
        Ran::Again
    }

    /// Set `ticket`'s record Reviewing and seat it, and say whether it holds a slot
    ///
    /// A read is a turn of the project's, so it takes one of its slots. With none free the
    /// record waits for one, and the read is made on the next read of GitHub.
    fn holds_slot(&self, ticket: &str, slots: usize) -> bool {
        let Some(records) = self.records() else {
            return false;
        };
        let mut records = lock(&records);
        if records.get(ticket).is_none() {
            return false;
        }
        let seated = records
            .set_status(ticket, Status::Reviewing)
            .and_then(|()| records.seat(slots));
        if let Err(why) = seated {
            eprintln!("kelpie: ticket {ticket} was not set to reviewing: {why}");
            return false;
        }
        records
            .get(ticket)
            .is_some_and(|record| record.seat == Seat::Held)
    }

    /// Set `ticket`'s record back to Ended, which gives its slot back
    fn idle(&self, ticket: &str, slots: usize) {
        let Some(records) = self.records() else {
            return;
        };
        let mut records = lock(&records);
        let written = records
            .set_status(ticket, Status::Ended)
            .and_then(|()| records.seat(slots));
        drop(records);
        if let Err(why) = written {
            eprintln!("kelpie: ticket {ticket} was not ended after its read: {why}");
        }
    }

    /// What `name` reads on: its agent file, then the project's Reviewer harness, model and effort
    ///
    /// `name` is as the project lists it, which is not always the file it is read for.
    fn binding(&self, ticket: &str, name: &str, standing: &Standing) -> Option<Binding> {
        let binding = standing
            .agents
            .reviewer_binding(Lineup::file(name), &standing.settings)
            .map_err(|error| error.to_string());
        match binding {
            Ok(binding) => Some(binding),
            Err(why) => {
                eprintln!(
                    "kelpie: ticket {ticket} has no reviewer to read it: {name} cannot run: {why}"
                );
                None
            }
        }
    }

    /// What the read's session is asked: its agent's own words over the diff, the issue's
    /// acceptance criteria, the pull request's body, what earlier readers were disputed, and
    /// on a second look what the first found
    fn prompt(
        &self,
        at: &At<'_>,
        look: &review::Look,
        kept: &Kept,
        standing: &Standing,
        binding: &Binding,
        first: Option<&[Finding]>,
    ) -> String {
        let worktree = self.paths.worktree(at.ticket);
        let base = format!("origin/{}", crate::workers::BASE);
        let diff = review::diff_against(&self.tree, &self.folder, &worktree, &base).unwrap_or_else(
            |error| {
                eprintln!("kelpie: ticket {}'s diff was not read: {error}", at.ticket);
                String::new()
            },
        );
        let criteria = self.criteria(at.ticket);
        let tests_folder = review::tests_folder(&look.build);
        let tests = standing.test_command.as_ref().map(|command| TestsAsked {
            command: command.as_str(),
            folder: &tests_folder,
            tool: review::write_tool(standing.settings.reviewer_harness.value),
        });
        let mut prompt = review::reviewer_prompt(
            binding.body.value.as_deref().unwrap_or_default(),
            &base,
            &diff,
            &criteria,
            first,
            tests,
        );
        prompt.push_str(&review::body_note(&self.body(at.ticket)));
        prompt.push_str(&review::disputes_note(&kept.disputes));
        prompt
    }

    /// The issue's acceptance criteria, which a reviewer checks the diff against
    fn criteria(&self, ticket: &str) -> String {
        let Ok(id) = ticket.parse() else {
            return String::new();
        };
        match self.tickets.issue(&id) {
            Ok(issue) => review::acceptance(&issue.body),
            Err(why) => {
                eprintln!(
                    "kelpie: ticket {ticket}'s issue was not read, so its criteria are not given to the reviewer: {why}"
                );
                String::new()
            }
        }
    }

    /// The pull request's body, as the worker's hand-over file wrote it
    fn body(&self, ticket: &str) -> String {
        let state = self.paths.state_dir(ticket);
        match worker_folder::read(&state, Path::new(PULL_REQUEST)) {
            Ok(Some(text)) => text,
            Ok(None) => String::new(),
            Err(why) => {
                eprintln!("kelpie: ticket {ticket}'s pull request body was not read: {why}");
                String::new()
            }
        }
    }
}
