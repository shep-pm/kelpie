//! A failing test a reviewer attaches to a finding
//!
//! With a Test command set, a reviewer may write a failing test for a finding into
//! the tests folder of the work item's build folder ([`tests_folder`]), at the
//! path the test should have in the repo, and name that path as the finding's
//! `test`. The reviewer can write nowhere else. When the round's findings are
//! taken, each attached file is checked, and a checked one is copied to
//! [`kept_folder`], a folder the worker cannot write: the original stays in the
//! build folder for the worker to read, and the copy is what a later step runs,
//! so the worker cannot change it.

use std::io;
use std::path::{Path, PathBuf};

use super::{Finding, TESTS_FOLDER, tests_folder};
use crate::ticket::TicketId;
use crate::worker_folder;

/// The folders a round's attached tests are checked between
#[derive(Debug, Clone, Copy)]
pub struct Folders<'a> {
    /// The work item's ticket
    pub ticket: TicketId,
    /// The work item's worktree, which must hold no file of a test's name
    pub worktree: &'a Path,
    /// The work item's build folder, whose tests folder the reviewer wrote in
    pub build: &'a Path,
    /// The worker folder, which holds the checked copies and which the worker
    /// cannot write
    pub worker: &'a Path,
}

/// Where the checked copies of `ticket`'s tests are kept, under the worker folder
#[must_use]
pub fn kept_folder(worker: &Path, ticket: TicketId) -> PathBuf {
    worker.join(format!("{TESTS_FOLDER}-{ticket}"))
}

/// `found` with each attached test checked: a regular file in the reviewer's
/// folder that the worktree has no file of that name for, which is then copied to
/// the worker folder's
///
/// A finding that fails keeps its text and loses `test`, and a note is pushed to
/// `notes` saying why. When the round was not `asked` for tests, a fifth field was
/// never one, so it goes back into `why` as an older kelpie read it.
pub fn checked_tests(
    asked: bool,
    folders: &Folders<'_>,
    found: Vec<Finding>,
    notes: &mut Vec<String>,
) -> Vec<Finding> {
    let (written, kept) = (
        tests_folder(folders.build),
        kept_folder(folders.worker, folders.ticket),
    );
    let mut checked = Vec::with_capacity(found.len());
    for mut finding in found {
        let Some(test) = finding.test.take() else {
            checked.push(finding);
            continue;
        };
        if !asked {
            finding.why = format!("{}|{test}", finding.why);
        } else {
            match keep(&written, &kept, folders.worktree, &test) {
                Ok(true) => finding.test = Some(test),
                // A pipe in the why can read as a path. With no file left, it
                // is part of the why again.
                Ok(false) => {
                    notes.push(refused(folders.ticket, &finding, &test, NO_FILE));
                    finding.why = format!("{}|{test}", finding.why);
                }
                Err(reason) => notes.push(refused(folders.ticket, &finding, &test, &reason)),
            }
        }
        checked.push(finding);
    }
    checked
}

const NO_FILE: &str = "the reviewer left no such file";

// The note for a test that is dropped while its finding stays.
fn refused(ticket: TicketId, finding: &Finding, test: &str, reason: &str) -> String {
    format!(
        "ticket {ticket}: the test {test} for {}:{} is dropped and the finding stays: {reason}",
        finding.file, finding.line
    )
}

// Checks `test` below `written` and copies it below `kept`: whether the
// reviewer left the file, and why a file it left is refused.
fn keep(written: &Path, kept: &Path, worktree: &Path, test: &str) -> Result<bool, String> {
    let rel = Path::new(test);
    let Some(text) = worker_folder::read(written, rel).map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    // A link there is as much the worktree's as a file.
    match worktree.join(rel).symlink_metadata() {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => return Err("the worktree already holds a file of that name".to_owned()),
        Err(e) => return Err(format!("cannot look for it in the worktree: {e}")),
    }
    worker_folder::write(kept, rel, &text).map_err(|e| format!("cannot keep a copy: {e}"))?;
    Ok(true)
}
