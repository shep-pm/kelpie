//! A reviewer's test kept, run before the fix and after it, and read
//!
//! SK's `runner/review/checks/tests.rs`, on [`FakeTestRunner`] and real folders:
//! the pass that drives these functions is a later slice's, so each test plays
//! its steps by hand: the reviewer's file is attached, the run before is made,
//! the worktree's head moves, and the run after is made.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;

use super::TestOutcome::{self, Failed, NotRun, Passed, TimedOut};
use super::fake::FakeTestRunner;
use super::{
    Finding, Folders, Next, Reading, Setup, Severity, TESTS_FOLDER, TestCheck, after_the_fix,
    checked_tests, kept_folder, round_checks, still_failing, still_failing_prompt, tests_after,
    tests_before, tests_folder, was_sent_back,
};
use crate::settings::TestCommand;
use crate::ticket::TicketId;

const TEST_FILE: &str = "tests/flag.rs";
const TEST_TEXT: &str = "#[test]\nfn flag_is_read() { assert!(false); }\n";
const HEAD: &str = "read-head";

fn failed(output: &str) -> TestOutcome {
    Failed {
        output: output.to_owned(),
    }
}

fn finding() -> Finding {
    Finding::new(
        Severity::Medium,
        "src/lib.rs",
        3,
        "the flag is read first",
        "a caller sees nothing",
    )
}

fn finding_testing(path: &str) -> Finding {
    finding().with_test(path)
}

// A work item's folders, a project with a Test command, and the runner's script.
struct Rig {
    _dir: TempDir,
    worktree: PathBuf,
    worker: PathBuf,
    build: PathBuf,
    tests: FakeTestRunner,
    command: TestCommand,
    reach: crabpot::Config,
    notes: RefCell<Vec<String>>,
}

impl Rig {
    fn new() -> Self {
        Self::with_command("cargo test --test {name}")
    }

    fn with_command(command: &str) -> Self {
        let dir = TempDir::new().unwrap();
        let (worktree, worker, build) = (
            dir.path().join("worktree"),
            dir.path().join("worker"),
            dir.path().join("build"),
        );
        for folder in [&worktree, &worker, &build] {
            std::fs::create_dir_all(folder).unwrap();
        }
        Self {
            _dir: dir,
            worktree,
            worker,
            build,
            tests: FakeTestRunner::new(),
            command: TestCommand::try_from(command.to_owned()).unwrap(),
            reach: crabpot::Config::default(),
            notes: RefCell::new(Vec::new()),
        }
    }

    fn said(&self) -> Vec<String> {
        self.notes.borrow().clone()
    }

    fn ticket() -> TicketId {
        "7".parse().unwrap()
    }

    fn setup(&self) -> Setup<'_> {
        Setup {
            runner: &self.tests,
            command: &self.command,
            ticket: Self::ticket(),
            worktree: &self.worktree,
            worker: &self.worker,
            reach: &self.reach,
            door: None,
        }
    }

    fn folders(&self) -> Folders<'_> {
        Folders {
            ticket: Self::ticket(),
            worktree: &self.worktree,
            build: &self.build,
            worker: &self.worker,
        }
    }

    // The reviewer writes `text` at `path` in its folder, names it in its finding,
    // and the round's findings are taken.
    fn attached(&self, path: &str, text: &str) -> Vec<Finding> {
        let at = tests_folder(&self.build).join(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
        let folders = self.folders();
        checked_tests(
            true,
            &folders,
            vec![finding_testing(path)],
            &mut self.notes.borrow_mut(),
        )
    }

    // The run before the fix, at the head the reviewer read.
    fn before(&self, found: &[Finding]) -> Vec<TestCheck> {
        let setup = self.setup();
        tests_before(
            &setup,
            2,
            (HEAD, Some(HEAD)),
            found,
            &mut self.notes.borrow_mut(),
        )
    }

    // The run after a fix, and what it comes to.
    fn after(&self, checks: &mut [TestCheck], found: &[Finding]) -> (Vec<TestCheck>, Next) {
        let setup = self.setup();
        let ran = tests_after(&setup, checks, 2, found, &mut self.notes.borrow_mut());
        (ran, after_the_fix(checks, 2, found))
    }

    fn kept(&self, path: &str) -> PathBuf {
        kept_folder(&self.worker, Self::ticket()).join(path)
    }

    fn record(&self) -> PathBuf {
        self.worker.join("test-placed-7")
    }
}

fn the_flag_check(before: TestOutcome, after: Option<TestOutcome>) -> TestCheck {
    TestCheck {
        file: "src/lib.rs".into(),
        line: 3,
        what: "the flag is read first".into(),
        round: 2,
        before,
        after,
        sent_back: false,
    }
}

fn sent_back(check: TestCheck) -> TestCheck {
    TestCheck {
        sent_back: true,
        ..check
    }
}

#[test]
fn a_test_that_fails_before_and_passes_after_is_recorded_verified() {
    let rig = Rig::new();
    rig.tests.script([failed("assertion failed: flag"), Passed]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);

    let mut checks = rig.before(&found);
    assert_eq!(
        checks,
        [the_flag_check(failed("assertion failed: flag"), None)],
        "the run before the fix is made with the findings"
    );
    let (ran, next) = rig.after(&mut checks, &found);

    let verified = the_flag_check(failed("assertion failed: flag"), Some(Passed));
    assert_eq!(verified.reading(), Reading::Verified);
    assert_eq!(checks, std::slice::from_ref(&verified));
    assert_eq!(ran, [verified]);
    assert_eq!(next, Next::Done);
}

#[test]
fn a_test_that_passes_before_the_fix_is_not_reproducing_and_blocks_nothing() {
    let rig = Rig::new();
    rig.tests.script([Passed, Passed]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    let mut checks = rig.before(&found);

    let (_, next) = rig.after(&mut checks, &found);

    assert_eq!(checks, [the_flag_check(Passed, Some(Passed))]);
    assert_eq!(checks[0].reading(), Reading::NotReproducing);
    assert_eq!(next, Next::Done, "the pass goes on");
}

fn the_failing_test(output: &str) -> super::FailingTest {
    super::FailingTest {
        test: TEST_FILE.to_owned(),
        output: output.to_owned(),
    }
}

#[test]
fn a_test_still_failing_after_the_fix_sends_the_worker_back_once() {
    let rig = Rig::new();
    rig.tests.script([failed("before"), failed("after")]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    let mut checks = rig.before(&found);

    let (_, next) = rig.after(&mut checks, &found);

    let Next::SendBack(tests) = next else {
        panic!("{next:?}");
    };
    assert_eq!(tests, [the_failing_test("after")]);
    assert_eq!(
        checks,
        [sent_back(the_flag_check(
            failed("before"),
            Some(failed("after"))
        ))],
        "the failed run stays, with the mark"
    );
    assert!(was_sent_back(&checks, 2, &found[0]));
    let prompt = still_failing_prompt(71, 2, &rig.build, &tests);
    assert!(prompt.contains(TEST_FILE), "{prompt}");
    assert!(prompt.contains("after"), "{prompt}");
    assert!(!prompt.contains("before"), "{prompt}");
    assert!(prompt.contains("leave the test alone"), "{prompt}");
    let kept = kept_folder(&rig.worker, Rig::ticket());
    assert!(
        !prompt.contains(kept.to_str().unwrap()),
        "the worker reads the build folder's copy: {prompt}"
    );
    assert_eq!(rig.tests.ran().len(), 2, "the test is not run mid-turn");
}

#[test]
fn a_test_still_failing_after_the_second_fix_is_the_ruling_test_still_fails() {
    let rig = Rig::new();
    rig.tests
        .script([failed("before"), failed("after"), failed("after again")]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    let mut checks = rig.before(&found);
    let (_, first) = rig.after(&mut checks, &found);
    assert!(matches!(first, Next::SendBack(_)), "{first:?}");

    let (_, second) = rig.after(&mut checks, &found);

    assert_eq!(second, Next::Ruling(vec![the_failing_test("after again")]));
    assert_eq!(rig.tests.ran().len(), 3);
    assert_eq!(checks[0].reading(), Reading::StillFailing);
}

#[test]
fn a_test_that_passes_on_the_second_fix_is_verified_and_raises_nothing() {
    let rig = Rig::new();
    rig.tests
        .script([failed("before"), failed("after"), Passed]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    let mut checks = rig.before(&found);
    rig.after(&mut checks, &found);

    let (_, next) = rig.after(&mut checks, &found);

    assert_eq!(next, Next::Done);
    assert_eq!(
        checks,
        [sent_back(the_flag_check(failed("before"), Some(Passed)))]
    );
}

#[test]
fn a_run_that_timed_out_or_never_ran_does_not_send_the_worker_back() {
    for after in [
        TimedOut,
        NotRun {
            reason: "the command could not start".into(),
        },
    ] {
        let rig = Rig::new();
        rig.tests.script([failed("before"), after.clone()]);
        let found = rig.attached(TEST_FILE, TEST_TEXT);
        let mut checks = rig.before(&found);

        let (_, next) = rig.after(&mut checks, &found);

        assert_eq!(next, Next::Done, "the pass went on");
        assert_eq!(checks[0].reading(), Reading::Unknown);
        assert!(!checks[0].sent_back);
        assert!(still_failing(&checks, 2, &found).is_empty());
    }
}

#[test]
fn a_run_past_its_ceiling_is_timed_out_and_the_run_asks_for_that_ceiling() {
    let rig = Rig::new();
    rig.tests.script([TimedOut]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);

    let checks = rig.before(&found);

    assert_eq!(checks, [the_flag_check(TimedOut, None)]);
    for run in rig.tests.ran() {
        assert_eq!(run.run.ceiling, Duration::from_secs(10 * 60));
    }
}

#[test]
fn the_test_file_is_gone_from_the_worktree_after_each_run() {
    let rig = Rig::new();
    rig.tests.watch(TEST_FILE);
    rig.tests.script([failed("no"), Passed]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);

    let mut checks = rig.before(&found);
    assert!(
        !rig.worktree.join(TEST_FILE).exists(),
        "gone after the first run"
    );
    assert!(
        !rig.worktree.join("tests").exists(),
        "and the folder it made"
    );
    rig.after(&mut checks, &found);
    assert!(
        !rig.worktree.join(TEST_FILE).exists(),
        "gone after the second run"
    );
    assert!(!rig.worktree.join("tests").exists());

    let ran = rig.tests.ran();
    assert_eq!(ran.len(), 2);
    for run in &ran {
        assert_eq!(
            run.held,
            [PathBuf::from(TEST_FILE)],
            "it was there as the run ran"
        );
    }
    // The reviewer's copy for the next run is untouched.
    assert!(rig.kept(TEST_FILE).is_file());
}

#[test]
fn the_after_run_waits_for_the_workers_own_head() {
    let rig = Rig::new();
    rig.tests.script([failed("no")]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);

    // The worktree is somewhere else than the head the reviewer read.
    let setup = rig.setup();
    let mut notes = Vec::new();
    let checks = tests_before(&setup, 2, (HEAD, Some("elsewhere")), &found, &mut notes);

    let NotRun { reason } = &checks[0].before else {
        panic!("{checks:?}");
    };
    assert!(reason.contains("not at the head"), "{reason}");
    assert!(rig.tests.ran().is_empty(), "nothing ran off the head");
}

#[test]
fn the_command_gets_the_tests_path_and_name_filled_in() {
    let rig = Rig::with_command("cargo test --test {name} -- {test}");
    let found = rig.attached(TEST_FILE, TEST_TEXT);

    rig.before(&found);

    let ran = rig.tests.ran();
    assert_eq!(ran.len(), 1);
    let run = &ran[0].run;
    assert_eq!(run.command, "cargo test --test flag -- tests/flag.rs");
    assert_eq!(run.cwd, rig.worktree);
    assert_eq!(run.reach, rig.reach, "it runs under the worker's settings");
    assert_eq!(run.door, None);
    assert!(run.settings.starts_with(&rig.worker));
}

#[test]
fn a_path_a_shell_would_read_is_recorded_not_run() {
    let rig = Rig::new();
    let found = rig.attached("tests/a;b.rs", TEST_TEXT);

    let checks = rig.before(&found);

    assert_eq!(checks.len(), 1);
    let NotRun { reason } = &checks[0].before else {
        panic!("{:?}", checks[0]);
    };
    assert!(reason.contains("tests/a;b.rs"), "{reason}");
    assert!(rig.tests.ran().is_empty(), "no command was built from it");
}

#[test]
fn a_finding_without_a_test_is_never_run() {
    let rig = Rig::new();

    let mut checks = rig.before(&[finding()]);
    let (ran, next) = rig.after(&mut checks, &[finding()]);

    assert!(checks.is_empty());
    assert!(ran.is_empty());
    assert_eq!(next, Next::Done);
    assert!(rig.tests.ran().is_empty());
}

// The path comes from the state file, which is read as it stands.
#[test]
fn a_path_that_leaves_the_worktree_is_recorded_not_run() {
    let rig = Rig::new();
    let found = [
        finding_testing("../escape.rs"),
        finding_testing("/etc/escape.rs"),
    ];

    let checks = rig.before(&found);

    assert_eq!(checks.len(), 2);
    for check in &checks {
        let NotRun { reason } = &check.before else {
            panic!("{check:?}");
        };
        assert!(reason.contains("inside the worktree"), "{reason}");
    }
    assert!(rig.tests.ran().is_empty(), "no command was built from them");
}

#[test]
fn files_a_run_leaves_in_a_folder_the_test_made_are_reported() {
    let rig = Rig::new();
    rig.tests.leave("tests/left.txt");
    rig.tests.script([failed("no")]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);

    let checks = rig.before(&found);

    assert!(
        !rig.worktree.join(TEST_FILE).exists(),
        "the test itself is out"
    );
    assert!(
        rig.worktree.join("tests/left.txt").exists(),
        "the run's own file stays"
    );
    let notes = rig.said().join("\n");
    assert!(notes.contains("left the worktree dirty"), "{notes}");
    assert!(notes.contains("tests"), "{notes}");
    assert_eq!(
        checks,
        [the_flag_check(failed("no"), None)],
        "the outcome is recorded all the same"
    );
}

// The fix prompt sends the worker to read the reviewer's test, and the natural
// fix commits a copy of it at its path.
#[test]
fn a_test_the_worker_committed_unchanged_is_still_run_after_the_fix_and_left() {
    let rig = Rig::new();
    rig.tests.watch(TEST_FILE);
    rig.tests.script([failed("no"), Passed]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    let mut checks = rig.before(&found);

    std::fs::create_dir_all(rig.worktree.join("tests")).unwrap();
    std::fs::write(rig.worktree.join(TEST_FILE), TEST_TEXT).unwrap();
    rig.after(&mut checks, &found);

    let verified = the_flag_check(failed("no"), Some(Passed));
    assert_eq!(verified.reading(), Reading::Verified);
    assert_eq!(checks, [verified]);
    let ran = rig.tests.ran();
    assert_eq!(ran.len(), 2);
    assert_eq!(ran[1].held, [PathBuf::from(TEST_FILE)]);
    assert_eq!(
        std::fs::read_to_string(rig.worktree.join(TEST_FILE)).unwrap(),
        TEST_TEXT,
        "the worker's committed file is the worker's, and stays"
    );
    assert!(!rig.record().exists());
}

#[test]
fn a_file_of_that_name_with_other_text_is_not_run_over_or_removed() {
    let rig = Rig::new();
    rig.tests.script([failed("no"), Passed]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    let mut checks = rig.before(&found);

    let other = "#[test]\nfn flag_is_read() {}\n";
    std::fs::create_dir_all(rig.worktree.join("tests")).unwrap();
    std::fs::write(rig.worktree.join(TEST_FILE), other).unwrap();
    rig.after(&mut checks, &found);

    let Some(NotRun { reason }) = &checks[0].after else {
        panic!("{checks:?}");
    };
    assert!(reason.contains("another file"), "{reason}");
    assert_eq!(rig.tests.ran().len(), 1, "only the run before the fix");
    assert_eq!(
        std::fs::read_to_string(rig.worktree.join(TEST_FILE)).unwrap(),
        other
    );
}

// A kelpie stopped mid-run leaves the file, and the record of it.
#[test]
fn a_test_left_in_the_worktree_by_a_stopped_run_is_taken_out_and_reported() {
    let rig = Rig::new();
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    std::fs::create_dir(rig.worktree.join("tests")).unwrap();
    std::fs::write(rig.worktree.join(TEST_FILE), TEST_TEXT).unwrap();
    std::fs::write(rig.record(), "tests/flag.rs\ntests\n").unwrap();
    rig.tests.watch(TEST_FILE);
    rig.tests.script([failed("no")]);

    let checks = rig.before(&found);

    assert!(
        !rig.worktree.join("tests").exists(),
        "the file and its folder"
    );
    assert!(!rig.record().exists());
    let notes = rig.said().join("\n");
    assert!(notes.contains("was stopped during a test run"), "{notes}");
    assert!(!notes.contains("dirty"), "{notes}");
    assert_eq!(checks, [the_flag_check(failed("no"), None)]);
    assert_eq!(
        rig.tests.ran().len(),
        1,
        "and the run was made all the same"
    );
}

#[test]
fn a_stranded_file_that_is_not_the_reviewers_text_is_left_and_reported() {
    let rig = Rig::new();
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    std::fs::create_dir(rig.worktree.join("tests")).unwrap();
    std::fs::write(rig.worktree.join(TEST_FILE), "the worker's own\n").unwrap();
    std::fs::write(rig.record(), "tests/flag.rs\ntests\n").unwrap();

    rig.before(&found);

    assert!(
        rig.worktree.join(TEST_FILE).is_file(),
        "not kelpie's to remove"
    );
    assert!(!rig.record().exists());
    let notes = rig.said().join("\n");
    assert!(notes.contains("not the reviewer's text"), "{notes}");
}

#[test]
fn a_run_records_the_test_it_puts_in_the_worktree_until_it_is_taken_out() {
    let rig = Rig::new();
    rig.tests.watch(rig.record());
    rig.tests.script([failed("no")]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);

    rig.before(&found);

    assert_eq!(
        rig.tests.ran()[0].held,
        vec![rig.record()],
        "the record stood as the test ran"
    );
    assert!(!rig.record().exists(), "gone once the test is out");
}

#[test]
fn a_round_reads_back_only_its_own_checks() {
    let rig = Rig::new();
    rig.tests.script([failed("no")]);
    let found = rig.attached(TEST_FILE, TEST_TEXT);
    let checks = rig.before(&found);

    assert_eq!(round_checks(&checks, 2, &found), checks);
    assert!(round_checks(&checks, 3, &found).is_empty());
    assert!(round_checks(&checks, 2, &[finding()]).is_empty());
}

#[test]
fn a_checked_test_is_copied_to_the_kept_folder_and_the_reviewers_stays() {
    let rig = Rig::new();

    let found = rig.attached(TEST_FILE, TEST_TEXT);

    assert_eq!(found[0].test.as_deref(), Some(TEST_FILE));
    assert_eq!(
        std::fs::read_to_string(rig.kept(TEST_FILE)).unwrap(),
        TEST_TEXT
    );
    assert!(tests_folder(&rig.build).join(TEST_FILE).is_file());
    assert_eq!(
        kept_folder(&rig.worker, Rig::ticket()),
        rig.worker.join(format!("{TESTS_FOLDER}-7"))
    );
    assert!(rig.said().is_empty(), "{:?}", rig.said());
}

#[test]
fn a_missing_file_costs_its_finding_only_the_test() {
    let rig = Rig::new();
    let folders = rig.folders();

    let found = checked_tests(
        true,
        &folders,
        vec![finding_testing(TEST_FILE)],
        &mut rig.notes.borrow_mut(),
    );

    assert_eq!(found.len(), 1, "the finding stays");
    assert_eq!(found[0].test, None);
    assert_eq!(
        found[0].why, "a caller sees nothing|tests/flag.rs",
        "a pipe in the why can read as a path"
    );
    let notes = rig.said().join("\n");
    assert!(notes.contains("the reviewer left no such file"), "{notes}");
    assert!(notes.contains("src/lib.rs:3"), "{notes}");
    assert!(!rig.kept(TEST_FILE).exists());
}

#[cfg(unix)]
#[test]
fn a_linked_test_file_is_refused() {
    let rig = Rig::new();
    let secret = rig.build.join("secret.txt");
    std::fs::write(&secret, "not a test\n").unwrap();
    let at = tests_folder(&rig.build).join(TEST_FILE);
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&secret, &at).unwrap();
    let folders = rig.folders();

    let found = checked_tests(
        true,
        &folders,
        vec![finding_testing(TEST_FILE)],
        &mut rig.notes.borrow_mut(),
    );

    assert_eq!(found.len(), 1, "the finding stays");
    assert_eq!(found[0].test, None);
    assert!(!rig.kept(TEST_FILE).exists(), "nothing was copied");
    assert_eq!(rig.said().len(), 1, "{:?}", rig.said());
    assert!(rig.said()[0].contains("is dropped and the finding stays"));
}

#[test]
fn a_path_the_worktree_already_holds_costs_its_finding_only_the_test() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.worktree.join("tests")).unwrap();
    std::fs::write(rig.worktree.join(TEST_FILE), "already here\n").unwrap();

    let found = rig.attached(TEST_FILE, TEST_TEXT);

    assert_eq!(found[0].test, None);
    let notes = rig.said().join("\n");
    assert!(
        notes.contains("already holds a file of that name"),
        "{notes}"
    );
    assert!(!rig.kept(TEST_FILE).exists());
}

#[test]
fn a_round_not_asked_for_tests_reads_the_fifth_field_as_part_of_why() {
    let rig = Rig::new();
    let folders = rig.folders();

    let found = checked_tests(
        false,
        &folders,
        vec![finding_testing(TEST_FILE)],
        &mut rig.notes.borrow_mut(),
    );

    assert_eq!(found[0].test, None);
    assert_eq!(found[0].why, "a caller sees nothing|tests/flag.rs");
    assert!(rig.said().is_empty());
}

#[test]
fn a_name_a_command_would_read_as_an_option_is_recorded_not_run() {
    let rig = Rig::new();
    let found = rig.attached("tests/--help.rs", TEST_TEXT);

    let checks = rig.before(&found);

    let NotRun { reason } = &checks[0].before else {
        panic!("{:?}", checks[0]);
    };
    assert!(reason.contains("tests/--help.rs"), "{reason}");
    assert!(rig.tests.ran().is_empty(), "no command was built from it");
}

#[test]
fn a_test_that_passed_before_the_fix_and_fails_after_it_blocks_nothing() {
    for before in [
        Passed,
        NotRun {
            reason: "the worktree is not at the head the reviewer read".into(),
        },
    ] {
        let rig = Rig::new();
        rig.tests.script([failed("after")]);
        let found = rig.attached(TEST_FILE, TEST_TEXT);
        let mut checks = vec![the_flag_check(before, None)];

        let (_, first) = rig.after(&mut checks, &found);
        let (_, second) = rig.after(&mut checks, &found);

        assert_eq!(first, Next::Done);
        assert_eq!(second, Next::Done);
        assert!(!checks[0].sent_back);
        assert!(still_failing(&checks, 2, &found).is_empty());
    }
}

#[test]
fn a_record_that_names_no_path_inside_the_worktree_is_reported_once_and_removed() {
    for text in ["../x\n", "\n"] {
        let rig = Rig::new();
        std::fs::write(rig.record(), text).unwrap();
        // No test to run, so nothing overwrites the record.
        rig.before(&[finding()]);
        rig.before(&[finding()]);

        assert!(!rig.record().exists(), "the bad record is taken out");
        let notes = rig.said();
        assert_eq!(
            notes.iter().filter(|n| n.contains("test-placed-7")).count(),
            1,
            "{notes:?}"
        );
    }
}
