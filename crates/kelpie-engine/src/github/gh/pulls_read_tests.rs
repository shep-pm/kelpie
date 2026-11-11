//! The open pull requests, against a scripted `gh` that answers from a recorded page
//!
//! No test needs `gh`, a login or the network. The fixture is the shape
//! GitHub's GraphQL answers a query of the fields read, written from its docs.

use std::num::NonZeroU64;

use super::GithubSource;
use super::hold::Hold;
use super::tests::{Script, answered, gh_over, refused};
use crate::github::{ForgeSlug, Timestamp};
use crate::ticket::{ForgeError, GithubTickets, OpenPull, TicketId};

const OPEN_PULLS: &str = include_str!("../../../tests/fixtures/gh-graphql-open-pull-requests.json");

fn source_over(script: &Script) -> GithubSource {
    let repo = ForgeSlug::try_from("shep-pm/kelpie".to_owned()).unwrap();
    GithubSource::new(Hold::new(gh_over(script)), repo)
}

fn number(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

fn id(n: u64) -> TicketId {
    TicketId::from_issue(n).unwrap()
}

#[test]
fn a_pull_request_reads_its_number_branch_and_draft_flag() {
    let script = Script::answering(answered(OPEN_PULLS));

    let pulls = source_over(&script).open_pulls().unwrap();

    let asked = script.asked();
    assert_eq!(asked.len(), 1);
    let call = &asked[0];
    assert_eq!(call[..3], ["api", "graphql", "-f"]);
    assert!(call[3].starts_with("query=query("), "{}", call[3]);
    assert_eq!(call[4..], ["-f", "owner=shep-pm", "-f", "name=kelpie"]);
    let numbers_and_branches: Vec<_> = pulls
        .iter()
        .map(|p| (p.number, p.branch.as_str()))
        .collect();
    assert_eq!(
        numbers_and_branches,
        [
            (number(81), "worktree-kelpie-79"),
            (number(66), "kelpie/63")
        ]
    );
    assert_eq!(
        pulls.iter().map(|p| p.draft).collect::<Vec<_>>(),
        [true, false]
    );
}

#[test]
fn closing_references_become_ticket_ids() {
    let script = Script::answering(answered(OPEN_PULLS));

    let pulls = source_over(&script).open_pulls().unwrap();

    assert_eq!(pulls[0].closes, []);
    assert_eq!(pulls[1].closes, [id(63), id(64)]);
}

#[test]
fn a_ticket_of_another_repo_is_not_one_a_pull_request_closes_here() {
    let elsewhere = OPEN_PULLS.replacen("\"kelpie\"", "\"other\"", 1);
    let script = Script::answering(answered(&elsewhere));

    let pulls = source_over(&script).open_pulls().unwrap();

    assert_eq!(pulls[1].closes, [id(64)]);
}

#[test]
fn the_walker_follows_the_end_cursor_to_the_last_page() {
    let first = r#"{"data":{"repository":{"pullRequests":{
        "pageInfo":{"hasNextPage":true,"endCursor":"CURSOR-1"},
        "nodes":[{"number":9,"headRefName":"kelpie/8","isDraft":false,
        "closingIssuesReferences":{"nodes":[]}}]}}}}"#;
    let script = Script::answering(answered(first));
    script.then(answered(OPEN_PULLS));

    let pulls = source_over(&script).open_pulls().unwrap();

    let asked = script.asked();
    assert_eq!(asked.len(), 2);
    assert!(!asked[0].iter().any(|arg| arg.starts_with("after=")));
    assert_eq!(asked[1][asked[1].len() - 2..], ["-f", "after=CURSOR-1"]);
    let numbers: Vec<_> = pulls.iter().map(|p| p.number.get()).collect();
    assert_eq!(numbers, [9, 81, 66]);
    assert_eq!(
        pulls[0],
        OpenPull {
            number: number(9),
            branch: "kelpie/8".to_owned(),
            draft: false,
            closes: Vec::new(),
        }
    );
}

#[test]
fn a_held_limit_stops_the_read() {
    let used_up = "gh: API rate limit exceeded for user ID 1. (HTTP 403)";
    let limits = r#"{"resources":{
        "core":{"limit":5000,"used":0,"remaining":5000,"reset":4000000000},
        "graphql":{"limit":5000,"used":5000,"remaining":0,"reset":4000000000},
        "search":{"limit":30,"used":0,"remaining":30,"reset":4000000000}}}"#;
    let script = Script::answering(refused(used_up));
    script.then(answered(limits));
    let source = source_over(&script);
    assert!(matches!(
        source.open_pulls(),
        Err(ForgeError::Limited { .. })
    ));
    let asked = script.asked().len();

    let held = source.open_pulls();

    assert_eq!(
        held.unwrap_err(),
        ForgeError::Held(Timestamp::from_secs(4_000_000_000))
    );
    assert_eq!(script.asked().len(), asked, "a held read starts no child");
}

#[test]
fn a_graphql_error_in_a_200_answer_is_a_failure() {
    let errors = r#"{"data":{"repository":null},"errors":[
        {"type":"NOT_FOUND","message":"Could not resolve to a Repository with the name 'shep-pm/kelpie'."}]}"#;
    let script = Script::answering(answered(errors));

    let pulls = source_over(&script).open_pulls();

    let Err(ForgeError::Failed(said)) = pulls else {
        panic!("{pulls:?}");
    };
    assert!(said.contains("Could not resolve to a Repository"), "{said}");
}

#[test]
fn an_answer_that_is_no_page_is_unreadable() {
    let script = Script::answering(answered("<html>nope</html>"));

    let pulls = source_over(&script).open_pulls();

    assert!(matches!(pulls, Err(ForgeError::Unreadable(_))), "{pulls:?}");
}
