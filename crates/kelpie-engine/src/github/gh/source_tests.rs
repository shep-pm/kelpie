//! The GitHub source, against a scripted `gh` that answers from recorded output
//!
//! No test needs `gh`, a login or the network. The fixtures are what `gh` 2.96
//! printed for each call, trimmed to the fields the source reads.

use std::io;
use std::process::Output;

use super::GithubSource;
use super::conditional::PER_PAGE;
use super::hold::Hold;
use super::listing::time;
use super::tests::refused;
use super::tests::{Script, answered, gh_over};
use crate::github::{ForgeSlug, Timestamp};
use crate::ticket::{ForgeError, GithubTickets, Issue, Listed, TicketId, Tickets};

const ISSUE_VIEW: &str = include_str!("../../../tests/fixtures/gh-issue-view.json");

fn repo() -> ForgeSlug {
    ForgeSlug::try_from("shep-pm/kelpie".to_owned()).unwrap()
}

fn id(text: &str) -> TicketId {
    text.parse().unwrap()
}

// A source whose `gh` is `script`
fn source_over(script: &Script) -> GithubSource {
    GithubSource::new(Hold::new(gh_over(script)), repo())
}

#[test]
fn issue_reads_title_body_labels_and_open_from_gh_issue_view() {
    let script = Script::answering(answered(ISSUE_VIEW));

    let issue = source_over(&script).issue(&id("7")).unwrap();

    assert_eq!(
        script.asked(),
        [[
            "issue",
            "view",
            "7",
            "--repo",
            "shep-pm/kelpie",
            "--json",
            "title,body,labels,state,url"
        ]]
    );
    assert_eq!(issue.title, "Project runner skeleton with a status trigger");
    assert!(
        issue
            .body
            .starts_with("## Parent\n\n#5\n\n## What to build")
    );
    assert_eq!(issue.labels, ["ready-for-agent"]);
    assert!(!issue.open, "the recorded issue is closed");
    let open = ISSUE_VIEW.replace("\"CLOSED\"", "\"OPEN\"");
    let reopened = source_over(&Script::answering(answered(&open)))
        .issue(&id("7"))
        .unwrap();
    assert_eq!(
        reopened,
        Issue {
            open: true,
            ..issue
        }
    );
}

#[test]
fn issue_on_a_pull_request_number_is_no_issue() {
    // `gh issue view` answers a pull request's number with the pull request,
    // whose url names `pull`; a number nothing holds is a GraphQL failure
    let pull = ISSUE_VIEW.replace("/issues/7", "/pull/7");
    let merged = pull.replace("\"CLOSED\"", "\"MERGED\"");
    let nothing = "GraphQL: Could not resolve to an issue or pull request with the number of 99. (repository.issue)\n";
    for (answer, number) in [
        (answered(&pull), "7"),
        (answered(&merged), "7"),
        (refused(nothing), "99"),
    ] {
        let script = Script::answering(answer);

        let issue = source_over(&script).issue(&id(number));

        assert_eq!(issue, Err(ForgeError::NoIssue { ticket: id(number) }));
    }
}

#[test]
fn a_local_ticket_is_no_issue_and_starts_no_child() {
    let script = Script::default();

    let issue = source_over(&script).issue(&id("k3"));

    assert_eq!(issue, Err(ForgeError::NoIssue { ticket: id("k3") }));
    assert!(script.asked().is_empty());
}

#[test]
fn time_reads_an_iso_8601_moment_in_whole_seconds() {
    assert_eq!(
        time("2026-10-14T08:00:00Z"),
        Some(Timestamp::from_secs(1_791_964_800))
    );
    assert_eq!(
        time("2026-10-14T10:00:00+02:00"),
        Some(Timestamp::from_secs(1_791_964_800)),
        "an offset is the same moment"
    );
    assert_eq!(
        time("2026-10-14T08:00:00.9Z"),
        Some(Timestamp::from_secs(1_791_964_800)),
        "the fraction is dropped"
    );
    for not in ["", "yesterday", "2026-10-14", "1969-12-31T23:59:59Z"] {
        assert_eq!(time(not), None, "{not:?}");
    }
}

const ISSUE_LIST: &str = include_str!("../../../tests/fixtures/gh-issue-list.json");

#[test]
fn ready_issues_passes_the_label_it_is_given() {
    let script = Script::answering(answered("[]"));

    let listed = source_over(&script).ready_issues("needs-triage").unwrap();

    assert!(listed.is_empty());
    assert_eq!(
        script.asked(),
        [[
            "issue",
            "list",
            "--repo",
            "shep-pm/kelpie",
            "--label",
            "needs-triage",
            "--state",
            "open",
            "--limit",
            "1000",
            "--json",
            "number,title,labels,author,createdAt"
        ]]
    );
}

#[test]
fn ready_issues_reads_author_and_creation_time() {
    let script = Script::answering(answered(ISSUE_LIST));

    let listed = source_over(&script)
        .ready_issues("ready-for-agent")
        .unwrap();

    let read: Vec<_> = listed
        .iter()
        .map(|l| (l.id, l.author.as_str(), l.created.secs()))
        .collect();
    assert_eq!(
        read,
        [
            (id("163"), "octocat", 1_794_347_931),
            (id("162"), "octocat", 1_794_347_928),
            (id("161"), "hubot", 1_794_347_926),
        ]
    );
    assert_eq!(
        listed[0].title,
        "feat(app): the PM and workers run on goose when the settings name it"
    );
    assert_eq!(
        listed[0].labels,
        ["ready-for-agent", "priority: P2", "kind:feat"]
    );
}

const REST_OPEN: &str = include_str!("../../../tests/fixtures/gh-rest-issues-open.json");
const REST_CLOSED: &str = include_str!("../../../tests/fixtures/gh-rest-issues-closed.json");

// What `gh api --include` prints for a 200 that carries `etag`
fn ok(body: &str, etag: &str) -> io::Result<Output> {
    answered(&format!(
        "HTTP/2.0 200 OK\r\nContent-Type: application/json\r\nEtag: {etag}\r\n\r\n{body}"
    ))
}

// A page of `PER_PAGE` open issues numbered from `first`, the first of them a pull request
fn full_page(first: u64) -> String {
    let issue = |n: u64, pull: &str| {
        format!(
            r#"{{"number":{n},"title":"t{n}","body":"","created_at":"2026-11-10T19:00:00Z","closed_at":null,"user":{{"login":"octocat","type":"User"}},"labels":[]{pull}}}"#
        )
    };
    let pull = r#","pull_request":{"url":"https://api.github.com/repos/shep-pm/kelpie/pulls/1"}"#;
    let items: Vec<String> = (first..first + PER_PAGE as u64)
        .map(|n| issue(n, if n == first { pull } else { "" }))
        .collect();
    format!("[{}]", items.join(","))
}

fn numbers(listed: &[Listed]) -> Vec<u64> {
    listed.iter().filter_map(|l| l.id.github()).collect()
}

// The path of each `gh api` call the script was asked
fn paths(script: &Script) -> Vec<String> {
    script
        .asked()
        .iter()
        .map(|args| args.last().unwrap().clone())
        .collect()
}

#[test]
fn open_issues_skips_pull_requests_and_walks_every_page() {
    let script = Script::answering(ok(&full_page(1000), "W/\"a\""));
    script.then(ok(REST_OPEN, "W/\"b\""));

    let listed = source_over(&script).open_issues().unwrap();

    assert_eq!(
        paths(&script),
        [
            "repos/shep-pm/kelpie/issues?state=open&per_page=100&page=1",
            "repos/shep-pm/kelpie/issues?state=open&per_page=100&page=2",
        ]
    );
    let all = numbers(&listed);
    assert_eq!(
        all.len(),
        99 + 3,
        "100 on the first page less its pull request, 3 on the second"
    );
    assert_eq!(all[..2], [1001, 1002]);
    assert_eq!(
        all[99..],
        [163, 161, 150],
        "the pull request 160 is left out"
    );
    let last = &listed[101];
    assert_eq!(
        (
            last.title.as_str(),
            last.author.as_str(),
            last.created.secs()
        ),
        (
            "docs: an issue whose author deleted the account",
            "",
            1_794_211_200
        )
    );
    assert_eq!(listed[100].author, "dependabot[bot]");
}

#[test]
fn open_issues_answers_a_304_from_the_cache() {
    let script = Script::answering(ok(REST_OPEN, "W/\"v1\""));
    script.then(answered("HTTP/2.0 304 Not Modified\r\n\r\n"));
    let source = source_over(&script);

    let first = source.open_issues().unwrap();
    let again = source.open_issues().unwrap();

    assert_eq!(first, again);
    assert_eq!(numbers(&first), [163, 161, 150]);
    let asked = script.asked();
    assert!(!asked[0].iter().any(|a| a.starts_with("If-None-Match")));
    assert!(
        asked[1].contains(&"If-None-Match: W/\"v1\"".to_owned()),
        "{asked:?}"
    );
}

#[test]
fn closed_issues_keeps_only_those_closed_since_the_given_time() {
    let script = Script::answering(ok(REST_CLOSED, "W/\"c\""));
    let since = Timestamp::from_secs(1_793_836_800);

    let listed = source_over(&script).closed_issues(since).unwrap();

    assert_eq!(
        paths(&script),
        [
            "repos/shep-pm/kelpie/issues?state=closed&since=2026-11-05T00:00:00Z&sort=updated&per_page=100&page=1"
        ]
    );
    assert_eq!(
        numbers(&listed),
        [170, 169],
        "closed at the cut-off counts; a pull request, and an issue closed a second before it, do not"
    );
    assert_eq!(listed[0].created.secs(), 1_793_520_000);
}

#[test]
fn an_entry_that_does_not_parse_is_skipped_and_the_rest_are_kept() {
    let gh = r#"[
        {"number":1,"title":"a","labels":[],"author":{"login":"x"},"createdAt":"2026-11-10T19:00:00Z"},
        {"number":2,"title":"b","labels":[],"author":null,"createdAt":"last Tuesday"},
        {"number":0,"title":"c","labels":[],"author":null,"createdAt":"2026-11-10T19:00:00Z"},
        {"number":4,"labels":[],"author":null,"createdAt":"2026-11-10T19:00:00Z"},
        {"number":5,"title":"e","labels":[],"author":null,"createdAt":"2026-11-10T19:00:00Z"}
    ]"#;
    let rest = r#"[
        {"number":6,"title":"f","created_at":"2026-11-10T19:00:00Z","closed_at":null,"user":null,"labels":[]},
        {"number":7,"title":"g","created_at":"yesterday","closed_at":null,"user":null,"labels":[]},
        {"number":8,"created_at":"2026-11-10T19:00:00Z","closed_at":null,"user":null,"labels":[]},
        {"number":9,"title":"i","created_at":"2026-11-10T19:00:00Z","closed_at":null,"user":null,"labels":[]}
    ]"#;
    let script = Script::answering(answered(gh));
    script.then(ok(rest, "W/\"r\""));
    let source = source_over(&script);

    let ready = source.ready_issues("ready-for-agent").unwrap();
    let open = source.open_issues().unwrap();

    assert_eq!(numbers(&ready), [1, 5]);
    assert_eq!(numbers(&open), [6, 9]);
}

#[test]
fn a_list_that_is_not_a_list_is_unreadable() {
    let script = Script::answering(answered(r#"{"message":"nope"}"#));

    let ready = source_over(&script).ready_issues("ready-for-agent");

    assert!(matches!(ready, Err(ForgeError::Unreadable(_))), "{ready:?}");
}

#[test]
fn a_held_limit_stops_every_read() {
    let used_up = "gh: API rate limit exceeded for user ID 1. (HTTP 403)";
    let limits = r#"{"resources":{
        "core":{"limit":5000,"used":5000,"remaining":0,"reset":4000000000},
        "graphql":{"limit":5000,"used":0,"remaining":5000,"reset":4000000000},
        "search":{"limit":30,"used":0,"remaining":30,"reset":4000000000}}}"#;
    let script = Script::answering(refused(used_up));
    script.then(answered(limits));
    let source = source_over(&script);
    assert!(matches!(
        source.ready_issues("ready-for-agent"),
        Err(ForgeError::Limited { .. })
    ));
    let asked = script.asked().len();
    let held = ForgeError::Held(Timestamp::from_secs(4_000_000_000));

    assert_eq!(source.issue(&id("7")).unwrap_err(), held);
    assert_eq!(source.ready_issues("ready-for-agent").unwrap_err(), held);
    assert_eq!(source.open_issues().unwrap_err(), held);
    let closed = source.closed_issues(Timestamp::from_secs(0));
    assert_eq!(closed.unwrap_err(), held);
    assert_eq!(script.asked().len(), asked, "a held read starts no child");
}
