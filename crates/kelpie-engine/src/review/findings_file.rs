//! The findings file a fix turn reads, and what the worker is sent
//!
//! A round's findings are written to `review-findings.md` in the worker's build
//! folder (`docs/design/review.md`, Findings). The worker copies a finding it
//! leaves unfixed into `deferred-findings.md` and one it shows is wrong into
//! `disputed-findings.md`, each with its reason on the line after it. Kelpie
//! reads and writes these only through [`crate::worker_folder`], which follows no
//! link a worker plants. The words are SK's (`runner/review/findings.rs`,
//! `runner/review/attached.rs`); a deferred finding stays in its file and is filed
//! as nothing.

use std::path::{Path, PathBuf};

use super::prompt::line;
use super::{Finding, parse_findings};
use crate::gate::KEEP_BODY_TRUE;
use crate::worker_folder;

/// Where a round's findings are written for the worker's next turn
const FINDINGS_FILE: &str = "review-findings.md";

/// Where the worker copies the findings it leaves unfixed
const DEFERRED_FILE: &str = "deferred-findings.md";

/// Where the worker copies the findings it shows are wrong, each with its reason
pub(super) const DISPUTED_FILE: &str = "disputed-findings.md";

/// The folder in the build folder where a reviewer writes its tests
pub const TESTS_FOLDER: &str = "review-tests";

/// The findings file's path, in the work item's build folder
#[must_use]
pub fn findings_path(build: &Path) -> PathBuf {
    build.join(FINDINGS_FILE)
}

/// Where the worker copies a finding it leaves unfixed, in the same folder as
/// the findings file
#[must_use]
pub fn deferred_path(build: &Path) -> PathBuf {
    build.join(DEFERRED_FILE)
}

/// Where the worker copies a finding it shows is wrong, in the same folder as
/// the findings file
#[must_use]
pub fn disputed_path(build: &Path) -> PathBuf {
    build.join(DISPUTED_FILE)
}

/// Where a reviewer writes the tests for a work item whose build folder is `build`
#[must_use]
pub fn tests_folder(build: &Path) -> PathBuf {
    build.join(TESTS_FOLDER)
}

/// A finding the worker copied into the disputed findings file, with the reason
/// on the line after it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disputed {
    /// The finding, as the worker copied it
    pub finding: Finding,
    /// The reason, when the next line is indented by two spaces and says something
    pub reason: Option<String>,
}

/// A finding the worker copied into the deferred findings file, with the reason
/// on the line after it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deferred {
    /// The finding, as the worker copied it
    pub finding: Finding,
    /// The reason, when the next line is indented by two spaces and says something
    pub reason: Option<String>,
}

/// A reviewer's test that failed at the fix's head, and what it wrote
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailingTest {
    /// The test's path in the repo
    pub test: String,
    /// The tail of its output
    pub output: String,
}

/// The findings a round sent the worker, and the deferred findings file's lines
/// when it sent them
#[derive(Debug, Clone, Copy)]
pub struct Sent<'a> {
    /// The findings sent
    pub findings: &'a [Finding],
    /// The deferred findings file's lines then
    pub deferred_before: &'a [Finding],
}

// The text of `name` in `build`, or nothing when there is no such file. A link
// is never followed, so a worker cannot have kelpie read what it points at.
fn read(build: &Path, name: &str) -> Result<String, String> {
    match worker_folder::read(build, Path::new(name)) {
        Ok(text) => Ok(text.unwrap_or_default()),
        Err(e) => Err(format!("cannot read {}: {e}", build.join(name).display())),
    }
}

fn write(build: &Path, name: &str, text: &str) -> Result<(), String> {
    worker_folder::write(build, Path::new(name), text)
        .map_err(|e| format!("cannot write {}: {e}", build.join(name).display()))
}

/// The findings the worker copied into the disputed findings file in `build`,
/// each with its reason: none when there is no file
///
/// # Errors
///
/// A message naming the file when it is there but cannot be read.
pub fn disputed(build: &Path) -> Result<Vec<Disputed>, String> {
    let text = read(build, DISPUTED_FILE)?;
    Ok((with_reasons(&text).into_iter())
        .map(|(finding, reason)| Disputed { finding, reason })
        .collect())
}

// The findings in `text`, each with the reason the worker gave on the line
// after it: indented by two spaces, and saying something.
fn with_reasons(text: &str) -> Vec<(Finding, Option<String>)> {
    let mut read: Vec<(Finding, Option<String>)> = Vec::new();
    for line in text.lines() {
        if let Some(finding) = parse_findings(line).pop() {
            read.push((finding, None));
        } else if let Some(reason) = line.strip_prefix("  ")
            && let Some(last) = read.last_mut().filter(|(_, reason)| reason.is_none())
        {
            last.1 = Some(reason.trim().to_owned()).filter(|r| !r.is_empty());
        }
    }
    read
}

/// The findings the worker copied into the deferred findings file in `build`, as
/// it wrote them: none when there is no file
///
/// # Errors
///
/// As [`deferred_with_reasons`].
pub fn deferred(build: &Path) -> Result<Vec<Finding>, String> {
    let read = deferred_with_reasons(build)?;
    Ok(read.into_iter().map(|d| d.finding).collect())
}

/// The findings the worker copied into the deferred findings file in `build`, each
/// with its reason: none when there is no file
///
/// # Errors
///
/// A message naming the file when it is there but cannot be read.
pub fn deferred_with_reasons(build: &Path) -> Result<Vec<Deferred>, String> {
    let text = read(build, DEFERRED_FILE)?;
    Ok((with_reasons(&text).into_iter())
        .map(|(finding, reason)| Deferred { finding, reason })
        .collect())
}

/// Whether the worker deferred every finding `sent` holds since they were sent,
/// which is never so when none were
///
/// A line the file held before counts once against the lines now, so a deferral
/// left by an earlier round defers nothing in this one.
///
/// # Errors
///
/// As [`deferred`].
pub fn all_deferred(build: &Path, sent: Sent<'_>) -> Result<bool, String> {
    if sent.findings.is_empty() {
        return Ok(false);
    }
    let mut since = deferred(build)?;
    for before in sent.deferred_before {
        if let Some(at) = since.iter().position(|d| d.is_same_as(before)) {
            since.remove(at);
        }
    }
    Ok(sent
        .findings
        .iter()
        .all(|f| since.iter().any(|d| d.is_same_as(f))))
}

/// Writes round `round`'s `findings` to the findings file in `build`, and empties
/// the disputed findings file, since lines an earlier round's worker left there
/// answer nothing in this one
///
/// # Errors
///
/// A message naming the file when it cannot be written.
pub fn write_findings_file(build: &Path, round: u32, findings: &[Finding]) -> Result<(), String> {
    let (deferred, disputed) = (deferred_path(build), disputed_path(build));
    let mut text = format!(
        "Round {round}'s findings, at the reviewer's severity. Fix each one, then \
         commit and push. A finding that is out of scope for this pull request may be \
         left: copy its line, as it stands here, onto a line of its own in {}, and on the \
         next line, indented by two spaces, give your reason for leaving it. A finding \
         you show is wrong is not fixed or deferred: \
         copy its line, as it stands here, into {}, and on the next line, indented by \
         two spaces, give your reason, naming its evidence: a file and line, a command \
         you ran and what it printed, or the version that settles it. The next reviewer \
         is shown your reason, and a finding it raises again is sent to you once more. A \
         finding of any severity, a low one included, that needs a design choice the issue \
         and the docs do not settle is a question for the maintainer, asked in a \
         `<kelpie-question>` block as your instructions say, rather than a guess or a \
         silent skip.",
        deferred.display(),
        disputed.display()
    );
    if findings.iter().any(|f| f.test.is_some()) {
        text.push_str(&format!(
            " A line with a fifth field names a failing test the reviewer wrote for it, \
             which is not in your worktree: read it at {}/<that path>.",
            tests_folder(build).display()
        ));
    }
    text.push_str("\n\n");
    for f in findings {
        text.push_str(&line(f));
    }
    write(build, FINDINGS_FILE, &text)?;
    write(build, DISPUTED_FILE, "")
}

/// What the worker is sent for a round's findings: the round, the pull request, the
/// file they are in, and each finding's line, so the worker's tab shows what it
/// was asked to fix
#[must_use]
pub fn fix_prompt(number: u64, round: u32, path: &Path, findings: &[Finding]) -> String {
    let lines: String = findings.iter().map(line).collect();
    format!(
        "Round {round} of the review on your pull request #{number} found \
         {} finding(s), in {}. Fix each one, then commit and push with \
         `git push origin HEAD`. {KEEP_BODY_TRUE}\n\n{}",
        findings.len(),
        path.display(),
        lines.trim_end()
    )
}

/// What the worker is sent when its last turn on round `round`'s findings pushed
/// nothing
#[must_use]
pub fn again_prompt(number: u64, round: u32, path: &Path) -> String {
    format!(
        "Your last turn on pull request #{number} pushed nothing, so round {round}'s \
         findings in {} still hold. Fix each one, then commit and push with \
         `git push origin HEAD`. {KEEP_BODY_TRUE}",
        path.display(),
    )
}

/// The second fix turn for a reviewer's test the first fix left failing
///
/// The tests are the reviewer's, so the worker reads them and changes the code.
#[must_use]
pub fn still_failing_prompt(
    number: u64,
    round: u32,
    build: &Path,
    failing: &[FailingTest],
) -> String {
    let mut prompt = format!(
        "Your fix for round {round} of the review on pull request #{number} left \
         {} of the reviewer's failing tests still failing. Change the code so each \
         passes, and leave the test alone: it is the reviewer's, and an edit to it \
         is not run.",
        failing.len()
    );
    for failed in failing {
        prompt.push_str(&format!(
            "\n\nThe test {} is kept at {}/{}. Its output ended:\n{}",
            failed.test,
            tests_folder(build).display(),
            failed.test,
            failed.output.trim_end()
        ));
    }
    prompt.push_str(&format!(
        "\n\nCommit and push with `git push origin HEAD`. {KEEP_BODY_TRUE}"
    ));
    prompt
}
