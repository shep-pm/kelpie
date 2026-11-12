//! A reviewer's test, run before the fix and after it
//!
//! With a Test command set, a finding may carry a failing test the reviewer wrote,
//! kept in [`kept_folder`] ([`super::attached`]). Once the head a round read is
//! known, each is put in the worktree at its path, run through a
//! [`TestRunner`] and taken out again; when the fix has moved the head, the run is
//! made again. The pass keeps the runs as [`TestCheck`]s. A test that fails after
//! the fix sends the worker back for one more fix turn ([`Next::SendBack`]), and
//! one that still fails after that is the ruling test still fails
//! ([`Next::Ruling`]). Any other outcome blocks nothing.
//!
//! These are functions over the pass's own values: the [`Setup`] a run needs, the
//! checks the pass keeps, and a list of notes for the maintainer. They start no
//! session, and nothing calls them until the pass does.

use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::attached::kept_folder;
use super::{CEILING, FailingTest, Finding, TestOutcome, TestRun, TestRunner};
use crate::settings::TestCommand;
use crate::ticket::TicketId;
use crate::tools::Door;
use crate::worker_folder;

/// What a check says when the worktree is not at the head the reviewer read
const NOT_AT_THE_HEAD: &str = "the worktree is not at the head the reviewer read";

/// A finding's test and what its two runs said
// wire format: changing this is a breaking change to the state file
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestCheck {
    /// The finding's file
    pub file: String,
    /// The finding's line
    pub line: u32,
    /// The finding's text
    pub what: String,
    /// The round whose findings it was sent in
    pub round: u32,
    /// The run at the head the reviewer read
    pub before: TestOutcome,
    /// The run at the fix's head. None while the fix has not moved the head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<TestOutcome>,
    /// Whether the worker was sent back to a run after the fix that failed.
    /// Set once, so a finding goes back once.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sent_back: bool,
}

/// What the two runs of a finding's test say, none of which holds the pass
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// It failed before the fix and passed after
    Verified,
    /// It passed before the fix, so it does not reproduce the finding
    NotReproducing,
    /// It failed after the fix as well
    StillFailing,
    /// A run was not made, or timed out, so nothing is said
    Unknown,
    /// The fix has not been run yet
    Waiting,
}

impl TestCheck {
    /// Whether this is `finding`'s check from `round`
    #[must_use]
    pub fn is_for(&self, round: u32, finding: &Finding) -> bool {
        self.round == round
            && self.file == finding.file
            && self.line == finding.line
            && self.what == finding.what
    }

    /// What its runs say
    #[must_use]
    pub fn reading(&self) -> Reading {
        use TestOutcome::{Failed, Passed};
        match (&self.before, &self.after) {
            (Passed, _) => Reading::NotReproducing,
            (Failed { .. }, Some(Passed)) => Reading::Verified,
            (Failed { .. }, Some(Failed { .. })) => Reading::StillFailing,
            (Failed { .. }, None) => Reading::Waiting,
            _ => Reading::Unknown,
        }
    }
}

/// What a run of a kept test needs, besides the test
#[derive(Clone, Copy)]
pub struct Setup<'a> {
    /// What runs the command
    pub runner: &'a dyn TestRunner,
    /// The project's Test command, with `{test}` and `{name}`
    pub command: &'a TestCommand,
    /// The work item's ticket
    pub ticket: TicketId,
    /// The work item's worktree, where a test is put while it runs
    pub worktree: &'a Path,
    /// The worker folder: the kept copies, the record of a test in the worktree,
    /// and the run's sandbox settings
    pub worker: &'a Path,
    /// The worker's own sandbox settings, never the maintainer's reach: the file
    /// and the code under it are a model's
    pub reach: &'a crabpot::Config,
    /// The door the run's lease is served on
    pub door: Option<&'a Door>,
}

impl fmt::Debug for Setup<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Setup")
            .field("command", &self.command)
            .field("ticket", &self.ticket)
            .field("worktree", &self.worktree)
            .field("worker", &self.worker)
            .field("door", &self.door)
            .finish_non_exhaustive()
    }
}

// What the worker folder holds while a test is in the worktree: a kelpie
// stopped mid-run leaves the file there, and this says so.
fn placed_name(ticket: TicketId) -> String {
    format!("test-placed-{ticket}")
}

/// The runs of `found`'s tests, one check each, with no run after the fix yet
///
/// `head` is the head the round read, and `worktree_head` the one the worktree is
/// at. Nothing is run when they differ: the outcome there would say nothing of
/// what the reviewer read. A test a stopped run left in the worktree is taken
/// out first ([`clear_stranded`]). The caller keeps the checks.
pub fn tests_before(
    setup: &Setup<'_>,
    round: u32,
    (head, worktree_head): (&str, Option<&str>),
    found: &[Finding],
    notes: &mut Vec<String>,
) -> Vec<TestCheck> {
    clear_stranded(setup, notes);
    let tested: Vec<(&Finding, &str)> = found
        .iter()
        .filter_map(|f| Some((f, f.test.as_deref()?)))
        .collect();
    let own = worktree_head == Some(head);
    tested
        .into_iter()
        .map(|(finding, test)| TestCheck {
            file: finding.file.clone(),
            line: finding.line,
            what: finding.what.clone(),
            round,
            before: match own {
                true => run_kept(setup, test, notes),
                false => TestOutcome::NotRun {
                    reason: NOT_AT_THE_HEAD.to_owned(),
                },
            },
            after: None,
            sent_back: false,
        })
        .collect()
}

/// Runs the tests of `found`, the findings a fix was sent in `round`, at the
/// worktree's head, and records each as its check's `after` in `checks`
///
/// Returns the round's checks as they now stand.
pub fn tests_after(
    setup: &Setup<'_>,
    checks: &mut [TestCheck],
    round: u32,
    found: &[Finding],
    notes: &mut Vec<String>,
) -> Vec<TestCheck> {
    clear_stranded(setup, notes);
    for finding in found {
        let Some(test) = finding.test.as_deref() else {
            continue;
        };
        if let Some(at) = waiting(checks, round, finding) {
            checks[at].after = Some(run_kept(setup, test, notes));
        }
    }
    round_checks(checks, round, found)
}

/// What a fix whose tests were run comes to
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// No test failed after the fix. Any other outcome blocks nothing.
    Done,
    /// The worker goes back to these once, with
    /// [`still_failing_prompt`](super::still_failing_prompt)
    SendBack(Vec<FailingTest>),
    /// The worker had its one more turn and these still fail: the ruling test
    /// still fails
    Ruling(Vec<FailingTest>),
}

/// What the runs after `round`'s fix come to, marking each failing check as sent
/// back when the worker goes back to it
pub fn after_the_fix(checks: &mut [TestCheck], round: u32, found: &[Finding]) -> Next {
    let failing = still_failing(checks, round, found);
    if failing.is_empty() {
        return Next::Done;
    }
    let tests: Vec<FailingTest> = failing.iter().map(|f| f.test.clone()).collect();
    if failing.iter().all(|f| f.sent_back) {
        return Next::Ruling(tests);
    }
    for failed in &failing {
        if let Some(at) = latest(checks, round, &failed.finding) {
            checks[at].sent_back = true;
        }
    }
    Next::SendBack(tests)
}

/// The tests of `found` that failed before the fix and whose latest run after it
/// failed too ([`Reading::StillFailing`]), with the finding each is for and
/// whether the worker was sent back to it
#[must_use]
pub fn still_failing(checks: &[TestCheck], round: u32, found: &[Finding]) -> Vec<Failing> {
    found
        .iter()
        .filter_map(|finding| {
            let test = finding.test.clone()?;
            let check = &checks[latest(checks, round, finding)?];
            // A test that did not fail before the fix never reproduced the finding.
            if check.reading() != Reading::StillFailing {
                return None;
            }
            let Some(TestOutcome::Failed { output }) = &check.after else {
                return None;
            };
            Some(Failing {
                finding: finding.clone(),
                test: FailingTest {
                    test,
                    output: output.clone(),
                },
                sent_back: check.sent_back,
            })
        })
        .collect()
}

/// The checks `found`'s tests have in `round`, as recorded
#[must_use]
pub fn round_checks(checks: &[TestCheck], round: u32, found: &[Finding]) -> Vec<TestCheck> {
    found
        .iter()
        .filter(|f| f.test.is_some())
        .filter_map(|f| latest(checks, round, f))
        .map(|at| checks[at].clone())
        .collect()
}

/// Whether the worker was already sent back to `finding` in `round`, so an
/// earlier fix turn of the round was sent it
#[must_use]
pub fn was_sent_back(checks: &[TestCheck], round: u32, finding: &Finding) -> bool {
    latest(checks, round, finding).is_some_and(|at| checks[at].sent_back)
}

/// A test that failed after the fix
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failing {
    /// The finding it is attached to
    pub finding: Finding,
    /// The test and its output
    pub test: FailingTest,
    /// Whether the worker was already sent back to it
    pub sent_back: bool,
}

/// Takes out the test a stopped kelpie left in the worktree, and says so in
/// `notes`: a stray file is one the worker may commit and the next
/// clean-worktree check would rule on
pub fn clear_stranded(setup: &Setup<'_>, notes: &mut Vec<String>) {
    let ticket = setup.ticket;
    let name = placed_name(ticket);
    let text = match worker_folder::read(setup.worker, Path::new(&name)) {
        Ok(Some(text)) => text,
        Ok(None) => return,
        Err(e) => {
            notes.push(format!(
                "ticket {ticket}: cannot read {name}, so a test a stopped run left in the worktree is not looked for: {e}"
            ));
            return;
        }
    };
    let mut paths = text.lines().map(PathBuf::from);
    let file = paths.next().filter(|file| inside(file));
    let kept = kept_folder(setup.worker, ticket);
    // Only the reviewer's own text is taken out: a file that reads
    // otherwise is not what kelpie put there.
    let ours =
        file.as_deref()
            .is_some_and(|file| match worker_folder::read(setup.worktree, file) {
                Ok(None) => true,
                Ok(Some(held)) => {
                    matches!(worker_folder::read(&kept, file), Ok(Some(copy)) if copy == held)
                }
                Err(_) => false,
            });
    let record = setup.worker.join(&name);
    let left = match file {
        Some(file) if ours => Placed {
            root: setup.worktree.to_owned(),
            file: Some(file),
            made: paths.filter(|dir| inside(dir)).collect(),
            record: Some(record),
        }
        .take_out(),
        Some(file) => {
            let mut left = vec![format!("{}: it is not the reviewer's text", file.display())];
            left.extend(
                std::fs::remove_file(&record)
                    .err()
                    .map(|e| format!("{name}: {e}")),
            );
            left
        }
        // Nothing of the worktree's to take out, and the record would be read again.
        None => match std::fs::remove_file(&record) {
            Ok(()) => {
                notes.push(format!(
                    "ticket {ticket}: kelpie was stopped during a test run, and {name} named no path inside the worktree, so it was removed"
                ));
                return;
            }
            Err(e) => vec![format!("{name}: it names no path inside the worktree: {e}")],
        },
    };
    notes.push(match left.is_empty() {
        true => format!(
            "ticket {ticket}: kelpie was stopped during a test run, and took the test out of the worktree"
        ),
        false => format!(
            "ticket {ticket}: kelpie was stopped during a test run, and the worktree may be dirty, remove by hand: {}",
            left.join("; ")
        ),
    });
}

// The kept copy of `test` run in the worktree, which is left as it was.
fn run_kept(setup: &Setup<'_>, test: &str, notes: &mut Vec<String>) -> TestOutcome {
    match try_run_kept(setup, test, notes) {
        Ok(outcome) => outcome,
        Err(reason) => TestOutcome::NotRun { reason },
    }
}

fn try_run_kept(
    setup: &Setup<'_>,
    test: &str,
    notes: &mut Vec<String>,
) -> Result<TestOutcome, String> {
    let command = fill(setup.command.as_str(), test)?;
    let rel = Path::new(test);
    let ticket = setup.ticket;
    let kept = kept_folder(setup.worker, ticket);
    let text = worker_folder::read(&kept, rel)
        .map_err(|e| format!("cannot read the kept copy of {test}: {e}"))?
        .ok_or_else(|| format!("the kept copy of {test} is gone"))?;
    let record = setup.worker.join(placed_name(ticket));
    let placed = Placed::put(setup.worktree, rel, &text, &record)?;
    let outcome = setup.runner.run(&TestRun {
        command,
        cwd: setup.worktree.to_owned(),
        reach: setup.reach.clone(),
        settings: setup.worker.join(format!("test-run-{ticket}.sandbox.json")),
        door: setup.door.cloned(),
        ceiling: CEILING,
    });
    // The outcome stands, but a worktree the run left dirty is the
    // maintainer's to know: the next run of this path refuses to start
    // over a file that is still there.
    let left = placed.take_out();
    if !left.is_empty() {
        notes.push(format!(
            "ticket {ticket}: the test {test} left the worktree dirty, remove by hand: {}",
            left.join("; ")
        ));
    }
    Ok(outcome)
}

// Whether `path` is a path of one or more plain steps, so it stays below the
// folder it is joined to.
fn inside(path: &Path) -> bool {
    path.components().next().is_some()
        && (path.components()).all(|step| matches!(step, Component::Normal(_)))
}

// `template` with the test's path at `{test}` and its file stem at `{name}`.
// The command goes to a shell, so a path it could read as anything but a
// path is refused.
fn fill(template: &str, test: &str) -> Result<String, String> {
    // The path is joined to the worktree, so it must stay below it, whatever
    // the state file holds.
    if !inside(Path::new(test)) {
        return Err(format!(
            "the path {test} is not a relative path inside the worktree, so it is not run"
        ));
    }
    let plain = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/');
    if !test.chars().all(plain) || test.starts_with('-') {
        return Err(format!(
            "the path {test} holds characters a shell would read, so no command is built from it"
        ));
    }
    let name = Path::new(test)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("the path {test} has no file name"))?;
    // `{name}` is the stem, and a stem can read as an option where the path cannot:
    // `tests/--help.rs` is a plain path.
    if name.starts_with('-') {
        return Err(format!(
            "the path {test} has a file name a command would read as an option, so no command is built from it"
        ));
    }
    let command = template.replace("{test}", test).replace("{name}", name);
    // An empty command is a shell that exits 0 having run nothing.
    match command.trim().is_empty() {
        true => Err("the Test command is empty, so there is nothing to run".to_owned()),
        false => Ok(command),
    }
}

// The newest check of `finding` in `round`, if any.
fn latest(checks: &[TestCheck], round: u32, finding: &Finding) -> Option<usize> {
    checks.iter().rposition(|c| c.is_for(round, finding))
}

// The newest check of `finding` in `round` that the fix has not been run for:
// none was made yet, or the worker was sent back after a failed one.
fn waiting(checks: &[TestCheck], round: u32, finding: &Finding) -> Option<usize> {
    latest(checks, round, finding).filter(|at| {
        let check = &checks[*at];
        check.after.is_none()
            || (check.sent_back && matches!(check.after, Some(TestOutcome::Failed { .. })))
    })
}

// A test file put in the worktree, and the folders made for it
//
// Until it is taken out, a record of it sits in the worker folder, so a kelpie
// that is stopped in the middle of a run can take it out when it starts again.
struct Placed {
    root: PathBuf,
    // none when the worktree held the file already: it is the worker's
    file: Option<PathBuf>,
    // below `root`, top down
    made: Vec<PathBuf>,
    record: Option<PathBuf>,
}

impl Placed {
    // `text` at `rel` below `root`, with `record` written first.
    //
    // A file already there with this very text is the worker's committed copy: it
    // is run as it is, and left. Any other file there is left alone.
    fn put(root: &Path, rel: &Path, text: &str, record: &Path) -> Result<Self, String> {
        match root.join(rel).symlink_metadata() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Ok(_) => {
                return match worker_folder::read(root, rel) {
                    Ok(Some(held)) if held == text => Ok(Self {
                        root: root.to_owned(),
                        file: None,
                        made: Vec::new(),
                        record: None,
                    }),
                    _ => Err("the worktree already holds another file of that name".to_owned()),
                };
            }
            Err(e) => return Err(format!("cannot look for it in the worktree: {e}")),
        }
        let mut made = Vec::new();
        let mut dir = PathBuf::new();
        for folder in rel.parent().into_iter().flat_map(Path::components) {
            dir.push(folder);
            if matches!(root.join(&dir).symlink_metadata(), Err(e) if e.kind() == io::ErrorKind::NotFound)
            {
                made.push(dir.clone());
            }
        }
        let placed = Self {
            root: root.to_owned(),
            file: Some(rel.to_owned()),
            made,
            record: Some(record.to_owned()),
        };
        if let Err(e) = placed.write_record() {
            return Err(format!(
                "cannot record the run, so the test is not put in the worktree: {e}"
            ));
        }
        match worker_folder::write(root, rel, text) {
            Ok(()) => Ok(placed),
            Err(e) => {
                let left = placed.take_out();
                let left = match left.is_empty() {
                    true => String::new(),
                    false => format!(" and cannot clear it: {}", left.join("; ")),
                };
                Err(format!("cannot put the test in the worktree: {e}{left}"))
            }
        }
    }

    // The file's path, then the path of each folder made, a line each.
    fn write_record(&self) -> Result<(), String> {
        let (Some(record), Some(file)) = (&self.record, &self.file) else {
            return Ok(());
        };
        let (Some(folder), Some(name)) = (record.parent(), record.file_name()) else {
            return Err("the record has no place".to_owned());
        };
        let paths = std::iter::once(file).chain(&self.made);
        let text = paths
            .map(|p| format!("{}\n", p.display()))
            .collect::<String>();
        worker_folder::write(folder, Path::new(name), &text).map_err(|e| e.to_string())
    }

    // Removes the file, then the folders made for it, then the record, and
    // names each that stays: a folder the run put something in is still the
    // worktree's mess.
    fn take_out(self) -> Vec<String> {
        let mut left = Vec::new();
        let mut gone = |shown: &Path, result: io::Result<()>| match result {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                left.push(format!("{}: {e}", shown.display()));
            }
            _ => {}
        };
        if let Some(rel) = &self.file {
            let file = self.root.join(rel);
            gone(rel, std::fs::remove_file(&file));
        }
        for rel in self.made.iter().rev() {
            let dir = self.root.join(rel);
            gone(rel, std::fs::remove_dir(&dir));
        }
        if let Some(record) = &self.record {
            gone(record, std::fs::remove_file(record));
        }
        left
    }
}
