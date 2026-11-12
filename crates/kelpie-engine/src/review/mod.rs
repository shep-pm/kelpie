//! A reviewer's read of a worker's pull request, and the port a read runs through
//!
//! A reviewer reads a diff and answers with findings (`docs/design/review.md`,
//! Findings and Reviewers). [`Finding`] is one of them, [`parse_findings`] and
//! [`read_review`] turn a reply's text into the list, and [`Reviewer`] is the port
//! a read runs through, so the pass asks one thing whatever stands behind it.
//! Nothing here starts a session: a session that reads stands behind [`Reviewer`]
//! in its own slice.
//!
//! `fake::FakeReviewer` (under the `testing` feature) answers each read from a
//! script for the code that reads the port.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::review::{Severity, parse_findings};
//!
//! let found = parse_findings("HIGH|src/lib.rs:42|does the bad thing|breaks prod");
//! assert_eq!(found[0].severity, Severity::High);
//! ```

#[cfg(any(test, feature = "testing"))]
pub mod fake;
mod finding;
#[cfg(test)]
mod finding_tests;
mod reviewer;

pub use finding::{CLEAN, Finding, Severity, is_test_path, parse_findings, read_review};
pub use reviewer::{Read, Reviewer, ReviewerError};
