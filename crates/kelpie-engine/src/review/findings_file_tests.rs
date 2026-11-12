use std::path::{Path, PathBuf};

use super::findings_file::{
    FailingTest, Sent, again_prompt, all_deferred, deferred, deferred_path, deferred_with_reasons,
    disputed, disputed_path, findings_path, fix_prompt, still_failing_prompt, tests_folder,
    write_findings_file,
};
use super::{Finding, Severity, parse_findings};
use crate::gate::KEEP_BODY_TRUE;

fn finding(what: &str) -> Finding {
    Finding::new(Severity::Medium, "src/lib.rs", 3, what, "it matters")
}

fn read(build: &Path) -> String {
    std::fs::read_to_string(findings_path(build)).unwrap()
}

#[test]
fn the_file_lists_one_finding_per_line_in_its_wire_format() {
    let dir = tempfile::tempdir().unwrap();
    let findings = [
        Finding::new(
            Severity::High,
            "src/lib.rs",
            9,
            "looks racy",
            "two threads write the same field",
        ),
        Finding::new(
            Severity::Low,
            "src/main.rs",
            1,
            "unused import",
            "dead code",
        ),
    ];

    write_findings_file(dir.path(), 3, &findings).unwrap();

    let text = read(dir.path());
    assert!(
        text.starts_with("Round 3's findings, at the reviewer's severity."),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "in {}, and on the next line, indented by two spaces, give your reason for \
             leaving it. A finding you show is wrong",
            dir.path().join("deferred-findings.md").display()
        )),
        "the header names where a finding left unfixed goes: {text}"
    );
    assert!(
        text.contains(&format!(
            "into {}, and on the next line",
            dir.path().join("disputed-findings.md").display()
        )),
        "the header names where a finding shown to be wrong goes: {text}"
    );
    assert!(
        text.contains(
            "A finding of any severity, a low one included, that needs a design choice \
             the issue and the docs do not settle is a question for the maintainer, asked \
             in a `<kelpie-question>` block as your instructions say"
        ),
        "{text}"
    );
    assert!(
        !text.contains("as an issue"),
        "kelpie files no deferred finding as an issue: {text}"
    );
    assert!(
        text.contains("HIGH|src/lib.rs:9|looks racy|two threads write the same field\n"),
        "{text}"
    );
    assert!(
        text.contains("LOW|src/main.rs:1|unused import|dead code\n"),
        "{text}"
    );
    assert_eq!(parse_findings(&text), findings);
}

#[test]
fn a_findings_test_is_its_fifth_field_and_the_header_says_where_to_read_it() {
    let dir = tempfile::tempdir().unwrap();
    let with = finding("first").with_test("tests/flag.rs");
    let without = finding("second");

    write_findings_file(dir.path(), 1, &[with.clone(), without.clone()]).unwrap();

    let text = read(dir.path());
    assert!(
        text.contains("\nMEDIUM|src/lib.rs:3|first|it matters|tests/flag.rs\n"),
        "{text}"
    );
    assert!(
        text.contains("\nMEDIUM|src/lib.rs:3|second|it matters\n"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "read it at {}/<that path>",
            dir.path().join("review-tests").display()
        )),
        "the header says where the reviewer's file is: {text}"
    );
    assert_eq!(parse_findings(&text), [with, without]);

    write_findings_file(dir.path(), 1, &[finding("third")]).unwrap();
    assert!(!read(dir.path()).contains("review-tests"));
}

#[test]
fn a_new_round_empties_the_disputed_file_an_earlier_round_left() {
    let dir = tempfile::tempdir().unwrap();
    let line = "MEDIUM|src/lib.rs:3|first|it matters\n  it is wrong, see src/lib.rs:2\n";
    std::fs::write(disputed_path(dir.path()), line).unwrap();
    assert_eq!(disputed(dir.path()).unwrap().len(), 1);

    write_findings_file(dir.path(), 2, &[finding("second")]).unwrap();

    assert_eq!(disputed(dir.path()).unwrap(), []);
}

#[cfg(unix)]
#[test]
fn a_findings_file_that_cannot_be_written_is_an_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("readonly");
    std::fs::create_dir(&folder).unwrap();
    let mode = |m| std::fs::set_permissions(&folder, PermissionsExt::from_mode(m)).unwrap();
    mode(0o555);

    let err = write_findings_file(&folder, 1, &[finding("nit")]).unwrap_err();

    mode(0o755);
    assert!(err.contains("review-findings.md"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_findings_file_that_links_to_another_file_is_replaced_and_that_file_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (build, hook) = (dir.path().join("build"), dir.path().join("pre-commit"));
    std::fs::create_dir(&build).unwrap();
    std::fs::write(&hook, "#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink(&hook, findings_path(&build)).unwrap();

    write_findings_file(
        &build,
        1,
        &[Finding::new(Severity::Low, "a.rs", 1, "nit", "style")],
    )
    .unwrap();

    assert_eq!(std::fs::read_to_string(&hook).unwrap(), "#!/bin/sh\n");
    assert!(read(&build).contains("a.rs:1"));
}

#[cfg(unix)]
#[test]
fn a_deferred_file_that_links_to_another_file_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let (build, theirs) = (dir.path().join("build"), dir.path().join("notes.md"));
    std::fs::create_dir(&build).unwrap();
    std::fs::write(&theirs, "HIGH|src/lib.rs:3|a thing|it is wrong\n").unwrap();
    std::os::unix::fs::symlink(&theirs, deferred_path(&build)).unwrap();

    let err = deferred(&build).unwrap_err();

    assert!(err.contains("deferred-findings.md"), "{err}");
    assert!(err.contains("not a regular file"), "{err}");
    assert!(!err.contains("a thing"), "{err}");
}

#[test]
fn no_deferred_file_defers_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(deferred(dir.path()).unwrap(), []);
    assert_eq!(disputed(dir.path()).unwrap(), []);
}

#[test]
fn a_deferred_line_reads_back_with_the_reason_on_the_line_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        "HIGH|src/a.rs:1|first|why a\n",
        "MEDIUM|src/b.rs:2|second|why b\n",
        "MEDIUM|src/c.rs:3|third|why c\n",
    );
    std::fs::write(
        deferred_path(dir.path()),
        format!("{a}  the fix is a new module\n{b}{c}  \n"),
    )
    .unwrap();

    let read = deferred_with_reasons(dir.path()).unwrap();

    let reasons: Vec<_> = read.iter().map(|d| d.reason.as_deref()).collect();
    assert_eq!(reasons, [Some("the fix is a new module"), None, None]);
    let found: Vec<_> = read.into_iter().map(|d| d.finding).collect();
    assert_eq!(found, parse_findings(&format!("{a}{b}{c}")));
}

#[test]
fn a_disputed_line_reads_back_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let text = "HIGH|src/a.rs:1|first|why a\n  src/a.rs:0 guards it\n";
    std::fs::write(disputed_path(dir.path()), text).unwrap();

    let read = disputed(dir.path()).unwrap();

    assert_eq!(read.len(), 1);
    assert_eq!(read[0].finding, parse_findings(text)[0]);
    assert_eq!(read[0].reason.as_deref(), Some("src/a.rs:0 guards it"));
}

#[test]
fn a_finding_deferred_since_it_was_sent_defers_it() {
    let dir = tempfile::tempdir().unwrap();
    let sent = [finding("first"), finding("second")];
    std::fs::write(
        deferred_path(dir.path()),
        "MEDIUM|src/lib.rs:3|first|it matters\nMEDIUM|src/lib.rs:3|second|worded otherwise\n",
    )
    .unwrap();
    let round = Sent {
        findings: &sent,
        deferred_before: &[],
    };

    assert_eq!(all_deferred(dir.path(), round), Ok(true));

    std::fs::write(
        deferred_path(dir.path()),
        "MEDIUM|src/lib.rs:3|first|it matters\n",
    )
    .unwrap();
    assert_eq!(all_deferred(dir.path(), round), Ok(false));
}

#[test]
fn no_findings_sent_are_never_all_deferred() {
    let dir = tempfile::tempdir().unwrap();
    let round = Sent {
        findings: &[],
        deferred_before: &[],
    };
    assert_eq!(all_deferred(dir.path(), round), Ok(false));
}

#[test]
fn a_deferral_an_earlier_round_left_does_not_defer_this_rounds_finding() {
    let dir = tempfile::tempdir().unwrap();
    let same = finding("the flag is misnamed");
    let line = "MEDIUM|src/lib.rs:3|the flag is misnamed|it matters\n";
    std::fs::write(deferred_path(dir.path()), line).unwrap();
    // The same line was in the file when this round's findings were sent.
    let round = Sent {
        findings: std::slice::from_ref(&same),
        deferred_before: std::slice::from_ref(&same),
    };

    assert_eq!(all_deferred(dir.path(), round), Ok(false));

    // The worker copies it once more, which does defer it.
    std::fs::write(deferred_path(dir.path()), format!("{line}{line}")).unwrap();
    assert_eq!(all_deferred(dir.path(), round), Ok(true));
    assert_eq!(
        std::fs::read_to_string(deferred_path(dir.path())).unwrap(),
        format!("{line}{line}"),
        "the file is kept for the follow-ups"
    );
}

#[test]
fn the_fix_prompt_names_the_round_the_pull_request_and_lists_each_finding() {
    let findings = [
        finding("first").with_test("tests/flag.rs"),
        Finding::new(Severity::High, "src/main.rs", 9, "second", "it breaks"),
    ];

    let prompt = fix_prompt(
        71,
        3,
        Path::new("/k/targets/shep/7/review-findings.md"),
        &findings,
    );

    assert_eq!(
        prompt,
        format!(
            "Round 3 of the review on your pull request #71 found 2 \
             finding(s), in /k/targets/shep/7/review-findings.md. Fix each one, then \
             commit and push with `git push origin HEAD`. {KEEP_BODY_TRUE}\n\n\
             MEDIUM|src/lib.rs:3|first|it matters|tests/flag.rs\n\
             HIGH|src/main.rs:9|second|it breaks"
        )
    );
}

#[test]
fn the_again_prompt_names_the_round_and_the_file() {
    let path = Path::new("/k/targets/shep/7/review-findings.md");

    assert_eq!(
        again_prompt(71, 1, path),
        format!(
            "Your last turn on pull request #71 pushed nothing, so round 1's findings in \
             /k/targets/shep/7/review-findings.md still hold. Fix each one, then commit \
             and push with `git push origin HEAD`. {KEEP_BODY_TRUE}"
        )
    );
}

#[test]
fn the_still_failing_prompt_names_each_test_where_it_is_kept_and_how_its_run_ended() {
    let build = PathBuf::from("/k/targets/shep/7");
    let failing = [FailingTest {
        test: "tests/flag.rs".to_owned(),
        output: "assertion failed\n".to_owned(),
    }];

    let prompt = still_failing_prompt(71, 2, &build, &failing);

    assert!(
        prompt.starts_with(
            "Your fix for round 2 of the review on pull request #71 left 1 of the \
             reviewer's failing tests still failing."
        ),
        "{prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "\n\nThe test tests/flag.rs is kept at {}/tests/flag.rs. Its output ended:\n\
             assertion failed\n\nCommit and push",
            tests_folder(&build).display()
        )),
        "{prompt}"
    );
    assert!(prompt.ends_with(KEEP_BODY_TRUE), "{prompt}");
}
