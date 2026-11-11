//! A comment and a close, through a scripted [`Runner`], so no test needs `gh`,
//! a login or the network
//!
//! [`Runner`]: super::Runner

use super::GithubSource;
use super::hold::Hold;
use super::tests::{Script, answered, gh_over, refused};
use crate::github::ForgeSlug;
use crate::ticket::{ForgeError, TicketId, Tickets};

fn source_over(script: &Script) -> GithubSource {
    let repo = ForgeSlug::try_from("shep-pm/koji".to_owned()).unwrap();
    GithubSource::new(Hold::new(gh_over(script)), repo)
}

fn id(text: &str) -> TicketId {
    text.parse().unwrap()
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

// `-f` sends the text as it is, where a flag could read a text that begins with
// a dash as a flag
#[test]
fn a_comment_is_sent_with_its_text_as_one_field() {
    let script = Script::answering(answered("{}\n"));

    source_over(&script)
        .comment(&id("7"), "- merged\n\nwithout a closing line")
        .unwrap();

    assert_eq!(
        script.asked(),
        [strings(&[
            "api",
            "--method",
            "POST",
            "repos/shep-pm/koji/issues/7/comments",
            "-f",
            "body=- merged\n\nwithout a closing line",
        ])]
    );
}

#[test]
fn a_close_asks_gh_to_close_that_issue() {
    let script = Script::answering(answered("{}\n"));

    source_over(&script).close(&id("12")).unwrap();

    assert_eq!(
        script.asked(),
        [strings(&[
            "api",
            "--method",
            "PATCH",
            "repos/shep-pm/koji/issues/12",
            "-f",
            "state=closed",
        ])]
    );
}

#[test]
fn a_close_gh_refuses_is_an_error_with_its_words() {
    let script = Script::answering(refused(
        "gh: Must have admin rights to Repository. (HTTP 403)",
    ));

    let closed = source_over(&script).close(&id("12"));

    assert_eq!(
        closed,
        Err(ForgeError::Failed(
            "gh: Must have admin rights to Repository. (HTTP 403)".to_owned()
        ))
    );
}

#[test]
fn a_local_ticket_has_no_issue_to_comment_on_or_close() {
    let script = Script::default();
    let source = source_over(&script);

    let commented = source.comment(&id("k1"), "hi");
    let closed = source.close(&id("k1"));

    let want = Err(ForgeError::NoIssue { ticket: id("k1") });
    assert_eq!(commented, want);
    assert_eq!(closed, want);
    assert_eq!(script.asked(), Vec::<Vec<String>>::new());
}
