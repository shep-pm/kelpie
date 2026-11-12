//! The head the merge yes is given on that no reviewer read, and the disputes no reader was
//! shown
//!
//! Lifted from SK's `runner/review/unread.rs`, `short.rs` and the posting cases of
//! `disputes_tests.rs` (`docs/lifted.md`): a head pushed after the pass's last read, or a pass
//! that ended short, reaches the merge yes with a warning naming it, and the yes is the
//! maintainer's all the same. The disputes go on the pull request as one comment, and nothing
//! else a pass finds goes to GitHub.

use std::fs;

use kelpie_engine::ci::HeadStatus;
use kelpie_engine::gate::short;
use kelpie_engine::review::{self, ReviewerError, Severity};
use kelpie_engine::ticket::ForgeError;
use kelpie_engine::ticket::TicketId;
use kelpie_engine::ticket::fake::{Asked, PullCall};
use kelpie_engine::worker::Status;

use super::{
    FIRST, SETTLE, TICKET, ended, finding, green, kept, line, prompt, read_green, rig,
    two_reviewers, until_some,
};
use crate::gate::tests::rig::Rig;

/// The comment the ticket's pull request earned, `None` when it has none
///
/// A dispute is posted with the pull request's number, which GitHub reads as an issue's own.
fn posted(rig: &Rig, head: &str) -> Option<String> {
    let number = rig.pull(TICKET, head, HeadStatus::Passed).number;
    let id = TicketId::from_issue(number.get()).unwrap();
    rig.tickets
        .asked()
        .into_iter()
        .find(|ask| matches!(ask, Asked::Comment(asked, _) if *asked == id))
        .map(|ask| match ask {
            Asked::Comment(_, body) => body,
            _ => unreachable!("asked is the comment above"),
        })
}

/// Every comment the ticket source was asked for, oldest first
fn comments(rig: &Rig) -> Vec<(TicketId, String)> {
    rig.tickets
        .asked()
        .into_iter()
        .filter_map(|ask| match ask {
            Asked::Comment(id, body) => Some((id, body)),
            _ => None,
        })
        .collect()
}

/// The worker's reason for the round's finding, naming `path` as the file it read
fn dispute(rig: &Rig, path: &std::path::Path) {
    dispute_as(
        rig,
        &format!("{} shows the finding is wrong", path.display()),
    );
}

/// The worker's reason for the round's finding, as `text`
///
/// A fix turn writes it beside the findings file, as `disputed-findings.md`.
fn dispute_as(rig: &Rig, text: &str) {
    let build = rig.paths().build_dir(TICKET);
    let findings = fs::read_to_string(review::findings_path(&build)).expect("the findings file");
    let line = (findings.lines())
        .find(|line| line.starts_with("HIGH|"))
        .expect("a finding's line");
    fs::write(review::disputed_path(&build), format!("{line}\n  {text}\n")).unwrap();
}

/// The finding the worker is sent, and its fix turn, on a head CI is green on
fn disputed(rig: &Rig, head: &str, path: &std::path::Path) {
    let sent = until_some(|| prompt(rig, 0));
    assert!(
        sent.contains("it is wrong, and it is in scope"),
        "the worker was sent the finding: {sent}"
    );
    dispute(rig, path);
    ended(rig);
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, head, HeadStatus::Passed)]);
}

/// Whether the gate put the ticket up to merge
fn ready(rig: &Rig) -> bool {
    rig.gate.ready().contains(TICKET)
}

#[test]
fn a_head_pushed_after_the_last_read_reaches_the_yes_with_a_warning() {
    let rig = rig();
    two_reviewers(&rig);
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(Vec::new()));
    reader.script_read(Ok(Vec::new()));
    let read = green(&rig, "work.txt");
    assert_eq!(
        line(&rig),
        Some("Ready to merge".to_owned()),
        "the head both reviewers read is ready with nothing to warn of"
    );

    // The maintainer sent the worker a Note, and it pushed again with no reviewer reading
    let pushed = rig.commit(TICKET, "later.txt", "later\n");
    read_green(&rig, &pushed);

    let text = line(&rig).expect("the card says where the ticket stands");
    assert!(
        text.starts_with("Ready to merge, unread: "),
        "the yes names the head no reviewer read: {text}"
    );
    assert!(text.contains(short(&pushed)), "the head it is on: {text}");
    assert!(
        text.contains(short(&read)),
        "and the head last read: {text}"
    );
    assert!(text.contains("no reviewer read it"), "{text}");
    assert_eq!(
        reader.prompts().len(),
        2,
        "no reviewer was sent to read the head it pushed"
    );
    assert!(ready(&rig), "and the yes is the maintainer's as before");
}

#[test]
fn a_pass_that_ended_short_names_who_was_missed() {
    let rig = rig();
    two_reviewers(&rig);
    let reader = rig.reader(TICKET);
    let down = || Err(ReviewerError::Failed("overloaded".to_owned()));
    reader.script_read(down());
    reader.script_read(down());
    reader.script_read(down());
    reader.script_read(Ok(Vec::new()));
    let head = green(&rig, "work.txt");

    let text = line(&rig).expect("the card says where the ticket stands");
    assert!(
        text.starts_with(&format!(
            "Ready to merge, unread: The review ended short at {}: 1 of 2 reviewers read it: ",
            short(&head)
        )),
        "{text}"
    );
    assert!(text.contains(FIRST), "which reviewer was missed: {text}");
    assert!(text.contains("passed over"), "{text}");
    assert_eq!(
        reader.prompts().len(),
        4,
        "the third failure passed the first reviewer over, and the second read"
    );
    assert!(ready(&rig), "and the yes is the maintainer's all the same");
}

#[test]
fn an_empty_reviewers_list_reaches_the_yes_unread_with_the_reason() {
    let rig = rig();
    rig.reviewers_listed(&[]);
    let head = green(&rig, "work.txt");

    assert_eq!(
        line(&rig),
        Some(format!(
            "Ready to merge, unread: No reviewer read {}.",
            short(&head)
        )),
        "a project with no reviewer to run goes to the yes with the head named unread"
    );
    assert!(
        rig.reader(TICKET).prompts().is_empty(),
        "and no reviewer read it"
    );
    assert!(ready(&rig), "with the yes the maintainer's as ever");
}

#[test]
fn a_fix_turns_own_push_after_the_last_read_carries_no_warning() {
    let rig = rig();
    two_reviewers(&rig);
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        5,
        "and so is this",
    )]));
    let read = green(&rig, "work.txt");

    // Each reader's finding is fixed by a push of the worker's own
    ended(&rig);
    let fixed = rig.commit(TICKET, "fixed.txt", "fixed\n");
    read_green(&rig, &fixed);
    ended(&rig);
    let last = rig.commit(TICKET, "last.txt", "last\n");
    read_green(&rig, &last);

    assert!(
        kept(&rig).heads.vouches_for(&last),
        "the head the pass's own fix turn pushed is one it vouches for"
    );
    assert!(
        !kept(&rig).heads.reviewed(&last),
        "and no reviewer ever read it: {} is the last read",
        short(&read)
    );
    assert_eq!(
        line(&rig),
        Some("Ready to merge".to_owned()),
        "so the yes warns of nothing"
    );
    assert_eq!(
        reader.prompts().len(),
        2,
        "and no reviewer was sent back to read the head it pushed"
    );
    assert!(ready(&rig), "and the yes is the maintainer's");
}

#[test]
fn with_no_reader_after_it_the_disputes_go_on_the_pull_request_as_one_comment() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    let head = green(&rig, "work.txt");
    // Its reason names a file of the worker's own build folder, which the comment names by its
    // name alone
    let build = rig.paths().build_dir(TICKET);
    disputed(&rig, &head, &build.join("maintainer-review.md"));

    let posted = posted(&rig, &head).expect("one comment for the disputes");
    assert!(posted.contains("work.txt:2"), "the finding: {posted}");
    assert!(
        posted.contains("it is wrong, and it is in scope"),
        "which is what the worker showed wrong: {posted}"
    );
    assert!(
        posted.contains("maintainer-review.md shows the finding is wrong"),
        "the worker's reason, its file by its name: {posted}"
    );
    assert!(
        !posted.contains(&build.display().to_string()),
        "and nothing of this machine's folders: {posted}"
    );
    assert_eq!(
        comments(&rig).len(),
        1,
        "one comment, for the disputes and nothing else"
    );
    assert_eq!(kept(&rig).disputes.len(), 1);
    assert!(
        kept(&rig).disputes[0].shown,
        "shown once posted, so a later pass posts none twice"
    );
    assert!(
        rig.pulls
            .calls()
            .iter()
            .all(|call| !matches!(call, PullCall::MarkReady { .. } | PullCall::Edit { .. })),
        "no findings, no review and no ready mark went to GitHub either: {:?}",
        rig.pulls.calls()
    );
    assert_eq!(
        reader.prompts().len(),
        1,
        "and no reader followed to be shown them"
    );
}

#[test]
fn a_reader_shown_the_disputes_leaves_none_to_post() {
    let rig = rig();
    two_reviewers(&rig);
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    reader.script_read(Ok(Vec::new()));
    let head = green(&rig, "work.txt");
    disputed(
        &rig,
        &head,
        &rig.paths().build_dir(TICKET).join("maintainer-review.md"),
    );

    assert_eq!(
        reader.prompts().len(),
        2,
        "the reader that follows was shown the dispute"
    );
    assert!(kept(&rig).disputes[0].shown, "and it counts as shown");
    assert!(
        comments(&rig).is_empty(),
        "so nothing is posted: {:?}",
        comments(&rig)
    );
}

#[test]
fn a_dispute_naming_another_local_path_rules_and_posts_nothing() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    let head = green(&rig, "work.txt");
    // Its reason names a file of the project's checkout, which is no file of the build folder
    disputed(&rig, &head, &rig.checkout().join("notes.md"));

    let record = rig.record(TICKET);
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    let reason = record.reason.expect("the ruling says why");
    assert!(reason.contains("cannot post them"), "{reason}");
    assert!(
        reason.contains("a path on this machine"),
        "which names what was refused: {reason}"
    );
    assert_eq!(line(&rig), Some(reason.clone()), "and the card says it too");
    assert!(
        comments(&rig).is_empty(),
        "nothing was posted: {:?}",
        comments(&rig)
    );
    assert!(
        !kept(&rig).disputes[0].shown,
        "the disputes stay unshown, for the maintainer to answer"
    );
    assert!(!ready(&rig), "and the ticket is not put up to merge");
    assert_eq!(reader.prompts().len(), 1, "and no reader followed");
}

#[test]
fn a_refused_post_is_tried_on_the_next_read() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    let head = green(&rig, "work.txt");
    // The forge will not take the comment just now
    rig.tickets
        .script_write(Err(ForgeError::Failed("HTTP 502: Bad Gateway".to_owned())));
    disputed(&rig, &head, &rig.paths().build_dir(TICKET).join("notes.md"));

    assert_eq!(
        comments(&rig).len(),
        1,
        "one comment was asked for and refused"
    );
    assert!(
        !kept(&rig).disputes[0].shown,
        "and the disputes are not marked shown by a post that did not land"
    );
    assert!(
        !ready(&rig),
        "and the head is not put up to merge while its disputes are unposted"
    );

    // The gate gives the yes again on the next read, which asks again
    rig.tickets.script_write(Ok(()));
    rig.clock.advance(SETTLE);
    rig.read(&[rig.pull(TICKET, &head, HeadStatus::Passed)]);

    assert_eq!(comments(&rig).len(), 2, "the next read asked again");
    assert!(
        kept(&rig).disputes[0].shown,
        "and they are shown once posted"
    );
    assert!(ready(&rig), "and the head goes ready once they are posted");
    assert_eq!(reader.prompts().len(), 1, "with no reviewer read again");
}

#[test]
fn a_dispute_whose_reason_is_written_again_is_posted_on_the_next_yes() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    let head = green(&rig, "work.txt");
    // Its reason names a file of the project's checkout, so the yes rules
    disputed(&rig, &head, &rig.checkout().join("notes.md"));
    assert!(
        comments(&rig).is_empty(),
        "nothing was posted: {:?}",
        comments(&rig)
    );
    assert!(!ready(&rig), "and the ticket is not put up to merge");

    // The maintainer sent the Note the ruling asks for, and the worker wrote its reason
    // again in the disputed findings file, naming no path
    dispute_as(&rig, "the file reads two values, not three");
    let pushed = rig.commit(TICKET, "later.txt", "later\n");
    read_green(&rig, &pushed);

    let posted = posted(&rig, &pushed).expect("the rewritten reason went on the pull request");
    assert!(posted.contains("work.txt:2"), "the finding: {posted}");
    assert!(
        posted.contains("the file reads two values, not three"),
        "what the worker wrote again: {posted}"
    );
    assert!(
        !posted.contains("notes.md"),
        "and not the reason that was refused: {posted}"
    );
    assert_eq!(comments(&rig).len(), 1, "one comment: {:?}", comments(&rig));
    assert!(ready(&rig), "so the ticket reaches the yes");
    assert_eq!(
        rig.record(TICKET).status,
        Status::Ended,
        "and the worker is back in play"
    );
    assert_eq!(reader.prompts().len(), 1, "with no reviewer read again");
}

#[test]
fn a_dispute_naming_another_projects_folder_rules_and_posts_nothing() {
    let rig = rig();
    let reader = rig.reader(TICKET);
    reader.script_read(Ok(vec![finding(
        Severity::High,
        "work.txt",
        2,
        "it is wrong, and it is in scope",
    )]));
    let head = green(&rig, "work.txt");
    // Its reason names a file of another project's, under kelpie's data folder, which is no
    // file of this project's checkout
    let other = rig
        .paths()
        .sessions_root()
        .join("other/worktrees/9/notes.md");
    disputed(&rig, &head, &other);

    assert!(
        comments(&rig).is_empty(),
        "nothing was posted: {:?}",
        comments(&rig)
    );
    let reason = rig.record(TICKET).reason.expect("the ruling says why");
    assert!(reason.contains("cannot post them"), "{reason}");
    assert!(
        reason.contains("a path on this machine"),
        "which names what was refused: {reason}"
    );
    assert!(!ready(&rig), "and the ticket is not put up to merge");
    assert_eq!(reader.prompts().len(), 1, "and no reader followed");
}
