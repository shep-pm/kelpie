use std::path::Path;

use super::disputes::{Dispute, Settled, answers, comment, mark_shown, record};
use super::findings_file::{Sent, deferred_path, disputed_path};
use super::{Finding, Severity};
use crate::local_paths::{Leak, LocalPaths, Surface};

// What round 1 found: one MEDIUM finding, and a HIGH one.
fn medium() -> Finding {
    Finding::new(
        Severity::Medium,
        "src/lib.rs",
        3,
        "the closure destructures a three-tuple",
        "it reads the zip as three values",
    )
}

fn high() -> Finding {
    Finding::new(
        Severity::High,
        "src/lib.rs",
        9,
        "another thing",
        "it is wrong",
    )
}

const MEDIUM_LINE: &str =
    "MEDIUM|src/lib.rs:3|the closure destructures a three-tuple|it reads the zip as three values\n";
const HIGH_LINE: &str = "HIGH|src/lib.rs:9|another thing|it is wrong\n";
const REASON: &str = "  src/runner/review.rs:312 destructures the two-tuple and builds the three-tuple for the next map\n";

fn sent(findings: &[Finding]) -> Sent<'_> {
    Sent {
        findings,
        deferred_before: &[],
    }
}

fn dispute_file(build: &Path, text: &str) {
    std::fs::write(disputed_path(build), text).unwrap();
}

fn defer_file(build: &Path, text: &str) {
    std::fs::write(deferred_path(build), text).unwrap();
}

fn earlier(finding: Finding) -> Dispute {
    Dispute {
        round: 1,
        finding,
        reason: "src/lib.rs:2 reads it second".to_owned(),
        shown: true,
    }
}

#[test]
fn a_fix_that_disputes_every_finding_with_a_reason_is_settled() {
    let build = tempfile::tempdir().unwrap();
    let found = [medium(), high()];
    dispute_file(
        build.path(),
        &format!("{MEDIUM_LINE}{REASON}{HIGH_LINE}  ran it, it printed 4\n"),
    );

    let settled = answers(build.path(), sent(&found), 1, &[])
        .unwrap()
        .settled();

    assert_eq!(
        settled,
        Some(Settled {
            deferred: 0,
            disputed: vec![
                (0, REASON.trim().to_owned()),
                (1, "ran it, it printed 4".to_owned())
            ],
        })
    );
}

#[test]
fn a_dispute_with_no_reason_line_answers_nothing() {
    let build = tempfile::tempdir().unwrap();
    let found = [medium()];
    dispute_file(build.path(), MEDIUM_LINE);

    let answered = answers(build.path(), sent(&found), 1, &[]).unwrap();

    assert_eq!(answered.disputed(), []);
    assert_eq!(answered.settled(), None);
}

#[test]
fn a_fix_that_defers_some_findings_and_disputes_the_rest_is_settled() {
    let build = tempfile::tempdir().unwrap();
    let found = [medium(), high()];
    defer_file(build.path(), HIGH_LINE);
    dispute_file(build.path(), &format!("{MEDIUM_LINE}{REASON}"));

    let settled = answers(build.path(), sent(&found), 1, &[])
        .unwrap()
        .settled();

    assert_eq!(
        settled,
        Some(Settled {
            deferred: 1,
            disputed: vec![(0, REASON.trim().to_owned())],
        })
    );
}

#[test]
fn a_fix_that_disputes_one_finding_and_leaves_another_is_not_settled() {
    let build = tempfile::tempdir().unwrap();
    let found = [medium(), high()];
    dispute_file(build.path(), &format!("{MEDIUM_LINE}{REASON}"));

    let answered = answers(build.path(), sent(&found), 1, &[]).unwrap();

    assert_eq!(answered.disputed(), [(0, REASON.trim().to_owned())]);
    assert_eq!(answered.settled(), None);
}

#[test]
fn a_finding_both_deferred_and_disputed_counts_as_disputed() {
    let build = tempfile::tempdir().unwrap();
    let found = [medium()];
    defer_file(build.path(), MEDIUM_LINE);
    dispute_file(build.path(), &format!("{MEDIUM_LINE}{REASON}"));

    let settled = answers(build.path(), sent(&found), 1, &[])
        .unwrap()
        .settled();

    assert_eq!(
        settled,
        Some(Settled {
            deferred: 0,
            disputed: vec![(0, REASON.trim().to_owned())],
        })
    );
}

#[cfg(unix)]
#[test]
fn a_dispute_file_that_links_to_another_file_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let (build, theirs) = (dir.path().join("build"), dir.path().join("notes.md"));
    std::fs::create_dir(&build).unwrap();
    std::fs::write(&theirs, format!("{MEDIUM_LINE}{REASON}")).unwrap();
    std::os::unix::fs::symlink(&theirs, disputed_path(&build)).unwrap();
    let found = [medium()];

    let err = answers(&build, sent(&found), 1, &[]).unwrap_err();

    assert!(err.contains("disputed-findings.md"), "{err}");
    assert!(err.contains("not a regular file"), "{err}");
    assert!(!err.contains("closure"), "{err}");
}

#[test]
fn a_finding_disputed_a_second_time_is_repeated() {
    let build = tempfile::tempdir().unwrap();
    let found = [medium()];
    dispute_file(
        build.path(),
        &format!("{MEDIUM_LINE}  I ran it again and it still holds.\n"),
    );

    let answered = answers(build.path(), sent(&found), 2, &[earlier(medium())]).unwrap();

    assert!(answered.repeated());
    assert_eq!(answered.disputed(), []);
    assert_eq!(answered.settled(), None);
}

#[test]
fn a_finding_disputed_a_second_time_and_deferred_is_still_repeated() {
    let build = tempfile::tempdir().unwrap();
    let found = [medium()];
    dispute_file(
        build.path(),
        &format!("{MEDIUM_LINE}  I ran it again and it still holds.\n"),
    );
    defer_file(build.path(), MEDIUM_LINE);

    let answered = answers(build.path(), sent(&found), 2, &[earlier(medium())]).unwrap();

    assert!(answered.repeated());
    assert_eq!(answered.settled(), None);
}

#[test]
fn a_finding_raised_again_and_fixed_is_settled() {
    // The fix pushed the raised-again finding's fix and disputed a new one: nothing
    // is disputed twice, so no ruling is owed and the new dispute is the one kept.
    let build = tempfile::tempdir().unwrap();
    let found = [medium(), high()];
    dispute_file(build.path(), &format!("{HIGH_LINE}{REASON}"));

    let answered = answers(build.path(), sent(&found), 2, &[earlier(medium())]).unwrap();

    assert!(!answered.repeated());
    assert_eq!(answered.disputed(), [(1, REASON.trim().to_owned())]);
}

fn raised(round: u32, reason: &str) -> Dispute {
    Dispute {
        round,
        finding: medium(),
        reason: reason.to_owned(),
        shown: false,
    }
}

#[test]
fn a_dispute_recorded_twice_for_the_same_round_is_kept_once() {
    let mut kept = vec![raised(1, "the first try")];

    record(&mut kept, &[raised(1, "the second try")]);

    assert_eq!(kept, [raised(1, "the second try")]);
}

#[test]
fn disputes_of_different_rounds_are_each_kept_and_marked_shown_together() {
    let mut kept = vec![raised(1, "first")];

    record(&mut kept, &[raised(2, "second")]);
    mark_shown(&mut kept);

    assert_eq!(kept.len(), 2);
    assert!(kept.iter().all(|d| d.shown));
}

#[test]
fn the_comment_lists_each_dispute_no_reader_was_shown() {
    let shown = Dispute {
        shown: true,
        ..Dispute {
            finding: high(),
            ..raised(1, "a reader has this one")
        }
    };
    let kept = [shown, raised(2, "src/lib.rs:2 reads it second")];

    let text = comment(&kept, Path::new("/builds/7")).unwrap();

    assert!(
        text.starts_with("The worker disputed these findings"),
        "{text}"
    );
    assert!(
        text.contains(
            "- MEDIUM `src/lib.rs:3`: the closure destructures a three-tuple. it reads the zip as three values\n  The worker's reason: src/lib.rs:2 reads it second\n"
        ),
        "{text}"
    );
    assert!(!text.contains("another thing"), "{text}");
    assert!(!text.contains("a reader has this one"), "{text}");
}

#[test]
fn a_comment_has_nothing_to_say_when_every_dispute_was_shown() {
    let kept = [Dispute {
        shown: true,
        ..raised(1, "src/lib.rs:2 reads it second")
    }];

    assert_eq!(comment(&kept, Path::new("/builds/7")), None);
}

#[test]
fn a_dispute_naming_a_build_file_is_posted_with_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("maintainer-review.md");
    let kept = [raised(
        1,
        &format!("{} shows the closure reads two values", file.display()),
    )];

    let text = comment(&kept, dir.path()).unwrap();

    assert!(
        text.contains(
            "The worker's reason: maintainer-review.md shows the closure reads two values"
        ),
        "the file by its name: {text}"
    );
    assert!(
        !text.contains(&dir.path().display().to_string()),
        "the build folder is left out: {text}"
    );
    let guard = LocalPaths::new([dir.path()]);
    assert_eq!(guard.find(&text, Surface::Prose), None);
}

#[test]
fn a_dispute_naming_another_local_path_is_held() {
    let dir = tempfile::tempdir().unwrap();
    let (build, notes) = (dir.path().join("build"), dir.path().join("notes.md"));
    std::fs::create_dir(&build).unwrap();
    let kept = [raised(
        1,
        &format!("{} shows the closure reads two values", notes.display()),
    )];

    let text = comment(&kept, &build).unwrap();

    let guard = LocalPaths::new([dir.path()]);
    assert_eq!(guard.find(&text, Surface::Prose), Some(Leak::Path));
}
