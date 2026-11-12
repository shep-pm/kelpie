//! Several tickets at once, and what the outside world sees of them
//!
//! Each case starts real workers through [`crate::workers::Workers`] on a
//! [`FakeAgent`] each, with the project's GitHub stood in by [`super::forge::Github`],
//! and asserts only what a person could see: the board's columns, the pull requests
//! opened, and the prompts each worker got. No case needs `gh`, the network or a model.

use kelpie_engine::ci::HeadStatus;
use kelpie_engine::gate::FORGE_CLOSE_WAIT;
use kelpie_engine::review::{Finding, Severity};
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::ticket::TicketId;
use kelpie_engine::worker::Status;

use super::{Rig, SETTLE, git, lock};
use crate::board::Column;
use crate::review::Exit;
use crate::workers::{NoteOutcome, WorkerStarted};

/// A reviewer's finding of `file:line`, for scripting one ticket's reader
fn finding(file: &str, line: u32, what: &str) -> Finding {
    Finding::new(Severity::High, file, line, what, "the maintainer sees it")
}

/// A worker whose first turn ends and whose second, the review's fix turn, keeps working
fn worker_that_fixes() -> FakeAgent {
    FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("fixing").until_cancelled())
}

#[test]
fn two_tickets_started_at_once_each_reach_review_with_a_draft_of_their_own() {
    let rig = Rig::new(2);
    rig.ticket(12, "Add the thing", "It does one.", &FakeAgent::new());
    rig.ticket(13, "Fix the other", "It does two.", &FakeAgent::new());
    assert_eq!(
        rig.columns(),
        [("12".into(), Column::Ready), ("13".into(), Column::Ready)],
        "both tickets wait on the board before a worker starts",
    );

    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);

    let pulls = rig.github.pulls();
    let branches: Vec<_> = pulls.iter().map(|pull| pull.branch.as_str()).collect();
    assert!(
        branches.contains(&"kelpie/12") && branches.contains(&"kelpie/13"),
        "a draft from each worker's own branch: {branches:?}",
    );
    for (ticket, title) in [(12, "feat: Add the thing"), (13, "feat: Fix the other")] {
        let pull = pulls
            .iter()
            .find(|pull| pull.branch == format!("kelpie/{ticket}"))
            .unwrap();
        assert_eq!(
            pull.title, title,
            "each draft is from its own worker's file"
        );
        assert!(pull.draft, "kelpie opens a draft");
        assert_eq!(
            pull.body.lines().last(),
            Some(format!("Resolves #{ticket}").as_str()),
            "and it closes its own ticket",
        );
    }
    assert_eq!(
        rig.columns(),
        [("12".into(), Column::Review), ("13".into(), Column::Review)],
        "both tickets sit in Review",
    );
}

#[test]
fn with_one_slot_the_second_ticket_waits_and_starts_when_the_first_turn_ends() {
    let rig = Rig::new(1);
    let first = FakeAgent::new()
        .turn(Turn::new().text("working").until_cancelled())
        .turn(Turn::new().text("done"));
    let second = FakeAgent::new();
    rig.ticket(12, "Add the thing", "It does one.", &first);
    rig.ticket(13, "Fix the other", "It does two.", &second);
    rig.start(12);
    rig.until_working(12);

    let waiting = rig.start(13);

    assert_eq!(
        waiting,
        WorkerStarted {
            ticket: "13".to_owned(),
            waiting: true
        },
        "no slot is free",
    );
    assert_eq!(second.launches(), 0, "nothing runs for a ticket in line");
    assert_eq!(
        rig.columns(),
        [
            ("12".into(), Column::Building),
            ("13".into(), Column::Building)
        ],
        "the ticket in line is on the board with the one at work",
    );

    assert_eq!(
        rig.stop_and_tell(12, "wrap it up"),
        NoteOutcome::Sent,
        "the first worker's turn is cut short and its next one is the maintainer's text",
    );
    rig.until_turns(12, 2);
    let started = rig.start(13);

    assert_eq!(
        started,
        WorkerStarted {
            ticket: "13".to_owned(),
            waiting: false
        },
        "the turn that ended freed the slot",
    );
    rig.until_turns(13, 1);
    assert_eq!(
        rig.prompts(&second).len(),
        1,
        "the second worker got its prompt"
    );
    assert!(
        rig.prompts(&second)[0].contains("It does two."),
        "and it is the second ticket's text: {:?}",
        rig.prompts(&second),
    );
}

#[test]
fn a_note_to_one_worker_reaches_only_that_worker() {
    let rig = Rig::new(2);
    let first = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    let second = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    rig.ticket(12, "Add the thing", "It does one.", &first);
    rig.ticket(13, "Fix the other", "It does two.", &second);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);

    let noted = rig.note(12, "use the new name");

    assert_eq!(
        noted,
        NoteOutcome::Sent,
        "a worker between turns takes it as its next turn"
    );
    rig.until_turns(12, 2);
    let first_prompts = rig.prompts(&first);
    assert_eq!(first_prompts.len(), 2);
    assert_eq!(
        first_prompts[1], "use the new name",
        "the first worker's next prompt is the note",
    );
    let second_prompts = rig.prompts(&second);
    assert_eq!(
        second_prompts.len(),
        1,
        "the second worker was not prompted again"
    );
    assert!(
        !second_prompts[0].contains("use the new name"),
        "and what it was told is its ticket alone: {second_prompts:?}",
    );
    assert_eq!(second.launches(), 1);
}

#[test]
fn a_ruling_answered_for_one_ticket_leaves_the_other_alone() {
    let rig = Rig::new(2);
    let first = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    let second = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    rig.ticket(12, "Add the thing", "It does one.", &first);
    rig.ticket(13, "Fix the other", "It does two.", &second);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    rig.rule(12, "the pull request is still red");
    rig.rule(13, "the pull request is still red");

    let answered = rig.note(12, "the failure is in the fixture, not the code");

    assert_eq!(
        answered,
        NoteOutcome::Sent,
        "the Note answers the ruling by sending the worker back",
    );
    rig.until_turns(12, 2);
    let first_prompts = rig.prompts(&first);
    assert_eq!(first_prompts.len(), 2);
    assert!(
        first_prompts[1].ends_with("the failure is in the fixture, not the code"),
        "the first worker's next prompt carries the answer",
    );
    assert_eq!(
        rig.prompts(&second).len(),
        1,
        "the second worker was not prompted again",
    );
    assert_eq!(second.launches(), 1, "and no session started for it");
    let waiting = rig.record(13);
    assert_eq!(
        (waiting.status, waiting.reason.as_deref()),
        (
            Status::WaitingOnMaintainer,
            Some("the pull request is still red")
        ),
        "the other ticket still waits on its own ruling",
    );
    assert_eq!(
        rig.record(12).reason,
        None,
        "the answered ticket's ruling is over",
    );
}

#[test]
fn a_push_to_the_base_catches_up_both_review_tickets() {
    let rig = Rig::new(2);
    rig.ticket(12, "Add the thing", "It does one.", &FakeAgent::new());
    rig.ticket(13, "Fix the other", "It does two.", &FakeAgent::new());
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    let first = rig.push_work(12, "one.txt");
    let second = rig.push_work(13, "two.txt");
    // Someone else's change lands on the base
    rig.main_moves("main.txt");

    // The board's next read runs the gate, off its own thread
    rig.columns();
    rig.until(|| rig.origin("kelpie/12") != first && rig.origin("kelpie/13") != second);

    for ticket in ["12", "13"] {
        assert_eq!(
            rig.origin_parents(&format!("kelpie/{ticket}")),
            [rig.origin("main")],
            "ticket {ticket}'s branch was caught up on the new base",
        );
    }
    assert_eq!(
        rig.columns(),
        [("12".into(), Column::Review), ("13".into(), Column::Review)],
        "both tickets stay in Review",
    );
}

#[test]
fn a_red_run_on_one_ticket_sends_only_that_worker_a_fix_turn() {
    let rig = Rig::new(2);
    // The first ticket's fix turn keeps working, so its ticket is in Building to be seen
    let one = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("fixing").until_cancelled());
    let two = FakeAgent::new();
    rig.ticket(12, "Add the thing", "It does one.", &one);
    rig.ticket(13, "Fix the other", "It does two.", &two);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    let first = rig.push_work(12, "one.txt");
    let second = rig.push_work(13, "two.txt");
    // CI fails one ticket in a file it changed, and passes the other
    rig.fails(12, &first, "test", "assertion failed at one.txt:4");
    rig.github.script_checks("kelpie/13", HeadStatus::Passed);
    assert_eq!(
        second,
        rig.origin("kelpie/13"),
        "the other ticket is pushed"
    );

    // The first read has each head wait for its checks to register, and the next reads them.
    // A pass that could not read git waits for the next read, so the read is asked again.
    rig.columns();
    rig.until_passed();
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.prompts(&one).len() == 2
    });
    let prompt = &rig.prompts(&one)[1];
    assert!(prompt.contains("test"), "{prompt}");
    assert!(prompt.contains("assertion failed at one.txt:4"), "{prompt}");
    assert_eq!(
        rig.prompts(&two).len(),
        1,
        "the passing ticket's worker got nothing after its first turn"
    );
    assert_eq!(
        rig.columns(),
        [
            ("12".into(), Column::Building),
            ("13".into(), Column::Review)
        ],
        "the fix turn moves its ticket back to Building and the other stays in Review",
    );
}

#[test]
fn a_merge_of_one_ticket_sends_the_other_to_catch_up_with_the_base() {
    let rig = Rig::new(2);
    rig.ticket(12, "Add the thing", "It does one.", &FakeAgent::new());
    rig.ticket(13, "Fix the other", "It does two.", &FakeAgent::new());
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    let first = rig.push_work(12, "one.txt");
    let second = rig.push_work(13, "two.txt");
    rig.github.script_checks("kelpie/12", HeadStatus::Passed);
    rig.github.script_checks("kelpie/13", HeadStatus::Passed);
    // Each read has the checks wait to register, and a later one finds both ready
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.gate().ready().len() == 2
    });

    // The maintainer says yes to the first: its draft is marked ready, its checks waited
    // out, and the merge pinned to the head the yes was given on
    rig.gate().yes("12").expect("the first ticket is ready");
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.merged("kelpie/12")
    });
    assert!(!rig.merged("kelpie/13"), "only the first was merged");
    assert_eq!(
        rig.origin("kelpie/12"),
        first,
        "and it was merged as it was"
    );

    // The stand-in forge records a merge without moving `main`, so the merge lands on
    // `main` the way GitHub would put it there
    rig.main_moves("one.txt-landed");
    rig.gate().yes("13").expect("the second is still ready");
    rig.until(|| {
        rig.columns();
        rig.until_passed();
        rig.origin("kelpie/13") != second
    });

    assert_eq!(
        rig.origin_parents("kelpie/13"),
        [rig.origin("main")],
        "the second ticket's branch was caught up on the base the first one landed on",
    );
    assert_eq!(
        rig.gate().kept("13").and_then(|kept| kept.yes),
        None,
        "its yes was withdrawn, and it is asked for again on the new head",
    );
    assert!(!rig.merged("kelpie/13"), "nothing merged the stale head");
}

#[test]
fn a_merged_ticket_is_done_and_its_sibling_keeps_its_worktree() {
    let rig = Rig::new(2);
    rig.ticket(12, "Add the thing", "It does one.", &FakeAgent::new());
    rig.ticket(13, "Fix the other", "It does two.", &FakeAgent::new());
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    rig.push_work(12, "one.txt");
    rig.push_work(13, "two.txt");
    rig.github.script_checks("kelpie/12", HeadStatus::Passed);
    rig.github.script_checks("kelpie/13", HeadStatus::Passed);
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.gate().ready().len() == 2
    });
    assert!(
        rig.state.workers().session(&rig.project, "12").is_some(),
        "the worker keeps its session until its ticket is merged"
    );
    rig.gate().yes("12").expect("the first ticket is ready");
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.merged("kelpie/12")
    });

    // The read after the merge cleans up, and GitHub is left its time to close the issue
    rig.columns();
    rig.until_passed();
    rig.clock.advance(FORGE_CLOSE_WAIT);
    rig.columns();
    rig.until_passed();

    assert_eq!(
        rig.columns(),
        [("12".into(), Column::Done), ("13".into(), Column::Review)],
        "the merged ticket is Done and the other stays in Review",
    );
    let worktree = |number: u64| rig.paths().worktree(&number.to_string());
    assert!(
        !worktree(12).exists(),
        "the merged ticket's worktree is gone"
    );
    assert!(worktree(13).exists(), "the sibling keeps its worktree");
    let branches = git(
        &rig.dir.join("origin.git"),
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    );
    assert!(
        !branches.contains("kelpie/12"),
        "the merged branch is gone from the origin: {branches}"
    );
    assert!(branches.contains("kelpie/13"), "the sibling's stays");
    assert!(
        rig.state.workers().session(&rig.project, "12").is_none(),
        "the merged ticket's session stopped"
    );
    assert!(
        rig.state.workers().session(&rig.project, "13").is_some(),
        "the sibling's did not"
    );
    let records = rig.state.workers().records(&rig.project).unwrap();
    assert_eq!(
        lock(&records).get("12").map(|record| record.status),
        None,
        "its record is gone"
    );
    assert!(
        lock(&records).get("13").is_some(),
        "and the sibling's stays"
    );
}

#[test]
fn two_tickets_each_get_their_own_reads_and_fix_turns() {
    let rig = Rig::new(2);
    let (one, two) = (worker_that_fixes(), worker_that_fixes());
    rig.ticket(12, "Add the thing", "It does one.", &one);
    rig.ticket(13, "Fix the other", "It does two.", &two);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    rig.push_work(12, "one.txt");
    rig.push_work(13, "two.txt");
    rig.github.script_checks("kelpie/12", HeadStatus::Passed);
    rig.github.script_checks("kelpie/13", HeadStatus::Passed);
    // Each ticket's reader is scripted with a finding in that ticket's own file
    rig.reviewer(12).script_read(Ok(vec![finding(
        "one.txt",
        2,
        "the first ticket reads a file it never opens",
    )]));
    rig.reviewer(13).script_read(Ok(vec![finding(
        "two.txt",
        3,
        "the second ticket drops what it wrote",
    )]));

    // The first read has each head wait for its checks to register, and the next reads them
    rig.columns();
    rig.until_passed();
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.prompts(&one).len() == 2 && rig.prompts(&two).len() == 2
    });

    for (number, own, other) in [(12, "one.txt", "two.txt"), (13, "two.txt", "one.txt")] {
        let read = rig.read_prompts(number);
        assert_eq!(
            read.len(),
            2,
            "ticket {number}'s reader read its own head, then looked again",
        );
        for prompt in read {
            assert!(prompt.contains(own), "and showed it its own diff: {own}");
            assert!(
                !prompt.contains(other),
                "ticket {number}'s reader was shown no other ticket's file",
            );
        }
    }
    let sent = |agent: &FakeAgent| rig.prompts(agent)[1].clone();
    assert!(sent(&one).contains("one.txt:2"), "{}", sent(&one));
    assert!(
        !sent(&one).contains("two.txt"),
        "the first worker was sent only its own finding",
    );
    assert!(sent(&two).contains("two.txt:3"), "{}", sent(&two));
    assert!(
        !sent(&two).contains("one.txt"),
        "the second worker was sent only its own finding",
    );
    assert_eq!(
        rig.columns(),
        [
            ("12".into(), Column::Building),
            ("13".into(), Column::Building)
        ],
        "each ticket is in Building while its own fix turn works",
    );
}

#[test]
fn a_review_ruling_on_one_ticket_leaves_the_other_to_the_merge_yes() {
    let rig = Rig::new(2);
    // The first ticket's fix turn ends with nothing pushed; the second is clean
    let one = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("I left it as it is"));
    let two = FakeAgent::new();
    rig.ticket(12, "Add the thing", "It does one.", &one);
    rig.ticket(13, "Fix the other", "It does two.", &two);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    rig.push_work(12, "one.txt");
    rig.push_work(13, "two.txt");
    rig.github.script_checks("kelpie/12", HeadStatus::Passed);
    rig.github.script_checks("kelpie/13", HeadStatus::Passed);
    rig.reviewer(12).script_read(Ok(vec![finding(
        "one.txt",
        2,
        "it is wrong, and it is in scope",
    )]));

    // The finding goes to the first ticket's worker, and the second ticket's pass ends clean
    rig.columns();
    rig.until_passed();
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.prompts(&one).len() == 2 && rig.gate().ready().contains("13")
    });
    assert!(
        !rig.gate().ready().contains("12"),
        "the ruled ticket is not put up to merge"
    );

    // Its turn ended with nothing pushed and nothing said of the finding
    rig.until_turns(12, 2);
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.record(12).status == Status::WaitingOnMaintainer
    });
    let reason = rig.record(12).reason.expect("the ruling says why");
    assert!(reason.contains("pushed nothing"), "{reason}");
    assert_eq!(
        rig.gate().ready().into_iter().collect::<Vec<_>>(),
        ["13".to_owned()],
        "the other ticket is still the one put up to merge",
    );
}

#[test]
fn a_dispute_on_one_ticket_is_posted_only_on_its_pull_request() {
    let rig = Rig::new(2);
    // The first ticket's fix turn shows its finding wrong and pushes nothing; the second is clean
    let one = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("it is wrong, and the file shows it"));
    let two = FakeAgent::new();
    rig.ticket(12, "Add the thing", "It does one.", &one);
    rig.ticket(13, "Fix the other", "It does two.", &two);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    rig.push_work(12, "one.txt");
    rig.push_work(13, "two.txt");
    rig.github.script_checks("kelpie/12", HeadStatus::Passed);
    rig.github.script_checks("kelpie/13", HeadStatus::Passed);
    rig.reviewer(12).script_read(Ok(vec![finding(
        "one.txt",
        2,
        "it is wrong, and it is in scope",
    )]));

    // The finding goes to the first ticket's worker, and the second ticket's pass ends clean
    rig.columns();
    rig.until_passed();
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.prompts(&one).len() == 2 && rig.gate().ready().contains("13")
    });

    // Its turn showed the finding wrong with a reason and pushed nothing, and no reader follows
    rig.dispute(12, "the file does not do that");
    rig.until_turns(12, 2);
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        !rig.github.comments().is_empty()
    });

    let posted = rig.github.comments();
    let (on, body) = posted
        .first()
        .expect("the disputes went on the pull request as one comment");
    assert_eq!(
        *on,
        TicketId::from_issue(rig.pull_of(12).get()).unwrap(),
        "the comment is on the first ticket's pull request: {posted:?}"
    );
    assert!(
        body.contains("one.txt:2"),
        "the finding it disputes: {body}"
    );
    assert!(
        body.contains("the file does not do that"),
        "and the worker's reason: {body}"
    );
    assert_eq!(
        posted.len(),
        1,
        "one comment, for one ticket's disputes: {posted:?}"
    );
    assert!(
        !body.contains("two.txt"),
        "and it holds nothing of the other ticket: {body}"
    );
    assert_eq!(
        rig.gate().ready().into_iter().collect::<Vec<_>>(),
        ["12".to_owned(), "13".to_owned()],
        "the disputed ticket still reaches the merge yes, with the other",
    );
    assert!(
        rig.github
            .pulls()
            .iter()
            .all(|pull| pull.draft && pull.body.contains("Resolves #")),
        "no findings, no review and no ready mark went to GitHub either: {:?}",
        rig.github.pulls()
    );
}

#[test]
fn a_reviewers_failing_test_on_one_ticket_runs_only_in_that_worktree() {
    let rig = Rig::new(2);
    rig.test_command("cargo test --test {name}");
    let (one, two) = (worker_that_fixes(), worker_that_fixes());
    rig.ticket(12, "Add the thing", "It does one.", &one);
    rig.ticket(13, "Fix the other", "It does two.", &two);
    rig.start(12);
    rig.start(13);
    rig.until_turns(12, 1);
    rig.until_turns(13, 1);
    rig.until_drafts(2);
    rig.push_work(12, "one.txt");
    rig.push_work(13, "two.txt");
    rig.github.script_checks("kelpie/12", HeadStatus::Passed);
    rig.github.script_checks("kelpie/13", HeadStatus::Passed);
    // Only the first ticket's reviewer writes a test; the second's finding comes with none
    rig.attach(
        12,
        "tests/flag.rs",
        "#[test]\nfn flag() { assert!(false); }\n",
    );
    rig.reviewer(12).script_read(Ok(vec![
        finding("one.txt", 2, "the first ticket reads a file it never opens")
            .with_test("tests/flag.rs"),
    ]));
    rig.reviewer(13).script_read(Ok(vec![finding(
        "two.txt",
        3,
        "the second ticket drops what it wrote",
    )]));
    rig.launches.script([Exit {
        code: 101,
        output: b"the flag is not read".to_vec(),
    }]);

    // The first read has each head wait for its checks to register, and the next reads them
    rig.columns();
    rig.until_passed();
    rig.until(|| {
        rig.clock.advance(SETTLE);
        rig.columns();
        rig.until_passed();
        rig.prompts(&one).len() == 2 && rig.prompts(&two).len() == 2
    });

    let ran = rig.launches.ran();
    assert_eq!(
        ran.len(),
        1,
        "one test was written, so one was run: {ran:?}"
    );
    assert_eq!(
        ran[0].cwd,
        rig.paths().worktree("12"),
        "and it ran in the worktree of the ticket whose reviewer wrote it"
    );
    assert!(
        !ran[0].cwd.starts_with(rig.paths().worktree("13")),
        "never in the other ticket's"
    );
    assert!(
        rig.prompts(&one)[1].contains("tests/flag.rs"),
        "the first worker was sent its finding with its test: {}",
        rig.prompts(&one)[1]
    );
    assert!(
        !rig.prompts(&two)[1].contains("tests/flag.rs"),
        "the second worker was sent no test"
    );
}
