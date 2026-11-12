//! What the gate leaves clean after a merge: the issue closed, the worktree and branch gone
//!
//! Each test merges a pull request, by the maintainer's yes or by hand, on a bare origin
//! with the forge stood in, and reads what the outside world shows of it: the comments and
//! closes asked of the tickets, the worktree on disk, the branch on the origin and the
//! worker's record.

use kelpie_engine::ci::HeadStatus;
use kelpie_engine::gate::FORGE_CLOSE_WAIT;
use kelpie_engine::ticket::fake::Asked;
use kelpie_engine::ticket::{ForgeError, Issue, OpenPull, PullState, TicketId};
use kelpie_engine::worker::Status;

use super::rig::{Rig, git};
use super::{SETTLE, TICKET, rig};

fn issue() -> TicketId {
    TicketId::from_issue(7).unwrap()
}

/// The worker has ended with its work pushed, and the pull request has been read once
///
/// The pull request is returned, as the listing showed it.
fn reviewed(rig: &Rig, closes: bool) -> OpenPull {
    let head = rig.worker_on(TICKET, "work.txt", "mine\n");
    let mut pull = rig.pull(TICKET, &head, HeadStatus::Passed);
    if !closes {
        pull.closes.clear();
    }
    rig.read(std::slice::from_ref(&pull));
    pull
}

/// The maintainer merged the pull request on GitHub, so it is gone from the next listing
fn merged_by_hand(rig: &Rig, pull: &OpenPull) {
    rig.pulls.script_state(pull.number, Ok(PullState::Merged));
}

/// The comments and closes asked of the tickets, in order
fn writes(rig: &Rig) -> Vec<Asked> {
    rig.tickets
        .asked()
        .into_iter()
        .filter(|asked| matches!(asked, Asked::Comment(..) | Asked::Close(_)))
        .collect()
}

fn comments(rig: &Rig) -> usize {
    (writes(rig).iter())
        .filter(|asked| matches!(asked, Asked::Comment(..)))
        .count()
}

fn closes(rig: &Rig) -> usize {
    (writes(rig).iter())
        .filter(|asked| matches!(asked, Asked::Close(_)))
        .count()
}

/// The branches the origin holds
fn branches(rig: &Rig) -> String {
    git(
        &rig.root.join("origin.git"),
        &["for-each-ref", "--format=%(refname:short)", "refs/heads"],
    )
}

fn has_record(rig: &Rig) -> bool {
    rig.records().iter().any(|record| record.ticket == TICKET)
}

#[test]
fn an_issue_the_pull_names_is_left_to_github_inside_the_wait() {
    let rig = rig();
    let pull = reviewed(&rig, true);
    merged_by_hand(&rig, &pull);

    rig.read(&[]);
    rig.clock.advance(FORGE_CLOSE_WAIT - 1);
    rig.read(&[]);

    assert_eq!(writes(&rig), [], "GitHub is given its time to close it");
    assert!(
        has_record(&rig),
        "the ticket is kept, so the next read looks again"
    );
}

#[test]
fn an_issue_still_open_after_the_wait_is_closed_with_one_comment() {
    let rig = rig();
    let pull = reviewed(&rig, true);
    merged_by_hand(&rig, &pull);
    rig.read(&[]);

    rig.clock.advance(FORGE_CLOSE_WAIT);
    rig.read(&[]);
    rig.read(&[]);

    assert_eq!(
        writes(&rig).iter().map(kind).collect::<Vec<_>>(),
        ["comment", "close"],
        "one comment, then the close"
    );
    assert!(
        matches!(&writes(&rig)[0], Asked::Comment(id, body) if *id == issue() && body.contains("#107")),
        "the comment names the pull request that merged: {:?}",
        writes(&rig)
    );
    assert!(!has_record(&rig), "the ticket is done with the gate");
}

fn kind(asked: &Asked) -> &'static str {
    match asked {
        Asked::Comment(..) => "comment",
        Asked::Close(_) => "close",
        _ => "other",
    }
}

#[test]
fn a_pull_merged_with_no_closing_reference_gets_its_issue_closed() {
    let rig = rig();
    let pull = reviewed(&rig, false);
    merged_by_hand(&rig, &pull);

    rig.read(&[]);

    assert_eq!(
        writes(&rig).iter().map(kind).collect::<Vec<_>>(),
        ["comment", "close"],
        "nothing names the issue, so GitHub is not waited on"
    );
}

#[test]
fn an_issue_github_already_closed_gets_no_comment_and_no_second_close() {
    let rig = rig();
    let pull = reviewed(&rig, true);
    merged_by_hand(&rig, &pull);
    rig.tickets.script_issue(
        issue(),
        Ok(Issue {
            title: "Ticket 7".to_owned(),
            body: "Build it.".to_owned(),
            labels: Vec::new(),
            open: false,
        }),
    );

    rig.read(&[]);

    assert_eq!(writes(&rig), [], "GitHub closed it itself");
    assert!(!has_record(&rig), "and the cleanup went on");
}

#[test]
fn a_close_github_refuses_is_tried_three_times_then_given_up() {
    let rig = rig();
    let pull = reviewed(&rig, false);
    merged_by_hand(&rig, &pull);
    rig.tickets
        .script_write(Err(ForgeError::Failed("HTTP 403".to_owned())));

    for _ in 0..5 {
        rig.read(&[]);
    }

    assert_eq!(closes(&rig), 3, "three tries");
    assert_eq!(comments(&rig), 1, "and the comment once");
    assert!(
        !has_record(&rig),
        "the ticket is cleaned up, with its issue left open for the maintainer"
    );
    assert!(!rig.worktree(TICKET).exists());
}

#[test]
fn a_merge_by_hand_is_cleaned_up_too() {
    let rig = rig();
    let pull = reviewed(&rig, true);
    merged_by_hand(&rig, &pull);

    rig.read(&[]);
    rig.clock.advance(FORGE_CLOSE_WAIT);
    rig.read(&[]);

    assert_eq!(closes(&rig), 1, "the issue was closed");
    assert!(!rig.worktree(TICKET).exists(), "the worktree is gone");
    assert!(!branches(&rig).contains("kelpie/7"), "so is the branch");
    assert!(!has_record(&rig), "so is the record");
    assert!(rig.gate.kept(TICKET).is_none(), "and the gate's facts");
}

#[test]
fn the_session_stops_and_the_worktree_and_remote_branch_go() {
    let rig = rig();
    let pull = reviewed(&rig, true);
    let build = rig.paths().build_dir(TICKET);
    std::fs::create_dir_all(&build).unwrap();
    std::fs::write(build.join("artifact"), "built").unwrap();
    assert!(branches(&rig).contains("kelpie/7"));
    merged_by_hand(&rig, &pull);

    rig.read(&[]);

    assert!(!rig.worktree(TICKET).exists(), "the worktree is gone");
    assert!(!build.exists(), "the build folder is gone");
    assert!(
        !branches(&rig).contains("kelpie/7"),
        "the remote branch is gone"
    );
    assert_eq!(
        rig.record(TICKET).status,
        Status::Ended,
        "the record stays until the issue is settled"
    );
    assert_eq!(
        git(&rig.checkout(), &["branch", "--list", "kelpie/7"]),
        "",
        "and no local branch is left"
    );
}

#[test]
fn a_branch_github_deleted_first_still_counts_as_cleaned_up() {
    let rig = rig();
    let pull = reviewed(&rig, false);
    git(
        &rig.root.join("origin.git"),
        &["branch", "--quiet", "-D", "kelpie/7"],
    );
    merged_by_hand(&rig, &pull);

    rig.read(&[]);

    assert!(!rig.worktree(TICKET).exists(), "the worktree is gone");
    assert!(!has_record(&rig), "and the cleanup finished");
}

#[test]
fn a_ticket_that_ended_unmerged_keeps_its_worktree_and_issue() {
    let rig = rig();
    let pull = reviewed(&rig, true);
    rig.pulls.script_state(pull.number, Ok(PullState::Closed));

    rig.read(&[]);
    rig.clock.advance(FORGE_CLOSE_WAIT + SETTLE);
    rig.read(&[]);

    assert!(rig.worktree(TICKET).exists(), "the worktree is kept");
    assert!(branches(&rig).contains("kelpie/7"), "so is the branch");
    assert_eq!(writes(&rig), [], "the issue is not closed");
    assert!(has_record(&rig), "the worker is handed back");
}

#[test]
fn a_step_that_failed_is_tried_on_the_next_read() {
    let rig = rig();
    let pull = reviewed(&rig, false);
    merged_by_hand(&rig, &pull);
    rig.tickets
        .script_write(Err(ForgeError::Failed("HTTP 502".to_owned())));
    rig.read(&[]);
    assert!(has_record(&rig), "one refusal is not given up on");
    assert_eq!(
        rig.gate
            .kept(TICKET)
            .and_then(|kept| kept.merged)
            .map(|merged| merged.tries),
        Some(1),
        "gate.json keeps the try"
    );

    rig.tickets.script_write(Ok(()));
    rig.read(&[]);

    assert_eq!(closes(&rig), 2, "the second read tried again");
    assert!(!has_record(&rig));
}
