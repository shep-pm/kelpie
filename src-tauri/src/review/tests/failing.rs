//! A reviewer's failing test, run at the head it read and after the fix
//!
//! The project's Test command is set, so a reviewer is asked for a failing test of its own and
//! the pass runs it twice: at the head the reviewer read, and at the fix's head. The runs go
//! through [`crate::review::SandboxedTests`], whose launch the rig stands in for the sandbox
//! and for `kelpie lease run`, neither of which a unit test can start. Each run is scripted to
//! exit with a number and some output, so no test needs `gh`, a model, the network or a wait.
//!
//! The cases lift SK's `runner/review/checks/tests.rs` and the kept tests of
//! `runner/review/tests/mod.rs` where they hold for the app's pass (`docs/lifted.md`).

use std::fs;
use std::sync::Arc;

use kelpie_engine::review::{self, CEILING, Finding, Reading, Severity, TestOutcome};
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::ticket::TicketId;
use kelpie_engine::worker::Status;

use super::{TICKET, finding, green, kept, prompt, rig, two_reviewers, until_some};
use crate::gate::tests::rig::{Reader, Rig};
use crate::review::{Exit, KEPT};

/// What the project's Test command is: one integration test file, by name
const COMMAND: &str = "cargo test --test {name}";

/// The test the reviewer writes, at the path it should have in the repo
const FLAG: &str = "tests/flag.rs";

/// What a reviewer's failing test says
const FAILS: &str = "#[test]\nfn flag_is_read() { assert!(false); }\n";

/// An exit of `code` after writing `text`
fn exit(code: u8, text: &str) -> Exit {
    Exit {
        code,
        output: text.as_bytes().to_vec(),
    }
}

/// The finding the reviewer attaches its test to
fn flagged() -> Finding {
    finding(Severity::High, "work.txt", 2, "the flag is read first").with_test(FLAG)
}

/// A rig whose worker ends each turn it is sent, as a worker does that fixes and pushes
///
/// The pass sends the worker a second turn when a test still fails, and a worker whose first
/// turn never ended would only have it queued behind that one.
fn rig_ending_its_turns() -> Rig {
    let rig = Rig::new(
        FakeAgent::new()
            .turn(Turn::new().text("Fixed."))
            .turn(Turn::new().text("Fixed again."))
            .turn(Turn::new().text("Fixed once more.")),
    );
    rig.reviewer_agent("reviewer", "\nsecond_look: false");
    rig
}

/// A rig whose project asks for tests, and whose reviewer attaches [`FAILS`] to its one finding
fn tested() -> (Rig, Arc<Reader>) {
    let rig = rig_ending_its_turns();
    rig.test_command(COMMAND);
    let reader = rig.reader(TICKET);
    reader.attach(FLAG, FAILS);
    reader.script_read(Ok(vec![flagged()]));
    (rig, reader)
}

/// The worker's fix turn ended with `file` pushed, and GitHub shows the new head green
fn fixed(rig: &Rig, file: &str) -> String {
    rig.set_status(TICKET, Status::Ended);
    let head = rig.commit(TICKET, file, "fixed\n");
    super::read_green(rig, &head);
    head
}

/// The one check the pass keeps of the ticket's test
fn check(rig: &Rig) -> review::TestCheck {
    let checks = kept(rig).checks;
    assert_eq!(checks.len(), 1, "{checks:?}");
    checks[0].clone()
}

/// Where the kept copies of ticket 7's tests are
fn kept_folder(rig: &Rig) -> std::path::PathBuf {
    let ticket: TicketId = TICKET.parse().unwrap();
    review::kept_folder(&rig.paths().project().join(KEPT), ticket)
}

#[test]
fn a_test_that_fails_before_and_passes_after_is_verified_and_blocks_nothing() {
    let (rig, reader) = tested();
    rig.launches.watch(FLAG);
    rig.launches
        .script([exit(101, "assertion failed"), exit(0, "ok")]);
    green(&rig, "work.txt");

    let before = check(&rig);
    assert_eq!(
        before.before,
        TestOutcome::Failed {
            output: "assertion failed".to_owned()
        },
        "the test ran at the head the reviewer read, and failed"
    );
    assert_eq!(before.reading(), Reading::Waiting, "the fix has not run");
    assert!(
        prompt(&rig, 0).is_some_and(|sent| sent.contains(FLAG)),
        "the worker was sent the finding with its test"
    );

    fixed(&rig, "fixed.txt");

    let verified = check(&rig);
    assert_eq!(verified.after, Some(TestOutcome::Passed));
    assert_eq!(verified.reading(), Reading::Verified);
    let ran = rig.launches.ran();
    assert_eq!(ran.len(), 2, "one run before the fix and one after it");
    assert!(
        ran.iter()
            .all(|run| run.held == [std::path::PathBuf::from(FLAG)]),
        "the kept test was in the worktree while each ran: {ran:?}"
    );
    assert!(
        !rig.worktree(TICKET).join(FLAG).exists(),
        "and it was taken out again"
    );
    assert!(
        rig.gate.ready().contains(TICKET),
        "a verified test blocks nothing, so the pass ended at the merge yes"
    );
    assert_eq!(reader.prompts().len(), 1);
}

#[test]
fn a_test_still_failing_sends_the_worker_back_once_naming_the_test_and_its_output() {
    let (rig, _reader) = tested();
    rig.launches.script([
        exit(101, "left: 1"),
        exit(101, "thread 'flag_is_read' panicked: the flag is not read"),
    ]);
    green(&rig, "work.txt");
    fixed(&rig, "fixed.txt");

    let back = until_some(|| prompt(&rig, 1));
    assert!(back.contains("still failing"), "{back}");
    assert!(back.contains(FLAG), "it names the test: {back}");
    assert!(
        back.contains("the flag is not read"),
        "and what the run said: {back}"
    );
    let sent = check(&rig);
    assert!(sent.sent_back, "the worker is sent back once");
    assert_eq!(
        sent.reading(),
        Reading::Waiting,
        "and the run of the second fix is waited for"
    );
    assert_eq!(
        prompt(&rig, 2),
        None,
        "nothing more is sent before the worker's second fix"
    );
    assert!(
        !rig.gate.ready().contains(TICKET),
        "the pass waits on the second fix turn"
    );
}

#[test]
fn a_test_failing_after_the_second_fix_rules_test_still_fails() {
    let (rig, reader) = tested();
    rig.launches.script([
        exit(101, "left: 1"),
        exit(101, "left: 2"),
        exit(101, "left: 3"),
    ]);
    green(&rig, "work.txt");
    fixed(&rig, "fixed.txt");
    until_some(|| prompt(&rig, 1));
    fixed(&rig, "again.txt");

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    let reason = record.reason.expect("the ruling says why");
    assert!(reason.contains("still fail"), "{reason}");
    assert!(reason.contains(FLAG), "{reason}");
    assert_eq!(rig.launches.ran().len(), 3, "each fix was run once");
    assert_eq!(
        reader.prompts().len(),
        1,
        "the pass ruled instead of reading again"
    );
    assert_eq!(prompt(&rig, 2), None, "and sent the worker back only once");
}

#[test]
fn a_run_takes_its_turn_in_the_cargo_test_lease() {
    let (rig, _reader) = tested();
    rig.launches.script([exit(101, "no"), exit(0, "ok")]);
    green(&rig, "work.txt");
    fixed(&rig, "fixed.txt");

    let ran = rig.launches.ran();
    assert_eq!(ran.len(), 2);
    let doors = rig.doors.lock().unwrap().clone();
    assert_eq!(doors.len(), 2, "each run is given a lease of its own");
    for (run, door) in ran.iter().zip(&doors) {
        assert_eq!(
            &run.door, door,
            "the run queued on the door of the lease it was given"
        );
        assert_eq!(run.cwd, rig.worktree(TICKET), "in the ticket's worktree");
        assert_eq!(
            run.script, "exec 2>&1; cargo test --test flag",
            "as the project's command, whatever it runs"
        );
    }
}

#[test]
fn a_run_past_its_ceiling_is_timed_out_and_blocks_nothing() {
    let (rig, _reader) = tested();
    rig.launches.script([exit(101, "no"), exit(124, "")]);
    green(&rig, "work.txt");
    fixed(&rig, "fixed.txt");

    let check = check(&rig);
    assert_eq!(check.after, Some(TestOutcome::TimedOut));
    assert_eq!(check.reading(), Reading::Unknown);
    assert!(
        rig.launches.ran().iter().all(|run| run.ceiling == CEILING),
        "each run could run for the ceiling once it held the lease"
    );
    assert!(
        rig.gate.ready().contains(TICKET),
        "a run that said nothing blocks nothing"
    );
    assert_eq!(prompt(&rig, 1), None, "and the worker is not sent back");
}

#[test]
fn with_no_test_command_no_test_is_asked_for_or_run() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    // A reviewer that writes a test nobody asked for
    reader.attach(FLAG, FAILS);
    reader.script_read(Ok(vec![flagged()]));
    green(&rig, "work.txt");

    let prompts = reader.prompts();
    assert!(
        !prompts[0].contains(review::TESTS_FOLDER),
        "no reviewer is asked for a test: {}",
        prompts[0]
    );
    assert!(rig.launches.ran().is_empty(), "nothing ran");
    assert!(kept(&rig).checks.is_empty(), "and nothing is kept of a run");
    assert!(!kept_folder(&rig).join(FLAG).exists(), "or of a test");
    let sent = until_some(|| prompt(&rig, 0));
    assert!(sent.contains("the flag is read first"), "{sent}");
}

#[test]
fn an_attached_file_is_kept_for_its_finding() {
    let (rig, _reader) = tested();
    green(&rig, "work.txt");

    let review = kept(&rig).review.expect("the pass");
    let review::ReviewStage::Fixing { sent, .. } = review.stage else {
        panic!("the findings were sent: {review:?}");
    };
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].test.as_deref(), Some(FLAG));
    assert_eq!(sent[0].why, "the maintainer sees it");
    let held = fs::read_to_string(kept_folder(&rig).join(FLAG)).unwrap();
    assert_eq!(held, FAILS, "the copy the runs read is the reviewer's text");
    let original = review::tests_folder(&rig.paths().build_dir(TICKET)).join(FLAG);
    assert_eq!(
        fs::read_to_string(original).unwrap(),
        held,
        "the original stays where the worker reads it"
    );
    let findings = fs::read_to_string(review::findings_path(&rig.paths().build_dir(TICKET)))
        .expect("the findings file");
    assert!(
        findings.contains(&format!("|{FLAG}")),
        "the worker's findings name the test: {findings}"
    );
}

#[test]
fn a_later_reviewers_first_look_keeps_the_tests_kept_before() {
    let (rig, reader) = tested();
    two_reviewers(&rig);
    rig.launches.script([exit(101, "no"), exit(0, "ok")]);
    green(&rig, "work.txt");
    let held = kept_folder(&rig).join(FLAG);
    assert!(held.is_file(), "the first reviewer's test is kept");

    fixed(&rig, "fixed.txt");

    assert_eq!(reader.prompts().len(), 2, "the second reviewer read");
    assert!(held.is_file(), "its first look keeps what was kept before");
    assert_eq!(check(&rig).reading(), Reading::Verified);
}

#[test]
fn a_missing_file_costs_its_finding_only_the_test() {
    let rig = rig();
    rig.test_command(COMMAND);
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![flagged()]));
    green(&rig, "work.txt");

    let review::ReviewStage::Fixing { sent, .. } = kept(&rig).review.expect("the pass").stage
    else {
        panic!("the findings were sent");
    };
    assert_eq!(sent[0].test, None);
    assert_eq!(sent[0].what, "the flag is read first");
    assert_eq!(
        sent[0].why, "the maintainer sees it|tests/flag.rs",
        "the reviewer may have meant a pipe in its why, so no word of it is lost"
    );
    assert!(!kept_folder(&rig).join(FLAG).exists());
    assert!(rig.launches.ran().is_empty());
}

#[test]
fn a_path_the_worktree_already_holds_costs_its_finding_only_the_test() {
    let rig = rig();
    rig.test_command(COMMAND);
    let reader = rig.reader(TICKET);
    // `work.txt` is on the branch the reviewer reads
    reader.attach("work.txt", "not the work\n");
    reader.script_read(Ok(vec![
        finding(Severity::High, "work.txt", 2, "the flag is read first").with_test("work.txt"),
    ]));
    green(&rig, "work.txt");

    let review::ReviewStage::Fixing { sent, .. } = kept(&rig).review.expect("the pass").stage
    else {
        panic!("the findings were sent");
    };
    assert_eq!(sent[0].test, None);
    assert!(!kept_folder(&rig).join("work.txt").exists());
    assert_eq!(
        fs::read_to_string(rig.worktree(TICKET).join("work.txt")).unwrap(),
        "mine\n",
        "the branch's file is as it was"
    );
    assert!(rig.launches.ran().is_empty());
}

#[test]
fn a_run_whose_settings_cannot_be_made_is_not_run_and_blocks_nothing() {
    let (rig, _reader) = tested();
    rig.runs_unreachable("the sandbox has no policy for it");
    green(&rig, "work.txt");
    fixed(&rig, "fixed.txt");

    let check = check(&rig);
    assert!(
        matches!(&check.before, TestOutcome::NotRun { reason } if reason.contains("no policy")),
        "{check:?}"
    );
    assert_eq!(check.reading(), Reading::Unknown);
    assert!(rig.launches.ran().is_empty(), "no command was started");
    assert!(
        rig.gate.ready().contains(TICKET),
        "a run that never ran blocks nothing"
    );
}
