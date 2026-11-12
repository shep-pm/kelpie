//! What a review pass does next, from a read to a fix turn to its end
//!
//! [`step`] reads where a ticket's pass stands in [`Kept`] and what the caller just read of
//! the pull request ([`Look`]), and answers with the one thing to do, in the order of
//! `docs/design/review.md`, The pass: push, read, second look, fix turn, run the tests, rule,
//! or end. It touches no git, GitHub or harness. The caller does the act and folds a read's
//! outcome in with [`after_read`]; [`after_push`] and [`after_fix`] fold the outcome of the
//! worker's turns, and [`step`] calls them when the worker is idle again, so a restart in the
//! middle of one steps the same way. Nothing runs the step yet.
//!
//! A read in flight at a restart is cleared by the restart: the kept round still names its
//! reviewer, and the next step reads again with the same one.

use std::path::PathBuf;

use super::checks::{Next, after_the_fix, round_checks, still_failing};
use super::disputes::{Answers, Dispute, answers, findings_of, mark_shown, record};
use super::findings_file::still_failing_prompt;
use super::findings_file::{FailingTest, Sent, deferred, findings_path, fix_prompt};
use super::lineup::RULE_READS;
use super::{
    Finding, Kept, Lineup, Review, ReviewStage, ReviewerError, Short, TestCheck, TestOutcome,
    Unread, both_looks, unread_of,
};
use crate::gate::short;
use crate::github::Timestamp;

/// Where the ticket's worktree stands, as the last read of it found
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    /// The commit it has checked out
    pub head: String,
    /// Whether it holds no change that is not committed
    pub clean: bool,
}

/// What a step read of the pull request and the worktree
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    /// The pull request's number
    pub number: u64,
    /// The head on `origin`, which a reviewer reads
    pub head: String,
    /// The ticket's worktree
    pub worktree: Checkout,
    /// The files the diff changes, which a reviewer's `paths` are matched against
    pub changed: Vec<String>,
    /// The worker folder holding the findings, deferred and disputed files
    pub build: PathBuf,
}

/// What a pass does next
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStep {
    /// The worktree is not the pushed head: the worker gets this one turn to push or discard
    Push(String),
    /// A fresh session of `reviewer` reads the diff
    Read {
        /// The reviewer, as the project lists it
        reviewer: String,
        /// Whether a second session reads again once this one has
        second_look: bool,
    },
    /// A fresh session of `reviewer` reads again, shown what the first found
    SecondLook {
        /// The reviewer, as the project lists it
        reviewer: String,
        /// What the first look found
        first: Vec<Finding>,
    },
    /// The worker gets these findings, or the tests it left failing, as one fix turn
    FixTurn {
        /// What the worker is sent
        prompt: String,
        /// The findings the turn is for
        findings: Vec<Finding>,
    },
    /// Run the reviewers' tests of the round's findings: before the fix when the round's
    /// findings have none run yet, after it once the fix moved the head
    RunTests,
    /// The pass will not settle it itself
    Rule(Ruling),
    /// The pass is over, and the gate goes on to the merge yes
    Ended {
        /// What the merge yes warns of, none when the head is read or vouched for
        unread: Option<Unread>,
    },
}

/// What the pass rules on, and the reason each puts on the ticket's needs-you line
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ruling {
    /// A reviewer raised findings again that the worker had disputed, and the worker disputed
    /// or left them again, so a reader and the worker disagree and the maintainer settles it
    ReadersDisagree {
        /// The findings
        findings: Vec<Finding>,
    },
    /// A fix turn pushed nothing for findings above a nit, and did not defer or dispute each
    FixNotPushed {
        /// The round whose findings it was
        round: u32,
    },
    /// A reviewer's test still fails after the worker's second fix turn
    TestStillFails {
        /// The tests that fail
        tests: Vec<FailingTest>,
    },
    /// The worktree is still not the pushed head after the worker's turn to push
    NotPushed {
        /// What is off, as [`off_text`] says it
        what: String,
    },
}

impl Ruling {
    /// The reason, which becomes the ticket's needs-you line
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::ReadersDisagree { findings } => format!(
                "A reviewer raised {} again that the worker had disputed, and the worker \
                 disputed or left it again: {}. The reviewer and the worker disagree.",
                findings_of(findings.len()),
                places(findings)
            ),
            Self::FixNotPushed { round } => format!(
                "The worker's fix turn for round {round}'s review findings pushed nothing, and \
                 left findings above a nit that it neither deferred nor disputed."
            ),
            Self::TestStillFails { tests } => format!(
                "{} of the reviewer's failing tests still fail after the worker's second fix \
                 turn: {}.",
                tests.len(),
                (tests.iter())
                    .map(|t| t.test.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::NotPushed { what } => format!(
                "The worker's worktree is still not the pushed head after its turn to push: \
                 {what}. The review waits for it."
            ),
        }
    }
}

// `src/lib.rs:3, src/main.rs`
fn places(findings: &[Finding]) -> String {
    let place = |f: &Finding| match f.line {
        0 => f.file.clone(),
        line => format!("{}:{line}", f.file),
    };
    findings.iter().map(place).collect::<Vec<_>>().join(", ")
}

/// How a fix turn that pushed nothing was left, and the pass went on
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unfixed {
    /// It was sent nits alone, and left them
    NitsDeclined,
    /// It disputed the findings above a nit that it did not defer
    Disputed,
    /// It deferred every finding above a nit as out of scope
    Deferred,
}

/// What a fix turn came to, when the pass goes on
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fixed {
    /// It pushed a head
    Pushed,
    /// It pushed nothing, and left the findings in a way the pass accepts
    Left {
        /// How
        how: Unfixed,
        /// What it disputed, by round, for the next reader
        disputes: Vec<Dispute>,
    },
}

/// What is off about the worktree against the pushed head, none when it is the pushed head,
/// clean
#[must_use]
pub fn off_text(look: &Look) -> Option<String> {
    if look.worktree.head != look.head {
        return Some(format!(
            "it is at {}, not the pushed head {}",
            short(&look.worktree.head),
            short(&look.head)
        ));
    }
    (!look.worktree.clean).then(|| "it holds changes that are not committed".to_owned())
}

/// What the worker is sent when its worktree is off the pushed head
#[must_use]
pub fn push_prompt(number: u64, off: &str) -> String {
    format!(
        "A review of your pull request #{number} is about to read it, but your worktree is \
         not the pushed head: {off}. Commit and push what belongs on the pull request with \
         `git push origin HEAD`, and discard the rest, so the reviewer reads what is on \
         GitHub."
    )
}

/// One step of a ticket's pass: what to do next
///
/// It starts the pass when none is kept. It changes `kept` by what the step itself decided:
/// the reviewer chosen and the head it reads, the stage a turn is sent in, and the round that
/// follows a settled fix. A read's outcome is [`after_read`]'s, and the tests' runs are the
/// caller's, written to [`Kept::checks`].
///
/// `now` is the pass's clock. Nothing in a pass waits on it: CI's wait is the gate's.
pub fn step(look: &Look, kept: &mut Kept, lineup: &Lineup, _now: Timestamp) -> ReviewStep {
    let review = kept.review.get_or_insert_with(Review::first);
    match review.stage {
        ReviewStage::Pushing => {
            if let Some(ruling) = after_push(kept, look) {
                return ReviewStep::Rule(ruling);
            }
        }
        ReviewStage::Fixing { .. } => {
            if let Some(step) = after_fix(kept, look) {
                return step;
            }
        }
        _ => {}
    }
    let review = kept.review.as_mut().expect("the pass was started above");
    match review.stage.clone() {
        ReviewStage::SecondLook { first } => ReviewStep::SecondLook {
            reviewer: review.reviewer.clone().unwrap_or_default(),
            first,
        },
        ReviewStage::Found { findings } => found(look, kept, &findings),
        // `Pushing` and `Fixing` fell through: the turn they wait on has been settled
        _ => round(look, kept, lineup),
    }
}

// A round about to run, or one whose read a restart cleared
fn round(look: &Look, kept: &mut Kept, lineup: &Lineup) -> ReviewStep {
    let review = kept.review.as_mut().expect("the pass is kept");
    review.stage = ReviewStage::Round;
    if let Some(name) = review.reviewer.clone() {
        // A restart cleared its read, or its last call failed: it reads again
        let Some(listed) = lineup.get(&name) else {
            // The project's list dropped it since
            kept.passed_over.push(name);
            *review = review.clone().next_round();
            return round(look, kept, lineup);
        };
        review.reading = Some(look.head.clone());
        return ReviewStep::Read {
            reviewer: name,
            second_look: listed.second_look,
        };
    }
    if pass_ended(review, &kept.passed_over, lineup, &look.changed) {
        return end_review(look, kept, lineup);
    }
    if let Some(off) = off_text(look) {
        review.stage = ReviewStage::Pushing;
        return ReviewStep::Push(push_prompt(look.number, &off));
    }
    let next = lineup.due(review, &look.changed)[0];
    review.reviewer = Some(next.name.clone());
    review.reading = Some(look.head.clone());
    ReviewStep::Read {
        reviewer: next.name.clone(),
        second_look: next.second_look,
    }
}

/// Whether the pass has nothing left to run: it has its reads, or no reviewer is left whose
/// paths touch the diff
#[must_use]
pub fn pass_ended(
    review: &Review,
    passed_over: &[String],
    lineup: &Lineup,
    changed: &[String],
) -> bool {
    lineup.enough_read(review, passed_over, RULE_READS) || lineup.due(review, changed).is_empty()
}

/// Ends the pass at `look`'s head: keeps how it ended short, if it did, and says what the
/// merge yes warns of
pub fn end_review(look: &Look, kept: &mut Kept, lineup: &Lineup) -> ReviewStep {
    let review = kept.review.get_or_insert_with(Review::first);
    let short = lineup.pass_unread(review, &kept.passed_over, RULE_READS);
    kept.short = short.clone().map(|why| Short { why });
    ReviewStep::Ended {
        unread: unread_of(&kept.heads, &look.head, short),
    }
}

// A round's findings: the tests they carry are run before the worker is sent them, then the
// worker is
fn found(look: &Look, kept: &mut Kept, findings: &[Finding]) -> ReviewStep {
    let round = kept.review.as_ref().map_or(1, |r| r.round);
    let unrun = (findings.iter())
        .filter(|f| f.test.is_some())
        .any(|f| !kept.checks.iter().any(|c| c.is_for(round, f)));
    if unrun {
        return ReviewStep::RunTests;
    }
    let deferred_before = deferred(&look.build).unwrap_or_default();
    let review = kept.review.as_mut().expect("the pass is kept");
    review.stage = ReviewStage::Fixing {
        head: look.head.clone(),
        sent: findings.to_vec(),
        deferred_before,
    };
    ReviewStep::FixTurn {
        prompt: fix_prompt(look.number, round, &findings_path(&look.build), findings),
        findings: findings.to_vec(),
    }
}

/// Folds what a read came to into the kept facts
///
/// A read with findings goes to the worker as one fix turn, or to a second look first where
/// the reviewer has one. A clean read goes on to the next reviewer. A call that failed is made
/// again, until the reviewer has failed [`ROUND_FAILURES`](super::ROUND_FAILURES) times in a
/// row and is passed over. A read ended by kelpie stopping counts for nothing, as a restart
/// does. Any read shows the reader the disputes the pass holds.
pub fn after_read(kept: &mut Kept, lineup: &Lineup, outcome: Result<Vec<Finding>, ReviewerError>) {
    let Some(review) = kept.review.as_mut() else {
        return;
    };
    let found = match outcome {
        Err(ReviewerError::Stopped) => return,
        Err(_) => {
            if !review.fail() {
                return;
            }
            let name = review.reviewer.clone().unwrap_or_default();
            kept.passed_over.push(name);
            // A second look that was passed over leaves the first's list as the round's
            if let ReviewStage::SecondLook { first } = review.stage.clone() {
                finish_read(kept, first);
            } else {
                after_round(kept);
            }
            return;
        }
        Ok(found) => found,
    };
    if let Some(head) = review.reading.clone() {
        kept.heads.read(&head);
    }
    review.unread = false;
    review.failures = 0;
    mark_shown(&mut kept.disputes);
    let second_look = (review.reviewer.as_deref())
        .and_then(|name| lineup.get(name))
        .is_some_and(|r| r.second_look);
    match review.stage.clone() {
        ReviewStage::SecondLook { first } => finish_read(kept, both_looks(first, found)),
        ReviewStage::Round if second_look => {
            review.stage = ReviewStage::SecondLook { first: found };
        }
        ReviewStage::Round => finish_read(kept, found),
        _ => {}
    }
}

// The round's reads are done with `found`: none goes to the next reviewer, some to the worker
fn finish_read(kept: &mut Kept, found: Vec<Finding>) {
    if found.is_empty() {
        after_round(kept);
    } else if let Some(review) = kept.review.as_mut() {
        review.stage = ReviewStage::Found { findings: found };
    }
}

/// The round is done: the next round of the same pass is about to run
pub fn after_round(kept: &mut Kept) {
    if let Some(review) = kept.review.take() {
        kept.review = Some(review.next_round());
    }
}

/// Folds the worker's turn to push into the kept facts
///
/// The turn was the worker's one, so a worktree that is still off the pushed head is the
/// ruling not pushed. One that is not goes back to the round, which reads it.
pub fn after_push(kept: &mut Kept, look: &Look) -> Option<Ruling> {
    if let Some(what) = off_text(look) {
        return Some(Ruling::NotPushed { what });
    }
    if let Some(review) = kept.review.as_mut() {
        review.stage = ReviewStage::Round;
    }
    None
}

/// Folds the worker's fix turn into the kept facts, and says what comes first when the round
/// is not done with it
///
/// A fix that moved the head has its reviewers' tests run ([`ReviewStep::RunTests`]), and one
/// that left a test failing sends the worker back once and then rules. One that pushed nothing
/// goes through [`fix_ended`]. Once the fix is settled the head it pushed is vouched for and
/// the round is done ([`after_round`]), and `None` says the step goes on.
pub fn after_fix(kept: &mut Kept, look: &Look) -> Option<ReviewStep> {
    let review = kept.review.as_ref()?;
    let round = review.round;
    let ReviewStage::Fixing {
        head,
        sent,
        deferred_before,
    } = review.stage.clone()
    else {
        return None;
    };
    let moved = look.head != head;
    let owed = round_checks(&kept.checks, round, &sent);
    // A run the fix has no result for yet: a send-back clears the run that failed
    let waiting = |c: &TestCheck| c.after.is_none();
    if moved && owed.iter().any(waiting) {
        return Some(ReviewStep::RunTests);
    }
    if !moved {
        // The worker's second turn on a failing test pushed nothing, so it still fails
        let tests = unfixed_tests(&kept.checks, round, &sent);
        if !tests.is_empty() {
            return Some(ReviewStep::Rule(Ruling::TestStillFails { tests }));
        }
    }
    if moved {
        match after_the_fix(&mut kept.checks, round, &sent) {
            Next::Done => {}
            Next::SendBack(tests) => {
                return Some(send_back(kept, look, &sent, &tests));
            }
            Next::Ruling(tests) => return Some(ReviewStep::Rule(Ruling::TestStillFails { tests })),
        }
    }
    let answered = match moved {
        true => Answers::default(),
        false => answers(
            &look.build,
            Sent {
                findings: &sent,
                deferred_before: &deferred_before,
            },
            round,
            &kept.disputes,
        )
        .unwrap_or_default(),
    };
    match fix_ended(&sent, &answered, &kept.disputes, round, moved) {
        Err(ruling) => Some(ReviewStep::Rule(ruling)),
        Ok(Fixed::Pushed) => {
            kept.heads
                .vouch(&look.head, sent.iter().all(Finding::is_nit));
            after_round(kept);
            None
        }
        Ok(Fixed::Left { disputes, .. }) => {
            record(&mut kept.disputes, &disputes);
            after_round(kept);
            None
        }
    }
}

// The tests the worker was sent back to and has not had a run for since: its second turn
// pushed nothing. Their output is what the run before the fix said.
fn unfixed_tests(checks: &[TestCheck], round: u32, sent: &[Finding]) -> Vec<FailingTest> {
    round_checks(checks, round, sent)
        .into_iter()
        .filter(|c| c.sent_back && c.after.is_none())
        .filter_map(|c| {
            let finding = sent.iter().find(|f| c.is_for(round, f))?;
            let TestOutcome::Failed { output } = c.before else {
                return None;
            };
            Some(FailingTest {
                test: finding.test.clone()?,
                output,
            })
        })
        .collect()
}

// The worker goes back once to the tests its fix left failing: the turn is judged against the
// head it started from
fn send_back(kept: &mut Kept, look: &Look, sent: &[Finding], tests: &[FailingTest]) -> ReviewStep {
    let review = kept.review.as_mut().expect("the pass is kept");
    if let ReviewStage::Fixing { head, .. } = &mut review.stage {
        head.clone_from(&look.head);
    }
    let failing = still_failing(&kept.checks, review.round, sent);
    // The run that failed is cleared, so the second fix's head is run for itself: the check
    // keeps `sent_back`, which says the worker has had its turn
    for failed in &failing {
        let check =
            (kept.checks.iter_mut().rev()).find(|c| c.is_for(review.round, &failed.finding));
        if let Some(check) = check {
            check.after = None;
        }
    }
    ReviewStep::FixTurn {
        prompt: still_failing_prompt(look.number, review.round, &look.build, tests),
        findings: failing.into_iter().map(|f| f.finding).collect(),
    }
}

/// What a fix turn came to: the pass goes on, or it is a ruling
///
/// `sent` is the round's findings, `answered` what the worker said of them in its files and
/// `earlier` what the pass had disputed before, which a finding raised again is held to. A fix
/// that `moved` the head goes on. One that pushed nothing goes on for nits alone, and for
/// findings above a nit that are each deferred or disputed; it is a ruling when it disputed
/// again or left a finding a reader raised again after a dispute (readers disagree), and
/// otherwise when it left a finding above a nit unanswered (fix not pushed).
///
/// # Errors
///
/// The [`Ruling`] the fix comes to.
pub fn fix_ended(
    sent: &[Finding],
    answered: &Answers,
    earlier: &[Dispute],
    round: u32,
    moved: bool,
) -> Result<Fixed, Ruling> {
    if moved {
        return Ok(Fixed::Pushed);
    }
    let above: Vec<(usize, &Finding)> = (sent.iter().enumerate())
        .filter(|(_, f)| !f.is_nit())
        .collect();
    let is_disputed = |at: usize| answered.disputed().iter().any(|(i, _)| *i == at);
    let is_deferred = |at: usize| answered.deferred().iter().any(|(i, _)| *i == at);
    let raised_again = |f: &Finding| {
        earlier
            .iter()
            .any(|d| d.round < round && d.finding.is_same_as(f))
    };
    // Raised again, and disputed again or left without a word
    let disagree: Vec<Finding> = above
        .iter()
        .filter(|(at, f)| {
            answered.repeated_at().contains(at)
                || (raised_again(f) && !is_disputed(*at) && !is_deferred(*at))
        })
        .map(|(_, f)| (*f).clone())
        .collect();
    if !disagree.is_empty() {
        return Err(Ruling::ReadersDisagree { findings: disagree });
    }
    if above.is_empty() {
        return Ok(Fixed::Left {
            how: Unfixed::NitsDeclined,
            disputes: Vec::new(),
        });
    }
    if !above
        .iter()
        .all(|(at, _)| is_disputed(*at) || is_deferred(*at))
    {
        return Err(Ruling::FixNotPushed { round });
    }
    let disputes: Vec<Dispute> = answered
        .disputed()
        .iter()
        .map(|(at, reason)| Dispute {
            round,
            finding: sent[*at].clone(),
            reason: reason.clone(),
            shown: false,
        })
        .collect();
    let how = match disputes.is_empty() {
        true => Unfixed::Deferred,
        false => Unfixed::Disputed,
    };
    Ok(Fixed::Left { how, disputes })
}
