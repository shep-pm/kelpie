//! The review pass the gate runs, on the gate's rig
//!
//! Each test ends a worker on a ticket with its work pushed, has the gate read a listing
//! that shows CI green, and follows what the pass does: the reads, the fix turn the findings
//! become, the ruling a pass cannot settle, and the merge yes it ends at. The reviewers are
//! [`rig::Reader`]s, so no test needs `gh`, a model, the network or a wait.
//!
//! The cases lift SK's `runner/review/tests/` (`mod.rs`, `disputes_tests.rs`,
//! `second_look.rs`) where they hold for a session reviewer (`docs/lifted.md`).

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use kelpie_engine::board::{self, Column};
use kelpie_engine::ci::{FailedCheck, FailedLog, HeadStatus};
use kelpie_engine::github::Timestamp;
use kelpie_engine::review::{self, Finding, ReviewerError, Severity};
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::settings::Field;
use kelpie_engine::ticket::{Listed, TicketId};
use kelpie_engine::worker::{Seat, Status};
use serde_json::json;

use crate::gate::tests::rig::Rig;

mod failing;

/// The ticket the tests work on
const TICKET: &str = "7";

/// A ticket the rig's forge has no issue for, so its worker has no ticket text to start from
const NO_ISSUE: &str = "9";

/// How long the gate waits for CI's checks to register
const SETTLE: u64 = 120;

/// The label that makes a listed issue a ticket
const READY: &str = "ready-for-agent";

/// The reviewer a project lists that reads every diff, and the second of them
const FIRST: &str = "kelpie:reviewer";
const SECOND: &str = "second";

/// A rig whose worker's turns say one line and then wait
///
/// kelpie's own reviewer has a second look, which [`a_reviewer_with_second_look_reads_twice_before_one_fix_turn`]
/// is about. The rest read it once, so the rig's `reviewer.md` turns the second look off and
/// one read is one round.
fn rig() -> Rig {
    let rig = Rig::new(FakeAgent::new().turn(Turn::new().text("Fixed.").until_cancelled()));
    rig.reviewer_agent("reviewer", "\nsecond_look: false");
    rig
}

/// The project lists two reviewers, so a pass owes the two reads its line counts to
fn two_reviewers(rig: &Rig) {
    rig.reviewers_listed(&[FIRST, SECOND]);
    rig.reviewer_agent(SECOND, "");
}

/// A finding a reviewer raises: `file:line` says `what`, seen as `why`
fn finding(severity: Severity, file: &str, line: u32, what: &str) -> Finding {
    Finding::new(severity, file, line, what, "the maintainer sees it")
}

/// The text of `at`'s prompt to the worker's agent, `None` before it arrives
fn prompt(rig: &Rig, at: usize) -> Option<String> {
    rig.agent
        .params("session/prompt")
        .into_iter()
        .nth(at)
        .and_then(|params| params["prompt"][0]["text"].as_str().map(str::to_owned))
}

/// Wait for `ask` to answer, and return what it answered
fn until_some<T>(mut ask: impl FnMut() -> Option<T>) -> T {
    let since = Instant::now();
    loop {
        if let Some(found) = ask() {
            return found;
        }
        assert!(
            since.elapsed() < Duration::from_secs(20),
            "it never happened"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Wait for `ask` to answer true
fn until(mut ask: impl FnMut() -> bool) {
    let since = Instant::now();
    while !ask() {
        assert!(
            since.elapsed() < Duration::from_secs(20),
            "it never happened"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// Read `head` green until its checks have had their time to register
///
/// A head is read twice: the first read starts the wait for its checks, and the second,
/// [`SETTLE`] later, is the one the pass runs on.
fn read_green(rig: &Rig, head: &str) {
    read_green_on(rig, TICKET, head);
}

/// [`read_green`] on `ticket`
fn read_green_on(rig: &Rig, ticket: &str, head: &str) {
    let pull = rig.pull(ticket, head, HeadStatus::Passed);
    rig.read(std::slice::from_ref(&pull));
    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(&pull));
}

/// Ticket 7's worker has ended with `file` pushed, on a head CI is green on
fn green(rig: &Rig, file: &str) -> String {
    let head = rig.worker_on(TICKET, file, "mine\n");
    read_green(rig, &head);
    head
}

/// What the pass keeps of ticket 7
fn kept(rig: &Rig) -> review::Kept {
    kept_on(rig, TICKET)
}

/// What the pass keeps of `ticket`
fn kept_on(rig: &Rig, ticket: &str) -> review::Kept {
    rig.review()
        .kept(ticket)
        .expect("the pass keeps its facts of the ticket")
}

/// The worker's turn ended with nothing pushed, as a fix turn that left the findings did
fn ended(rig: &Rig) {
    rig.set_status(TICKET, Status::Ended);
}

/// The worker copies the round's findings into the disputed findings file with a reason,
/// as a fix turn that shows each wrong does
fn dispute(rig: &Rig) {
    let build = rig.paths().build_dir(TICKET);
    let findings = fs::read_to_string(review::findings_path(&build)).expect("the findings file");
    let line = (findings.lines())
        .find(|line| line.starts_with("HIGH|"))
        .expect("a finding's line");
    fs::write(
        review::disputed_path(&build),
        format!("{line}\n  the file does not do that\n"),
    )
    .unwrap();
}

/// The column ticket 7's card sits in, as the board's own rule places it
fn column(rig: &Rig, head: &str) -> Column {
    let id = TicketId::from_issue(TICKET.parse().unwrap()).unwrap();
    let read = board::Read {
        open: vec![Listed {
            id,
            title: "Ticket 7".to_owned(),
            labels: vec![READY.to_owned()],
            author: "someone".to_owned(),
            created: Timestamp::from_secs(1_700_000_000),
        }],
        ready: Vec::new(),
        closed: Vec::new(),
        pulls: vec![rig.pull(TICKET, head, HeadStatus::Passed)],
    };
    let rules = board::Rules {
        pick_up: kelpie_engine::settings::PickUp::Everyone,
        ready_label: READY.to_owned(),
        login: "someone".to_owned(),
    };
    board::place(&read, &rig.records(), &rules)
        .tickets()
        .iter()
        .find(|placed| placed.id == id)
        .expect("the ticket is on the board")
        .column
}

/// The line ticket 7's card shows
fn line(rig: &Rig) -> Option<String> {
    rig.lines().get(TICKET).map(kelpie_engine::gate::Line::text)
}

#[test]
fn a_green_head_no_reviewer_read_starts_a_pass_instead_of_the_yes() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong",
    )]));
    let head = green(&rig, "work.txt");

    assert_eq!(
        reader.prompts().len(),
        1,
        "the gate sent the head to a reviewer before the merge yes"
    );
    assert_eq!(
        kept(&rig).heads.read,
        [head],
        "the read was of the head on origin"
    );
    assert!(
        !rig.gate.ready().contains(TICKET),
        "the head was not put up to merge before a reviewer had read it"
    );
    assert!(
        matches!(
            kept(&rig).review.as_ref().expect("the pass").stage,
            review::ReviewStage::Fixing { .. }
        ),
        "what the reviewer found went to the worker before the pass was done"
    );
}

#[test]
fn two_reads_with_a_fix_turn_each_reach_the_yes() {
    let rig = rig();
    two_reviewers(&rig);
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong",
    )]));
    let head = green(&rig, "work.txt");
    assert!(
        !rig.gate.ready().contains(TICKET),
        "the pass is at the worker's fix turn at {head}"
    );

    // The worker fixed it and pushed, which is the head the second reviewer reads
    ended(&rig);
    let fixed = rig.commit(TICKET, "fixed.txt", "fixed\n");
    read_green(&rig, &fixed);

    assert_eq!(
        reader.prompts().len(),
        2,
        "the fix turn sits between the two reviewers' reads"
    );
    assert!(
        kept(&rig).heads.vouches_for(&fixed),
        "the head the fix turn pushed is one the pass vouches for"
    );
    assert!(
        rig.gate.ready().contains(TICKET),
        "and the pass ended at the merge yes"
    );
}

#[test]
fn a_read_holds_a_slot_and_waits_when_none_is_free() {
    let rig = rig();
    rig.set(Field::WorkerSlots, &json!(1));
    // Ticket 8's worker took the project's one slot before ticket 7's pass came to read
    rig.worker_on("8", "other.txt", "theirs\n");
    rig.set_status("8", Status::Running);
    let reader = rig.reader(TICKET);
    let head = green(&rig, "work.txt");

    let record = rig.record(TICKET);
    assert_eq!(
        record.status,
        Status::Reviewing,
        "the read was started, and it needs a slot"
    );
    assert_eq!(record.seat, Seat::Waiting, "and none was free");
    assert!(
        reader.prompts().is_empty(),
        "so no reviewer read the head yet"
    );

    // The slot frees up, and the next read of GitHub runs the read that waited
    rig.set_status("8", Status::Ended);
    read_green(&rig, &head);

    assert_eq!(reader.prompts().len(), 1, "the read that waited ran");
    assert_eq!(
        rig.record(TICKET).status,
        Status::Ended,
        "and the worker is back to holding no slot"
    );
}

#[test]
fn the_card_says_reviewing_read_1_of_2_while_the_first_reader_reads() {
    let rig = rig();
    two_reviewers(&rig);
    let reader = rig.reader(TICKET);
    reader.hold();
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    rig.read(std::slice::from_ref(&pull));
    rig.clock.advance(SETTLE);
    // The pass runs on this read, and its first read is held
    rig.gate
        .after_read(&rig.records(), std::slice::from_ref(&pull));

    until(|| reader.asked());
    assert_eq!(
        line(&rig),
        Some("Reviewing, read 1 of 2".to_owned()),
        "the card says which read of the two the ticket is on"
    );

    reader.release();
    rig.gate.idle();
    assert_eq!(reader.prompts().len(), 2, "and then both reads ran");
    assert_eq!(
        line(&rig),
        Some("Ready to merge".to_owned()),
        "with the pass done, the card is the merge yes"
    );
}

#[test]
fn the_findings_are_the_workers_next_prompt_and_the_ticket_shows_building() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it reads a file it never opens",
    )]));
    let head = green(&rig, "work.txt");

    let sent = until_some(|| prompt(&rig, 0));
    assert!(
        sent.contains("it reads a file it never opens"),
        "the finding is in the worker's next prompt: {sent}"
    );
    assert!(
        sent.contains("review-findings.md"),
        "which names the file the round's findings are in: {sent}"
    );
    let build = rig.paths().build_dir(TICKET);
    assert!(
        review::findings_path(&build).exists(),
        "and the file is there before the turn that reads it starts"
    );

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::Running, "the worker has a turn");
    assert_eq!(record.seat, Seat::Held, "which holds a slot");
    assert_eq!(
        column(&rig, &head),
        Column::Building,
        "so the ticket is in Building, where a live worker puts it"
    );
    assert!(
        !rig.gate.ready().contains(TICKET),
        "and the pass is not at the merge yes yet"
    );
}

#[test]
fn a_red_head_is_a_ci_fix_turn_before_any_read() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let run = "https://github.com/acme/app/actions/runs/42";
    rig.ci.script_logs(
        &head,
        vec![FailedLog {
            check: "test".to_owned(),
            run: 42,
            url: run.to_owned(),
            tail: "FAILED work.txt:2: it reads a file it never opens".to_owned(),
        }],
    );
    let pull = rig.pull(
        TICKET,
        &head,
        HeadStatus::Failed(vec![FailedCheck {
            name: "test".to_owned(),
            url: Some(run.to_owned()),
        }]),
    );
    rig.read(std::slice::from_ref(&pull));
    rig.clock.advance(SETTLE);
    rig.read(std::slice::from_ref(&pull));

    let sent = until_some(|| prompt(&rig, 0));
    assert!(sent.contains("test"), "the red run is the worker's: {sent}");
    assert!(
        reader.prompts().is_empty(),
        "and no reviewer read a head CI is red on"
    );
}

#[test]
fn a_reviewer_with_second_look_reads_twice_before_one_fix_turn() {
    let rig = rig();
    rig.reviewers_listed(&["deep"]);
    rig.reviewer_agent("deep", "\nsecond_look: true");
    let reader = rig.reader(TICKET);
    let first = finding(Severity::High, "work.txt", 2, "it is wrong");
    let more = finding(Severity::Medium, "other.txt", 1, "and this is too");
    reader.script_read(Ok(vec![first.clone()]));
    reader.script_read(Ok(vec![more.clone()]));
    green(&rig, "work.txt");

    let prompts = reader.prompts();
    assert_eq!(
        prompts.len(),
        2,
        "a fresh session read again after its first"
    );
    assert!(
        prompts[1].contains(&first.what),
        "the second look is shown what the first found: {}",
        prompts[1]
    );

    let sent = until_some(|| prompt(&rig, 0));
    assert!(sent.contains(&first.what), "{sent}");
    assert!(sent.contains(&more.what), "{sent}");
    assert_eq!(
        prompt(&rig, 1),
        None,
        "and both looks go to the worker as one fix turn"
    );
}

#[test]
fn a_fix_that_pushes_nothing_rules_fix_not_pushed_on_the_card() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    let head = green(&rig, "work.txt");
    let sent = until_some(|| prompt(&rig, 0));
    assert!(
        sent.contains("it is wrong, and it is in scope"),
        "the worker was sent the finding: {sent}"
    );

    // Its turn ended with nothing pushed and nothing said of the finding
    ended(&rig);
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    let reason = record.reason.expect("the ruling says why");
    assert!(reason.contains("pushed nothing"), "{reason}");
    assert_eq!(
        line(&rig),
        Some(reason),
        "and the ticket's card is the review's ruling"
    );
    assert_eq!(
        reader.prompts().len(),
        1,
        "the pass ruled instead of reading again"
    );
}

#[test]
fn a_finding_disputed_again_after_a_later_reader_raised_it_rules_readers_disagree() {
    let rig = rig();
    two_reviewers(&rig);
    let reader = rig.reader(TICKET);
    let raised = finding(Severity::High, "work.txt", 2, "it is wrong");
    reader.script_read(Ok(vec![raised.clone()]));
    reader.script_read(Ok(vec![raised.clone()]));
    let head = green(&rig, "work.txt");

    // The worker showed the finding wrong and pushed nothing. The second reviewer
    // then raised it again, and went back to the worker with it.
    dispute(&rig);
    ended(&rig);
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);
    assert_eq!(reader.prompts().len(), 2, "the second reviewer read it");
    assert_eq!(
        kept(&rig).disputes.len(),
        1,
        "and the worker's dispute is kept"
    );

    // The worker disputed nothing further and pushed nothing again
    ended(&rig);
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    let reason = record.reason.expect("the ruling says why");
    assert!(reason.contains("disagree"), "{reason}");
    assert!(reason.contains("work.txt:2"), "{reason}");
    assert_eq!(line(&rig), Some(reason), "and the card says it too");
}

#[test]
fn a_read_in_flight_at_a_relaunch_reads_again_with_the_same_reviewer() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    // kelpie stopped with the read in flight, which is what a relaunch does to one
    reader.script_read(Err(ReviewerError::Stopped));
    let head = green(&rig, "work.txt");

    let facts = kept(&rig);
    assert_eq!(
        facts.review.as_ref().expect("the pass").reviewer.as_deref(),
        Some(FIRST),
        "the round still names the reviewer it had"
    );
    assert!(
        facts.heads.read.is_empty(),
        "a stopped read counts for nothing"
    );
    assert_eq!(reader.prompts().len(), 1, "and it is not counted as a read");

    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    assert_eq!(reader.prompts().len(), 2, "the same reviewer reads again");
    assert_eq!(kept(&rig).heads.read, [head], "and the head is read");
    assert_eq!(
        kept(&rig).review.expect("the pass").ran,
        [FIRST],
        "once, with the reviewer the round chose"
    );
}

#[test]
fn a_catch_up_of_a_read_head_needs_no_new_read() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    let head = green(&rig, "work.txt");
    assert!(kept(&rig).heads.reviewed(&head), "the head was read");

    // The worker's turn ended with nothing pushed, and the base moved under the branch
    ended(&rig);
    rig.main_moves("main.txt", "main\n");
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    let caught = rig.origin("kelpie/7");
    assert_ne!(caught, head, "the gate caught the branch up and pushed it");
    assert!(
        kept(&rig).heads.vouches_for(&caught),
        "and the head it pushed is vouched for, as the read head it was rebased from"
    );

    rig.read(&[rig.pull(TICKET, &caught, HeadStatus::Passed)]);
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &caught, HeadStatus::Passed)]);

    assert_eq!(
        reader.prompts().len(),
        1,
        "no reviewer was sent to read the caught-up head"
    );
    assert!(
        rig.gate.ready().contains(TICKET),
        "and the merge yes is on the caught-up head"
    );
}

/// What a pass hands the gate when it ends: the same `Review` the gate steps on
#[test]
fn a_pass_hands_the_ticket_back_to_the_gate_that_holds_it() {
    let rig = rig();
    let head = green(&rig, "work.txt");
    let review = rig.review();
    assert_eq!(
        review.looked(TICKET, &head, &["work.txt".to_owned()]).of,
        kelpie_engine::review::RULE_READS,
        "the pass says how many reads it owes"
    );
    assert!(
        !review
            .looked(TICKET, &head, &["work.txt".to_owned()])
            .can_run,
        "and that it has had all it can"
    );
    assert!(
        rig.gate.ready().contains(TICKET),
        "which is what lets the gate give the yes"
    );
}

/// A rig whose worker's turns end on their own, so a pass can send it a second fix turn
fn rig_that_fixes() -> Rig {
    let rig = Rig::new(
        FakeAgent::new()
            .turn(Turn::new().text("fixed"))
            .turn(Turn::new().text("fixed again").until_cancelled()),
    );
    rig.reviewer_agent("reviewer", "\nsecond_look: false");
    rig
}

/// Each round's test gets its run after its own fix, and an earlier round's run is not set again
#[test]
fn a_later_rounds_test_gets_its_run_after_its_own_fix() {
    let rig = rig_that_fixes();
    rig.test_command("cargo test --test {name}");
    two_reviewers(&rig);
    let reader = rig.reader(TICKET);
    // Both reviewers' tests are written as the first reads, and the second keeps its own
    reader.attach(
        "tests/first.rs",
        "#[test]\nfn first() { assert!(false); }\n",
    );
    reader.attach(
        "tests/second.rs",
        "#[test]\nfn second() { assert!(false); }\n",
    );
    reader.script_read(Ok(vec![
        finding(Severity::High, "work.txt", 2, "it is wrong").with_test("tests/first.rs"),
    ]));
    reader.script_read(Ok(vec![
        finding(Severity::High, "fixed.txt", 3, "and so is this").with_test("tests/second.rs"),
    ]));
    green(&rig, "work.txt");

    // The worker fixes round 1, and the second reviewer reads the head it pushed
    ended(&rig);
    let fixed = rig.commit(TICKET, "fixed.txt", "fixed\n");
    read_green(&rig, &fixed);

    // The worker fixes round 2, whose test has its run at the head that fix moved
    ended(&rig);
    let last = rig.commit(TICKET, "last.txt", "last\n");
    read_green(&rig, &last);

    let runs = kept(&rig)
        .checks
        .iter()
        .map(|check| (check.round, check.after.is_some()))
        .collect::<Vec<_>>();
    assert_eq!(
        runs,
        [(1, true), (2, true)],
        "each round's test has its run before the fix and after its own fix"
    );
    assert!(
        rig.gate.ready().contains(TICKET),
        "and the pass reached the merge yes instead of spinning on a run it never recorded"
    );
}

/// A fix turn the worker was never sent leaves the pass at its findings, not at a fix to rule on
#[test]
fn a_fix_turn_that_cannot_start_leaves_the_findings_for_the_next_read() {
    let rig = rig();
    let reader = rig.reader(NO_ISSUE);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    // The project has no issue 9, so its worker has no ticket text to start a turn from
    let head = rig.worker_on(NO_ISSUE, "work.txt", "mine\n");
    read_green_on(&rig, NO_ISSUE, &head);

    let pass = kept_on(&rig, NO_ISSUE).review.expect("the pass");
    assert!(
        matches!(pass.stage, review::ReviewStage::Found { .. }),
        "the pass is kept at the findings it could not send, not at the fix turn it never sent"
    );
    let record = rig.record(NO_ISSUE);
    assert_eq!(record.status, Status::Ended, "the worker was never sent");
    assert!(record.reason.is_none(), "and nothing is held against it");

    // The next read runs the same step again rather than ruling a fix that pushed nothing
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(NO_ISSUE, &head, HeadStatus::Passed)]);

    let record = rig.record(NO_ISSUE);
    assert_ne!(
        record.status,
        Status::WaitingOnMaintainer,
        "no ruling of a fix that pushed nothing: {:?}",
        record.reason
    );
    assert_eq!(
        reader.prompts().len(),
        1,
        "and the reviewer read once, rather than being sent back to read again"
    );
}

/// A pass whose reviewers cannot be read holds the ticket off the merge yes
#[test]
fn a_worker_plan_that_cannot_be_read_holds_the_ticket_off_the_merge_yes() {
    let rig = rig();
    // The project's own worker.md cannot be used, so the pass cannot read its reviewers
    fs::write(
        rig.agent_file("worker"),
        "A worker with no frontmatter at all.\n",
    )
    .unwrap();
    let head = green(&rig, "work.txt");

    assert!(
        rig.review()
            .looked(TICKET, &head, &["work.txt".to_owned()])
            .can_run,
        "a pass that cannot read its reviewers still owes its reads"
    );
    assert!(
        !rig.gate.ready().contains(TICKET),
        "so the gate does not put up a head no reviewer read"
    );
    assert_eq!(
        line(&rig),
        Some("Reviewing, read 1 of 2".to_owned()),
        "and its card says the pass is at work, not ready to merge"
    );
    assert!(
        rig.reader(TICKET).prompts().is_empty(),
        "and no reviewer read it"
    );
}

/// A read in flight holds no lock another ticket's pass needs
#[test]
fn a_read_in_flight_leaves_another_tickets_pass_free() {
    let rig = rig();
    let held = rig.reader(TICKET);
    held.hold();
    held.script_read(Ok(Vec::new()));
    let seven = rig.worker_on(TICKET, "work.txt", "mine\n");
    let eight = rig.worker_on("8", "other.txt", "theirs\n");

    // Ticket 7's pass reaches its read and is held there. Its first read only starts the
    // wait for its checks, and the pass must be done before the second one is asked for
    rig.gate.after_read(
        &rig.records(),
        &[rig.pull(TICKET, &seven, HeadStatus::Passed)],
    );
    rig.gate.idle();
    rig.clock.advance(SETTLE);
    rig.gate.after_read(
        &rig.records(),
        &[rig.pull(TICKET, &seven, HeadStatus::Passed)],
    );
    until(|| held.asked());

    // Ticket 8's pass runs while 7's read is still in flight
    rig.gate
        .after_read(&rig.records(), &[rig.pull("8", &eight, HeadStatus::Passed)]);
    let since = Instant::now();
    while !rig.lines().contains_key("8") {
        assert!(
            since.elapsed() < Duration::from_secs(5),
            "ticket 8's pass is held behind ticket 7's read"
        );
        thread::sleep(Duration::from_millis(20));
    }

    held.release();
    rig.gate.idle();
    assert_eq!(held.prompts().len(), 1, "and ticket 7's read ran on");
}
