//! A reviewer's read of a worker's pull request, and the port a read runs through
//!
//! A reviewer reads a diff and answers with findings (`docs/design/review.md`,
//! Findings and Reviewers). [`Finding`] is one of them, [`parse_findings`] and
//! [`read_review`] turn a reply's text into the list, and [`Reviewer`] is the port
//! a read runs through, so the pass asks one thing whatever stands behind it.
//! [`SessionReviewer`] is the one that stands behind it: each read is a fresh
//! ACP session of the reviewer, in its sandbox, with no command (`session`).
//! [`Review`] is where one pass stands, and [`Lineup`] says which reviewer reads
//! next and when the pass has its reads.
//!
//! [`step`] says what a pass does next, from a read to a fix turn to its end: a
//! [`ReviewStep`], over what [`Kept`] holds of the ticket and a [`Look`] at the pull
//! request. [`Ruling`] is what it will not settle itself, and [`unread_of`] what the
//! merge yes warns of at a head no reader read or vouched for. [`Book`] keeps every
//! ticket's facts in `review.json`. Nothing runs the step yet, and nothing calls
//! [`SessionReviewer`] yet: the step that runs a pass does.
//!
//! A reviewer's failing test (`docs/design/review.md`, A reviewer's failing test)
//! is kept by [`checked_tests`], run through [`TestRunner`] by [`tests_before`]
//! and [`tests_after`], and read as a [`Reading`]. Nothing here runs a test: the
//! pass does.
//!
//! `fake::FakeReviewer` and `fake::FakeTestRunner` (under the `testing` feature)
//! answer each read and each run from a script for the code that reads the ports.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::review::{Severity, parse_findings};
//!
//! let found = parse_findings("HIGH|src/lib.rs:42|does the bad thing|breaks prod");
//! assert_eq!(found[0].severity, Severity::High);
//! ```

mod attached;
mod checks;
#[cfg(test)]
mod checks_tests;
mod criteria;
mod disputes;
#[cfg(test)]
mod disputes_tests;
#[cfg(any(test, feature = "testing"))]
pub mod fake;
mod finding;
#[cfg(test)]
mod finding_tests;
mod findings_file;
#[cfg(test)]
mod findings_file_tests;
mod kept;
mod lineup;
#[cfg(test)]
mod lineup_tests;
mod pass;
#[cfg(test)]
mod pass_tests;
mod prompt;
#[cfg(test)]
mod prompt_tests;
mod reviewer;
mod session;
#[cfg(test)]
mod session_tests;
mod step;
#[cfg(test)]
mod step_tests;
mod test_runner;
mod unread;
#[cfg(test)]
mod unread_tests;

pub use attached::{Folders, checked_tests, kept_folder};
pub use checks::{
    Failing, Next, Reading, Setup, TestCheck, after_the_fix, clear_stranded, round_checks,
    still_failing, tests_after, tests_before, was_sent_back,
};
pub use criteria::acceptance;
pub use disputes::{
    Answers, Dispute, Settled, answers, comment, findings_of, mark_shown, record, refresh,
};
pub use finding::{CLEAN, Finding, Severity, is_test_path, parse_findings, read_review};
pub use findings_file::{
    Deferred, Disputed, FailingTest, Sent, TESTS_FOLDER, again_prompt, all_deferred, deferred,
    deferred_path, deferred_with_reasons, disputed, disputed_path, findings_path, fix_prompt,
    still_failing_prompt, tests_folder, write_findings_file,
};
pub use kept::{Book, FILE, Kept, Short};
pub use lineup::{LeftOut, Lineup, Listed, RULE_READS, touches};
pub use pass::{ROUND_FAILURES, Review, ReviewStage, both_looks};
pub use prompt::{TestsAsked, body_note, disputes_note, reviewer_prompt, severity_tag};
pub use reviewer::{Read, Reviewer, ReviewerError};
pub use session::{
    Open, Opening, ReviewProject, SessionReviewer, diff_against, open_on, write_tool,
};
pub use step::{
    Checkout, Fixed, Look, ReviewStep, Ruling, Unfixed, after_fix, after_push, after_read,
    after_round, end_review, fix_ended, off_text, pass_ended, push_prompt, step,
};
pub use test_runner::{CEILING, OUTPUT_KEPT, TestOutcome, TestRun, TestRunner, tail};
pub use unread::{Heads, Unread, unread_of};
