//! What a reviewer's session is asked
//!
//! The reviewer's agent file body, with the commit the change is against at
//! `{{BASE}}` and the diff at `{{DIFF}}`, then what the issue asks for, and on a
//! second look what the first found. A body with no `{{DIFF}}` gets the diff after
//! it. The words are SK's (`runner/review/prompts.rs`, `runner/review/calls.rs`).
//! What earlier rounds asked, the PM's plan and the standing rulings are note 3's
//! (`docs/design/review.md`, Reviewers) and are not here.

use std::path::Path;

use super::{CLEAN, Finding, Severity};

/// The most of a pull request's body a reader is given, in bytes, the end of it
const MOST_BODY: usize = 4000;

/// A finding the worker disputed, and the worker's reason
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dispute {
    /// The finding, as an earlier reader raised it
    pub finding: Finding,
    /// Why the worker says it is wrong
    pub reason: String,
}

/// What a reviewer is asked of tests on a project with a test command
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestsAsked<'a> {
    /// The project's test command, with `{test}` and `{name}` in it
    pub command: &'a str,
    /// The only folder the reviewer may write
    pub folder: &'a Path,
    /// The name of the tool its harness writes with
    pub tool: &'a str,
}

/// `body` with `base` and `diff` in their places
pub(super) fn filled(body: &str, base: &str, diff: &str) -> String {
    let body = body.replace("{{BASE}}", base);
    // The diff goes in last, so a placeholder inside it is left as it is.
    match body.contains("{{DIFF}}") {
        true => body.replace("{{DIFF}}", diff),
        false => format!("{body}\n\n--- diff against {base} ---\n{diff}\n--- end ---"),
    }
}

/// What a reviewer's session is asked: its `body` over `diff`, which is
/// against `base`, the issue's `criteria`, on a project with a test command
/// the `tests` it may write, and on a second look what the `first` found
#[must_use]
pub fn reviewer_prompt(
    body: &str,
    base: &str,
    diff: &str,
    criteria: &str,
    first: Option<&[Finding]>,
    tests: Option<TestsAsked<'_>>,
) -> String {
    let mut prompt = filled(body, base, diff);
    if !criteria.trim().is_empty() {
        prompt.push_str(&format!(
            "\n\nThe issue this pull request resolves asks for the following. \
             Report anything it asks for that the diff leaves undone or gets wrong, \
             in the same format.\n\n{criteria}"
        ));
    }
    if let Some(TestsAsked {
        command,
        folder,
        tool,
    }) = tests
    {
        let folder = folder.display();
        prompt.push_str(&format!(
            "\n\nWhere a finding's trigger can be written as a test, also write one failing \
             test per finding with the {tool} tool. Write it into {folder}, which is the only \
             place you may write, at the path the test should have in the repo: a test for \
             tests/flag.rs is written to {folder}/tests/flag.rs. Name that path as a fifth \
             field on the finding's line, SEVERITY|file:line|what|why|tests/flag.rs, and leave \
             the field off a finding with no test. The test must fail on the code as it stands, \
             for the reason the finding gives, and pass once the finding is fixed. Name no path \
             the repo already holds. The project runs a test with `{command}`, where {{test}} \
             is the file's path in the repo and {{name}} its name without the extension."
        ));
    }
    if let Some(first) = first {
        let lines: String = match first.is_empty() {
            true => format!("{CLEAN}\n"),
            false => first.iter().map(line).collect(),
        };
        prompt.push_str(&format!(
            "\n\nAnother reader has already read this change and found what follows. \
             Report only what it missed: a defect it did not list. Do not repeat or \
             reword anything it found. If it missed nothing, output exactly {CLEAN} and \
             nothing else.\n\n--- the first reader's findings ---\n{lines}--- end ---"
        ));
    }
    prompt
}

/// What a reader is told of the findings the worker disputed, to add after
/// its prompt: each finding with the worker's reason, indented under it, and
/// nothing when there are none
#[must_use]
pub fn disputes_note(disputes: &[Dispute]) -> String {
    if disputes.is_empty() {
        return String::new();
    }
    let lines: String = (disputes.iter())
        .map(|d| format!("{}  {}\n", line(&d.finding), d.reason))
        .collect();
    format!(
        "\n\nThe worker disputed these findings of an earlier reader on this pull \
         request, saying each is wrong and why: each finding is followed by the \
         worker's reason, indented. Check the reason against the code. Raise a finding \
         again, in the same format, only if the reason is wrong; leave out one whose \
         reason holds.\n\n--- the disputed findings ---\n{lines}--- end ---"
    )
}

/// The pull request's `body` for a reader to check the change against, to add
/// after its prompt, and nothing for a blank body
///
/// A body over 4000 bytes keeps its end, where a project's CI appends.
#[must_use]
pub fn body_note(body: &str) -> String {
    let body = body.trim();
    if body.is_empty() {
        return String::new();
    }
    let (body, left_out) = match body.len().checked_sub(MOST_BODY) {
        Some(over) if over > 0 => {
            let start = (over..)
                .find(|i| body.is_char_boundary(*i))
                .unwrap_or(body.len());
            (&body[start..], "The start of the body is left out.\n")
        }
        _ => (body, ""),
    };
    format!(
        "\n\nThe author and the project's CI wrote this pull request's body. It \
         describes the change and proves nothing about it: check the diff for what it \
         says.\n{left_out}\n--- the pull request's body ---\n{body}\n--- end ---"
    )
}

/// The severity as a reviewer's reply spells it
#[must_use]
pub fn severity_tag(severity: Severity) -> &'static str {
    match severity {
        Severity::Low => "LOW",
        Severity::Medium => "MEDIUM",
        Severity::High => "HIGH",
    }
}

/// One finding as the readers write it, its test as a fifth field, and a line end
pub(super) fn line(f: &Finding) -> String {
    let test = (f.test.as_ref()).map_or_else(String::new, |t| format!("|{t}"));
    format!(
        "{}|{}:{}|{}|{}{test}\n",
        severity_tag(f.severity),
        f.file,
        f.line,
        f.what,
        f.why
    )
}
