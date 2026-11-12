//! A reviewer's read of a worker's pull request, and the port a read runs through
//!
//! A reviewer reads a diff and answers with findings (`docs/design/review.md`,
//! Findings and Reviewers). [`Finding`] is one of them, [`parse_findings`] and
//! [`read_review`] turn a reply's text into the list, and [`Reviewer`] is the port
//! a read runs through, so the pass asks one thing whatever stands behind it.
//! Nothing here starts a session: a session that reads stands behind [`Reviewer`]
//! in its own slice.
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
#[cfg(any(test, feature = "testing"))]
pub mod fake;
mod finding;
#[cfg(test)]
mod finding_tests;
mod findings_file;
#[cfg(test)]
mod findings_file_tests;
mod prompt;
#[cfg(test)]
mod prompt_tests;
mod reviewer;
mod test_runner;

pub use attached::{Folders, checked_tests, kept_folder};
pub use checks::{
    Failing, Next, Reading, Setup, TestCheck, after_the_fix, clear_stranded, round_checks,
    still_failing, tests_after, tests_before, was_sent_back,
};
pub use criteria::acceptance;
pub use finding::{CLEAN, Finding, Severity, is_test_path, parse_findings, read_review};
pub use findings_file::{
    Deferred, Disputed, FailingTest, Sent, TESTS_FOLDER, again_prompt, all_deferred, deferred,
    deferred_path, deferred_with_reasons, disputed, disputed_path, findings_path, fix_prompt,
    still_failing_prompt, tests_folder, write_findings_file,
};
pub use prompt::{Dispute, TestsAsked, body_note, disputes_note, reviewer_prompt, severity_tag};
pub use reviewer::{Read, Reviewer, ReviewerError};
pub use test_runner::{CEILING, OUTPUT_KEPT, TestOutcome, TestRun, TestRunner, tail};
