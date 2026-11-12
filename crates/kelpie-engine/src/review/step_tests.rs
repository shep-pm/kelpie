//! A pass's step, each turn of it on plain values

use std::fs;
use std::path::Path;

use super::findings_file::{deferred_path, disputed_path};
use super::step::{Checkout, Fixed, Look, Ruling, Unfixed, fix_ended};
use super::{
    Book, Finding, Kept, Lineup, ReviewStage, ReviewStep, ReviewerError, Severity, TestCheck,
    TestOutcome, Unread, after_read, step,
};
use crate::agents::{Agents, FOLDER};
use crate::github::Timestamp;
use crate::settings::{ConfigDir, Reviewers};
use crate::ticket::TicketId;

const HEAD: &str = "aaaaaaa1111111111111111111111111111111aa";
const FIX: &str = "bbbbbbb2222222222222222222222222222222bb";
const FIX_TWO: &str = "ccccccc3333333333333333333333333333333cc";

const PLAIN: &str = "---\nrole: reviewer\n---\nRead it.\n";
const TWICE: &str = "---\nrole: reviewer\nsecond_look: true\n---\nRead it.\n";

fn ticket(number: u64) -> TicketId {
    TicketId::from_issue(number).unwrap()
}

fn now() -> Timestamp {
    Timestamp::from_secs(1_000)
}

fn lineup(files: &[(&str, &str)], names: &[&str]) -> Lineup {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join(FOLDER);
    fs::create_dir_all(&folder).unwrap();
    for (file, text) in files {
        fs::write(folder.join(file), text).unwrap();
    }
    let agents = Agents::load(&ConfigDir::at(dir.path())).unwrap();
    let reviewers: Reviewers = serde_json::from_value(serde_json::json!(names)).unwrap();
    Lineup::of(&reviewers, &agents)
}

fn three() -> Lineup {
    lineup(
        &[("a.md", PLAIN), ("b.md", PLAIN), ("c.md", PLAIN)],
        &["a", "b", "c"],
    )
}

// A look at `head`, with the worktree on it, clean
fn look(build: &Path, head: &str) -> Look {
    Look {
        number: 71,
        head: head.to_owned(),
        worktree: Checkout {
            head: head.to_owned(),
            clean: true,
        },
        changed: vec!["src/lib.rs".to_owned()],
        build: build.to_path_buf(),
    }
}

fn high() -> Finding {
    Finding::new(Severity::High, "src/lib.rs", 9, "a thing", "it is wrong")
}

fn nit() -> Finding {
    Finding::new(Severity::Low, "src/lib.rs", 2, "a name", "it reads badly")
}

const HIGH_LINE: &str = "HIGH|src/lib.rs:9|a thing|it is wrong\n";
const REASON: &str = "  src/lib.rs:8 already guards it\n";

fn read(name: &str, second_look: bool) -> ReviewStep {
    ReviewStep::Read {
        reviewer: name.to_owned(),
        second_look,
    }
}

// Steps to a read, then folds `found` in as that read's outcome
fn reads(kept: &mut Kept, lineup: &Lineup, look: &Look, found: Vec<Finding>) {
    let step = step(look, kept, lineup, now());
    assert!(matches!(step, ReviewStep::Read { .. }), "{step:?}");
    after_read(kept, lineup, Ok(found));
}

fn fix_turn(step: ReviewStep) -> (String, Vec<Finding>) {
    match step {
        ReviewStep::FixTurn { prompt, findings } => (prompt, findings),
        other => panic!("not a fix turn: {other:?}"),
    }
}

#[test]
fn each_reviewer_reads_once_in_order_and_the_pass_ends_with_its_reads() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());

    assert_eq!(step(&look, &mut kept, &lineup, now()), read("a", false));
    after_read(&mut kept, &lineup, Ok(vec![]));
    assert_eq!(step(&look, &mut kept, &lineup, now()), read("b", false));
    after_read(&mut kept, &lineup, Ok(vec![]));

    // Two reads are what the rule owes: c does not read
    assert_eq!(
        step(&look, &mut kept, &lineup, now()),
        ReviewStep::Ended { unread: None }
    );
    assert_eq!(kept.review.unwrap().ran, ["a", "b"]);
    assert!(kept.heads.reviewed(HEAD));
}

#[test]
fn a_read_with_findings_sends_every_finding_in_one_fix_turn() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![nit(), high()]);

    let (prompt, sent) = fix_turn(step(&look, &mut kept, &lineup, now()));

    assert_eq!(sent, [nit(), high()]);
    assert!(prompt.contains("pull request #71"), "{prompt}");
    assert!(prompt.contains("LOW|src/lib.rs:2|a name"), "{prompt}");
    assert!(prompt.contains(HIGH_LINE.trim_end()), "{prompt}");
    assert!(matches!(
        kept.review.unwrap().stage,
        ReviewStage::Fixing { head, .. } if head == HEAD
    ));
}

#[test]
fn a_clean_read_goes_to_the_next_reviewer() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![]);

    assert_eq!(step(&look, &mut kept, &lineup, now()), read("b", false));
    assert_eq!(kept.review.unwrap().round, 2);
}

#[test]
fn a_reviewer_with_second_look_reads_again_shown_the_first_list() {
    let lineup = lineup(&[("a.md", TWICE)], &["a"]);
    let build = tempfile::tempdir().unwrap();
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());

    assert_eq!(step(&look, &mut kept, &lineup, now()), read("a", true));
    after_read(&mut kept, &lineup, Ok(vec![nit()]));
    assert_eq!(
        step(&look, &mut kept, &lineup, now()),
        ReviewStep::SecondLook {
            reviewer: "a".to_owned(),
            first: vec![nit()],
        }
    );
    after_read(&mut kept, &lineup, Ok(vec![nit(), high()]));

    // Both lists as one, each finding once, the worst first
    let (_, sent) = fix_turn(step(&look, &mut kept, &lineup, now()));
    assert_eq!(sent, [high(), nit()]);
}

#[test]
fn a_worktree_off_the_pushed_head_gets_one_turn_to_push_then_not_pushed() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let mut off = look(build.path(), HEAD);
    off.worktree.head = FIX.to_owned();
    let mut kept = Kept::default();

    let ReviewStep::Push(prompt) = step(&off, &mut kept, &lineup, now()) else {
        panic!("the worker was not sent a turn to push");
    };
    assert!(
        prompt.contains("it is at bbbbbbb, not the pushed head aaaaaaa"),
        "{prompt}"
    );

    // Its turn ended with the worktree still off
    let ruling = step(&off, &mut kept, &lineup, now());
    let ReviewStep::Rule(ruling) = ruling else {
        panic!("not a ruling: {ruling:?}");
    };
    assert!(matches!(ruling, Ruling::NotPushed { .. }));
    assert!(
        ruling
            .reason()
            .contains("it is at bbbbbbb, not the pushed head aaaaaaa"),
        "{}",
        ruling.reason()
    );

    // The worker pushed, so the worktree is the pushed head and the round reads
    let pushed = look(build.path(), FIX);
    assert_eq!(step(&pushed, &mut kept, &lineup, now()), read("a", false));
    assert_eq!(kept.review.unwrap().reading.as_deref(), Some(FIX));
}

#[test]
fn a_worktree_with_uncommitted_changes_is_off_too() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let mut dirty = look(build.path(), HEAD);
    dirty.worktree.clean = false;

    let step = step(&dirty, &mut Kept::default(), &lineup, now());

    let ReviewStep::Push(prompt) = step else {
        panic!("not a push turn: {step:?}");
    };
    assert!(
        prompt.contains("it holds changes that are not committed"),
        "{prompt}"
    );
}

#[test]
fn a_nit_only_fix_starts_no_new_read_once_the_pass_has_its_reads() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (mut look, mut kept) = (look(build.path(), HEAD), Kept::default());

    // The first nit-only fix pushed a head, and the pass still owes a read
    reads(&mut kept, &lineup, &look, vec![nit()]);
    fix_turn(step(&look, &mut kept, &lineup, now()));
    look = self::look(build.path(), FIX);
    assert_eq!(step(&look, &mut kept, &lineup, now()), read("b", false));

    // The second's head counts as read, so the pass is over with no read of c
    after_read(&mut kept, &lineup, Ok(vec![nit()]));
    fix_turn(step(&look, &mut kept, &lineup, now()));
    look = self::look(build.path(), FIX_TWO);
    assert_eq!(
        step(&look, &mut kept, &lineup, now()),
        ReviewStep::Ended { unread: None }
    );
    assert!(kept.heads.nit_fixed(FIX_TWO));
    assert!(kept.heads.reviewed(FIX_TWO));
}

#[test]
fn a_fix_that_pushes_nothing_for_a_finding_above_a_nit_is_fix_not_pushed() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![high(), nit()]);
    fix_turn(step(&look, &mut kept, &lineup, now()));

    let ruling = step(&look, &mut kept, &lineup, now());

    assert_eq!(ruling, ReviewStep::Rule(Ruling::FixNotPushed { round: 1 }));
}

#[test]
fn a_fix_that_defers_or_disputes_every_finding_goes_on() {
    // Deferred
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![high()]);
    fix_turn(step(&look, &mut kept, &lineup, now()));
    fs::write(deferred_path(build.path()), format!("{HIGH_LINE}{REASON}")).unwrap();

    assert_eq!(step(&look, &mut kept, &lineup, now()), read("b", false));
    assert!(kept.disputes.is_empty());

    // Disputed: the reason is kept, for the next reader and the comment
    let (build, mut kept) = (tempfile::tempdir().unwrap(), Kept::default());
    let look = self::look(build.path(), HEAD);
    reads(&mut kept, &lineup, &look, vec![high()]);
    fix_turn(step(&look, &mut kept, &lineup, now()));
    fs::write(disputed_path(build.path()), format!("{HIGH_LINE}{REASON}")).unwrap();

    assert_eq!(step(&look, &mut kept, &lineup, now()), read("b", false));
    assert_eq!(kept.disputes.len(), 1);
    assert_eq!(kept.disputes[0].round, 1);
    assert_eq!(kept.disputes[0].finding, high());
    assert_eq!(kept.disputes[0].reason, "src/lib.rs:8 already guards it");
}

// Round 1 found `high`, which the worker disputed; round 2's reader raises it again
fn raised_again(build: &Path, lineup: &Lineup, kept: &mut Kept) -> Look {
    let look = look(build, HEAD);
    reads(kept, lineup, &look, vec![high()]);
    fix_turn(step(&look, kept, lineup, now()));
    fs::write(disputed_path(build), format!("{HIGH_LINE}{REASON}")).unwrap();
    assert_eq!(step(&look, kept, lineup, now()), read("b", false));
    after_read(kept, lineup, Ok(vec![high()]));
    look
}

#[test]
fn a_finding_raised_again_after_a_dispute_goes_to_the_fix_turn() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let mut kept = Kept::default();
    let look = raised_again(build.path(), &lineup, &mut kept);

    let (_, sent) = fix_turn(step(&look, &mut kept, &lineup, now()));

    assert_eq!(sent, [high()]);
    // The reader was shown the earlier dispute
    assert!(kept.disputes[0].shown);
}

#[test]
fn disputing_a_finding_raised_again_is_readers_disagree() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let mut kept = Kept::default();
    let look = raised_again(build.path(), &lineup, &mut kept);
    fix_turn(step(&look, &mut kept, &lineup, now()));

    // The worker disputes it again, pushing nothing
    fs::write(disputed_path(build.path()), format!("{HIGH_LINE}{REASON}")).unwrap();
    let ruling = step(&look, &mut kept, &lineup, now());

    let ReviewStep::Rule(ruling) = ruling else {
        panic!("not a ruling: {ruling:?}");
    };
    assert_eq!(
        ruling,
        Ruling::ReadersDisagree {
            findings: vec![high()]
        }
    );
    assert!(
        ruling.reason().contains("src/lib.rs:9"),
        "{}",
        ruling.reason()
    );
}

#[test]
fn leaving_a_finding_raised_again_unanswered_is_readers_disagree_too() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let mut kept = Kept::default();
    let look = raised_again(build.path(), &lineup, &mut kept);
    fix_turn(step(&look, &mut kept, &lineup, now()));

    let ruling = step(&look, &mut kept, &lineup, now());

    assert!(matches!(
        ruling,
        ReviewStep::Rule(Ruling::ReadersDisagree { .. })
    ));
}

#[test]
fn a_fix_that_pushed_a_head_goes_on_and_is_vouched_for() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![high()]);
    fix_turn(step(&look, &mut kept, &lineup, now()));

    let pushed = self::look(build.path(), FIX);

    assert_eq!(step(&pushed, &mut kept, &lineup, now()), read("b", false));
    assert!(kept.heads.vouches_for(FIX));
    assert!(!kept.heads.reviewed(FIX));
}

fn failed() -> TestOutcome {
    TestOutcome::Failed {
        output: "assertion failed".to_owned(),
    }
}

#[test]
fn a_test_still_failing_after_the_second_fix_is_test_still_fails() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let tested = high().with_test("tests/found.rs");
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![tested.clone()]);

    // The test is run at the head the reviewer read, before the worker is sent anything
    assert_eq!(step(&look, &mut kept, &lineup, now()), ReviewStep::RunTests);
    kept.checks.push(TestCheck {
        file: tested.file.clone(),
        line: tested.line,
        what: tested.what.clone(),
        round: 1,
        before: failed(),
        after: None,
        sent_back: false,
    });
    fix_turn(step(&look, &mut kept, &lineup, now()));

    // The first fix pushed, and the test is run again and fails
    let first = self::look(build.path(), FIX);
    assert_eq!(
        step(&first, &mut kept, &lineup, now()),
        ReviewStep::RunTests
    );
    kept.checks[0].after = Some(failed());
    let (prompt, sent) = fix_turn(step(&first, &mut kept, &lineup, now()));
    assert!(prompt.contains("tests/found.rs"), "{prompt}");
    assert!(prompt.contains("assertion failed"), "{prompt}");
    assert_eq!(sent, [tested]);

    // The second fix pushed too, and the test fails again
    let second = self::look(build.path(), FIX_TWO);
    assert_eq!(
        step(&second, &mut kept, &lineup, now()),
        ReviewStep::RunTests
    );
    kept.checks[0].after = Some(failed());
    let ruling = step(&second, &mut kept, &lineup, now());

    let ReviewStep::Rule(ruling) = ruling else {
        panic!("not a ruling: {ruling:?}");
    };
    assert!(matches!(ruling, Ruling::TestStillFails { .. }));
    assert!(
        ruling.reason().contains("tests/found.rs"),
        "{}",
        ruling.reason()
    );
}

#[test]
fn a_second_fix_that_pushes_nothing_for_a_failing_test_is_test_still_fails() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let tested = high().with_test("tests/found.rs");
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![tested.clone()]);
    kept.checks.push(TestCheck {
        file: tested.file.clone(),
        line: tested.line,
        what: tested.what.clone(),
        round: 1,
        before: failed(),
        after: None,
        sent_back: false,
    });
    fix_turn(step(&look, &mut kept, &lineup, now()));
    let first = self::look(build.path(), FIX);
    assert_eq!(
        step(&first, &mut kept, &lineup, now()),
        ReviewStep::RunTests
    );
    kept.checks[0].after = Some(failed());
    fix_turn(step(&first, &mut kept, &lineup, now()));

    // The head did not move in the second turn
    let ruling = step(&first, &mut kept, &lineup, now());

    assert!(matches!(
        ruling,
        ReviewStep::Rule(Ruling::TestStillFails { .. })
    ));
}

#[test]
fn a_test_that_passes_after_the_fix_goes_on() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let tested = high().with_test("tests/found.rs");
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![tested.clone()]);
    kept.checks.push(TestCheck {
        file: tested.file.clone(),
        line: tested.line,
        what: tested.what.clone(),
        round: 1,
        before: failed(),
        after: None,
        sent_back: false,
    });
    fix_turn(step(&look, &mut kept, &lineup, now()));
    let fixed = self::look(build.path(), FIX);
    assert_eq!(
        step(&fixed, &mut kept, &lineup, now()),
        ReviewStep::RunTests
    );
    kept.checks[0].after = Some(TestOutcome::Passed);

    assert_eq!(step(&fixed, &mut kept, &lineup, now()), read("b", false));
}

#[test]
fn a_round_in_flight_at_a_restart_reads_again_with_the_same_reviewer() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![]);
    assert_eq!(step(&look, &mut kept, &lineup, now()), read("b", false));

    // The restart kept only what was saved: b's read was in flight
    let mut book = Book::load(build.path().join("review.json")).unwrap();
    *book.entry(ticket(7)) = kept;
    book.save().unwrap();
    let mut book = Book::load(build.path().join("review.json")).unwrap();
    let kept = book.entry(ticket(7));

    assert_eq!(step(&look, kept, &lineup, now()), read("b", false));
    assert_eq!(kept.review.as_ref().unwrap().ran, ["a"]);
}

#[test]
fn a_reviewer_whose_calls_keep_failing_is_passed_over() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());

    for _ in 0..3 {
        assert_eq!(step(&look, &mut kept, &lineup, now()), read("a", false));
        after_read(
            &mut kept,
            &lineup,
            Err(ReviewerError::Failed("no".to_owned())),
        );
    }

    assert_eq!(kept.passed_over, ["a"]);
    assert_eq!(step(&look, &mut kept, &lineup, now()), read("b", false));
}

#[test]
fn a_read_a_restart_stopped_counts_for_nothing() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    for _ in 0..5 {
        assert_eq!(step(&look, &mut kept, &lineup, now()), read("a", false));
        after_read(&mut kept, &lineup, Err(ReviewerError::Stopped));
    }

    assert!(kept.passed_over.is_empty());
}

#[test]
fn a_pass_with_no_reviewer_that_can_run_ends_unread_with_the_reason() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), lineup(&[], &["ghost"]));
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());

    let ended = step(&look, &mut kept, &lineup, now());

    let ReviewStep::Ended {
        unread: Some(unread),
    } = ended
    else {
        panic!("not unread: {ended:?}");
    };
    assert!(
        matches!(&unread, Unread::Short { why: Some(why), .. } if why.contains("ghost is left out")),
        "{unread:?}"
    );
    assert!(kept.short.is_some());
}

#[test]
fn a_pass_that_read_less_than_it_owed_for_a_reason_ends_short() {
    // `a` read, and `ghost` cannot run
    let lineup = lineup(&[("a.md", PLAIN)], &["a", "ghost"]);
    let build = tempfile::tempdir().unwrap();
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![]);

    let ended = step(&look, &mut kept, &lineup, now());

    let ReviewStep::Ended {
        unread: Some(unread),
    } = ended
    else {
        panic!("not short: {ended:?}");
    };
    assert!(
        unread.text().contains("1 of 2 reviewers read it"),
        "{}",
        unread.text()
    );
}

#[test]
fn a_pass_that_ended_with_a_head_pushed_after_its_last_read_warns_of_it() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![]);
    reads(&mut kept, &lineup, &look, vec![]);

    // Someone else pushed after the last read
    let later = self::look(build.path(), FIX_TWO);
    let ended = step(&later, &mut kept, &lineup, now());

    assert!(matches!(
        ended,
        ReviewStep::Ended {
            unread: Some(Unread::Pushed { .. })
        }
    ));
}

#[test]
fn a_fix_that_pushes_nothing_for_nits_alone_goes_on() {
    let sent = [nit()];

    let fixed = fix_ended(&sent, &Default::default(), &[], 1, false);

    assert_eq!(
        fixed,
        Ok(Fixed::Left {
            how: Unfixed::NitsDeclined,
            disputes: vec![]
        })
    );
}

#[test]
fn the_kept_facts_round_trip_through_review_json() {
    let (build, lineup) = (tempfile::tempdir().unwrap(), three());
    let tested = high().with_test("tests/found.rs");
    let (look, mut kept) = (look(build.path(), HEAD), Kept::default());
    reads(&mut kept, &lineup, &look, vec![tested.clone()]);
    kept.checks.push(TestCheck {
        file: tested.file.clone(),
        line: tested.line,
        what: tested.what.clone(),
        round: 1,
        before: failed(),
        after: Some(TestOutcome::Passed),
        sent_back: false,
    });
    fix_turn(step(&look, &mut kept, &lineup, now()));
    let fixed = self::look(build.path(), FIX);
    assert_eq!(step(&fixed, &mut kept, &lineup, now()), read("b", false));
    kept.disputes.push(super::Dispute {
        round: 1,
        finding: high(),
        reason: "it holds".to_owned(),
        shown: false,
    });
    kept.passed_over.push("z".to_owned());
    let file = build.path().join("project").join("review.json");

    let mut book = Book::load(file.clone()).unwrap();
    *book.entry(ticket(7)) = kept.clone();
    book.save().unwrap();
    let reloaded = Book::load(file).unwrap();

    assert_eq!(reloaded.get(ticket(7)), Some(&kept));
    assert_eq!(reloaded.get(ticket(8)), None);
    let json = fs::read_to_string(reloaded.file()).unwrap();
    assert!(json.contains("\"stage\": \"round\""), "{json}");
}
