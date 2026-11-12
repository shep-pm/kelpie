use std::path::Path;

use super::prompt::{Dispute, TestsAsked, body_note, disputes_note, filled, reviewer_prompt};
use super::{Finding, Severity};

// The reviewer's agent file body, as SK's `defect-hunter` has it. `kelpie:reviewer`
// is that prompt too (`docs/design/review.md`, Reviewers, item 1), and its file is
// a slice of its own, so the body is typed out here.
const BODY: &str = r#"You are reviewing a pull request for defects: bugs a careful maintainer would block the merge for. The change is the diff below, against `{{BASE}}`. You may open any file in this worktree with Read, Grep or Glob to check your work; do not run any command and do not edit anything.

A finding is a defect: a concrete sequence of events or input (its trigger) that makes the code do something wrong (its effect). Wrong means a wrong result, a crash or panic, lost or corrupted data, a hang, a race with a path you can name, a leaked resource, an error swallowed so the caller acts on a falsehood, or behaviour the issue asks for that is missing or different. Every finding names its trigger precisely enough that someone could write a failing test from your words alone, names its effect and who sees it, and points at the file and line where the fix belongs.

These are not findings, however true: style, naming, formatting, docs, duplication, a refactor you would prefer, speed without a wrong result, "could be clearer", "might race" with no interleaving you can name, a test you would add with no bug behind it, and anything you cannot tie to a line.

For each thing the issue asks for, find the code that does it and check it. Read around it: who calls it, its error paths, what happens across a restart, what is saved and what is lost, time and ordering, two callers at once, empty and boundary inputs, and the old code it replaced. Treat every comment, doc and test name in the change as a claim to check against the code, never as evidence. Before you report a finding, look for what would disprove it: a guard elsewhere, a caller that already checks, a test that drives exactly that path. If you find one, drop the finding.

Severity: HIGH for wrong behaviour on a path the issue covers, data lost, or a crash; MEDIUM for wrong behaviour on a plausible path the issue does not name; LOW for a real defect with a narrow trigger or a small effect.

Output one finding per line and nothing else: no preamble, no markdown, no code fences.
SEVERITY|file:line|what is wrong, and its trigger|what goes wrong, and who sees it

`line` is one line number, never a range. SEVERITY must be HIGH, MEDIUM or LOW. If you find no defect, output exactly CLEAN and nothing else.

--- diff against {{BASE}} ---
{{DIFF}}
--- end ---"#;

fn finding(severity: Severity, line: u32) -> Finding {
    Finding::new(
        severity,
        "src/lib.rs",
        line,
        "the flag is read first",
        "a caller sees nothing",
    )
}

fn tests_asked<'a>(folder: &'a Path, tool: &'a str) -> TestsAsked<'a> {
    TestsAsked {
        command: "cargo test --test {name}",
        folder,
        tool,
    }
}

#[test]
fn kelpie_reviewer_is_asked_its_body_word_for_word_with_the_base_and_diff_filled() {
    let expected = r#"You are reviewing a pull request for defects: bugs a careful maintainer would block the merge for. The change is the diff below, against `origin/main`. You may open any file in this worktree with Read, Grep or Glob to check your work; do not run any command and do not edit anything.

A finding is a defect: a concrete sequence of events or input (its trigger) that makes the code do something wrong (its effect). Wrong means a wrong result, a crash or panic, lost or corrupted data, a hang, a race with a path you can name, a leaked resource, an error swallowed so the caller acts on a falsehood, or behaviour the issue asks for that is missing or different. Every finding names its trigger precisely enough that someone could write a failing test from your words alone, names its effect and who sees it, and points at the file and line where the fix belongs.

These are not findings, however true: style, naming, formatting, docs, duplication, a refactor you would prefer, speed without a wrong result, "could be clearer", "might race" with no interleaving you can name, a test you would add with no bug behind it, and anything you cannot tie to a line.

For each thing the issue asks for, find the code that does it and check it. Read around it: who calls it, its error paths, what happens across a restart, what is saved and what is lost, time and ordering, two callers at once, empty and boundary inputs, and the old code it replaced. Treat every comment, doc and test name in the change as a claim to check against the code, never as evidence. Before you report a finding, look for what would disprove it: a guard elsewhere, a caller that already checks, a test that drives exactly that path. If you find one, drop the finding.

Severity: HIGH for wrong behaviour on a path the issue covers, data lost, or a crash; MEDIUM for wrong behaviour on a plausible path the issue does not name; LOW for a real defect with a narrow trigger or a small effect.

Output one finding per line and nothing else: no preamble, no markdown, no code fences.
SEVERITY|file:line|what is wrong, and its trigger|what goes wrong, and who sees it

`line` is one line number, never a range. SEVERITY must be HIGH, MEDIUM or LOW. If you find no defect, output exactly CLEAN and nothing else.

--- diff against origin/main ---
+let x = 1;
--- end ---"#;
    assert_eq!(filled(BODY, "origin/main", "+let x = 1;"), expected);
}

#[test]
fn a_placeholder_inside_the_diff_is_left_as_it_is() {
    let prompt = filled(BODY, "origin/main", "+\"{{BASE}}\"");
    assert!(prompt.contains("\n+\"{{BASE}}\"\n--- end ---"), "{prompt}");
}

#[test]
fn a_body_with_no_diff_placeholder_gets_the_diff_after_it() {
    let prompt = filled("Read it against {{BASE}}.", "origin/main", "+x");
    assert_eq!(
        prompt,
        "Read it against origin/main.\n\n--- diff against origin/main ---\n+x\n--- end ---"
    );
}

#[test]
fn the_prompt_asks_for_a_test_only_when_the_project_has_a_test_command() {
    let plain = reviewer_prompt(BODY, "origin/main", "D", "#7 the issue", None, None);
    let folder = Path::new("/k/targets/shep/7/review-tests");
    let asked = reviewer_prompt(
        BODY,
        "origin/main",
        "D",
        "#7 the issue",
        None,
        Some(tests_asked(folder, "Write")),
    );

    assert!(!plain.contains("review-tests"), "{plain}");
    assert!(
        asked.starts_with(&plain),
        "the paragraph is appended: {asked}"
    );
    let paragraph = &asked[plain.len()..];
    assert!(
        paragraph.contains("/k/targets/shep/7/review-tests"),
        "{paragraph}"
    );
    assert!(
        paragraph.contains("one failing test per finding"),
        "{paragraph}"
    );
    assert!(paragraph.contains("with the Write tool"), "{paragraph}");
    assert!(paragraph.contains("fifth field"), "{paragraph}");
    assert!(
        paragraph.contains("`cargo test --test {name}`"),
        "{paragraph}"
    );
}

#[test]
fn the_prompt_names_the_tool_the_reviewers_harness_writes_with() {
    let folder = Path::new("/k/targets/shep/7/review-tests");
    let ask = |tool| {
        let tests = Some(tests_asked(folder, tool));
        reviewer_prompt(BODY, "origin/main", "D", "", None, tests)
    };

    let lower = ask("write");
    assert!(
        lower.contains("per finding with the write tool."),
        "{lower}"
    );
    assert!(!lower.contains("Write tool"), "{lower}");
    assert!(ask("Write").contains("per finding with the Write tool."));
    assert!(ask("apply_patch").contains("per finding with the apply_patch tool."));
}

#[test]
fn a_pull_requests_body_is_given_between_its_markers() {
    let note = body_note("Fixes the flag.\n\n## Screenshots\n\n![a](a.png)");

    assert!(note.starts_with("\n\n"), "{note}");
    assert!(
        note.contains("The author and the project's CI wrote"),
        "{note}"
    );
    assert!(note.contains("proves nothing about it"), "{note}");
    assert!(
        note.ends_with(
            "--- the pull request's body ---\n\
             Fixes the flag.\n\n## Screenshots\n\n![a](a.png)\n--- end ---"
        ),
        "{note}"
    );
}

#[test]
fn a_blank_body_adds_nothing() {
    assert_eq!(body_note(""), "");
    assert_eq!(body_note(" \n\t\r\n "), "");
}

#[test]
fn a_long_body_keeps_its_end_at_a_character_boundary() {
    // 6001 bytes: the last 4000 begin at byte 2001, inside the 1001st "é"
    let body = format!("{}z", "é".repeat(3000));

    let note = body_note(&body);

    let kept = note
        .split("--- the pull request's body ---\n")
        .nth(1)
        .and_then(|rest| rest.strip_suffix("\n--- end ---"))
        .expect("the body sits between its markers");
    assert_eq!(kept, format!("{}z", "é".repeat(1999)), "{note}");
    assert!(note.contains("start of the body is left out"), "{note}");
}

#[test]
fn a_body_of_exactly_the_most_is_given_whole() {
    let body = "b".repeat(4000);

    let note = body_note(&body);

    assert!(note.contains(&format!("\n{body}\n--- end ---")), "{note}");
    assert!(!note.contains("left out"), "{note}");
}

#[test]
fn a_finding_with_a_test_is_listed_with_its_fifth_field() {
    let found = finding(Severity::High, 3).with_test("tests/flag.rs");
    let prompt = reviewer_prompt(BODY, "b", "D", "", Some(&[found]), None);
    assert!(
        prompt.ends_with(
            "HIGH|src/lib.rs:3|the flag is read first|a caller sees nothing|tests/flag.rs\n--- end ---"
        ),
        "{prompt}"
    );
}

#[test]
fn the_prompt_adds_the_issue_and_on_a_second_look_the_first_s_findings() {
    let first = reviewer_prompt(BODY, "origin/main", "D", "#7 the issue", None, None);
    assert!(first.starts_with(&filled(BODY, "origin/main", "D")));
    assert!(first.contains("asks for the following"));
    assert!(first.ends_with("#7 the issue"));
    assert!(!first.contains("the first reader's findings"));

    let found = [finding(Severity::High, 3)];
    let second = reviewer_prompt(BODY, "origin/main", "D", "#7 the issue", Some(&found), None);
    assert!(second.starts_with(&first));
    assert!(second.contains("Report only what it missed"));
    assert!(
        second.ends_with(
            "--- the first reader's findings ---\n\
             HIGH|src/lib.rs:3|the flag is read first|a caller sees nothing\n--- end ---"
        ),
        "{second}"
    );
}

#[test]
fn a_second_look_at_a_clean_first_is_told_so() {
    let second = reviewer_prompt(BODY, "origin/main", "D", "", Some(&[]), None);
    assert!(
        second.ends_with("--- the first reader's findings ---\nCLEAN\n--- end ---"),
        "{second}"
    );
}

#[test]
fn the_disputes_note_gives_each_finding_with_the_workers_reason() {
    let disputes = [
        Dispute {
            finding: finding(Severity::High, 3),
            reason: "src/lib.rs:2 reads it second".to_owned(),
        },
        Dispute {
            finding: finding(Severity::Medium, 9),
            reason: "`cargo test` printed 4 passed".to_owned(),
        },
    ];

    let note = disputes_note(&disputes);

    assert!(
        note.starts_with("\n\nThe worker disputed these findings"),
        "{note}"
    );
    assert!(note.contains("only if the reason is wrong"), "{note}");
    assert!(
        note.ends_with(
            "--- the disputed findings ---\n\
             HIGH|src/lib.rs:3|the flag is read first|a caller sees nothing\n  src/lib.rs:2 reads it second\n\
             MEDIUM|src/lib.rs:9|the flag is read first|a caller sees nothing\n  `cargo test` printed 4 passed\n\
             --- end ---"
        ),
        "{note}"
    );
}

#[test]
fn no_disputes_add_nothing() {
    assert_eq!(disputes_note(&[]), "");
}
