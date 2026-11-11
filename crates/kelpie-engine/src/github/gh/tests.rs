//! The call to `gh` and what a failure of it says
//!
//! A scripted [`Runner`] stands in for the child, so no test needs `gh`, a
//! login or the network.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::process::{ExitStatus, Output};
use std::sync::{Arc, Mutex};

use super::account::RateBucket;
use super::failed::failed;
use super::{Gh, Runner, command};
use crate::github::{Cooldown, Timestamp};
use crate::ticket::ForgeError;

#[cfg(unix)]
fn status(code: i32) -> ExitStatus {
    std::os::unix::process::ExitStatusExt::from_raw(code << 8)
}

#[cfg(windows)]
fn status(code: i32) -> ExitStatus {
    std::os::windows::process::ExitStatusExt::from_raw(code.cast_unsigned())
}

pub(super) fn answered(stdout: &str) -> io::Result<Output> {
    Ok(Output {
        status: status(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    })
}

pub(super) fn refused(stderr: &str) -> io::Result<Output> {
    Ok(Output {
        status: status(1),
        stdout: Vec::new(),
        stderr: stderr.as_bytes().to_vec(),
    })
}

// The child, scripted: each call is answered with the next answer and recorded
#[derive(Debug, Clone, Default)]
pub(super) struct Script {
    answers: Arc<Mutex<VecDeque<io::Result<Output>>>>,
    asked: Arc<Mutex<Vec<Vec<String>>>>,
}

impl Script {
    pub(super) fn answering(answer: io::Result<Output>) -> Self {
        let script = Self::default();
        script.answers.lock().unwrap().push_back(answer);
        script
    }

    // The next answer after those already scripted
    pub(super) fn then(&self, answer: io::Result<Output>) -> &Self {
        self.answers.lock().unwrap().push_back(answer);
        self
    }

    pub(super) fn asked(&self) -> Vec<Vec<String>> {
        self.asked.lock().unwrap().clone()
    }
}

impl Runner for Script {
    fn run(&self, args: &[String]) -> io::Result<Output> {
        self.asked.lock().unwrap().push(args.to_vec());
        self.answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("a call the script has no answer for")
    }
}

pub(super) fn gh_over(script: &Script) -> Gh {
    Gh::with_runner(Box::new(script.clone()))
}

// The shape `GH_DEBUG=api` prints for a call a secondary limit refused: the
// exchange, the body, then gh's own error line
const SECONDARY: &str = "\
* Request at 2026-10-10 05:31:54.1 +0000 UTC
* Request to https://api.github.com/graphql
> POST /graphql HTTP/1.1
> Host: api.github.com
>
< HTTP/2.0 403 Forbidden
< Content-Type: application/json; charset=utf-8
< Retry-After: 75
< X-Ratelimit-Reset: 1790000600
<
{\"message\":\"You have exceeded a secondary rate limit.\"}
gh: You have exceeded a secondary rate limit. Please wait a few minutes before you try again. (HTTP 403)
";

// A 504 on a GraphQL read: the query and the page GitHub answered with, then gh's status line
const PAGE: &str = "\
GraphQL query:
query IssueByNumber($owner: String!, $repo: String!, $number: Int!) {
    repository(owner: $owner, name: $repo) { hasIssuesEnabled }
}
GraphQL variables: {\"number\":1368,\"owner\":\"shep-pm\",\"repo\":\"kelpie\"}


<!DOCTYPE html>
<html>
  <head>
    <title>Unicorn! &middot; GitHub</title>
  </head>
  <body>
    <div class=\"container\"><h1>Unicorn!</h1></div>
  </body>
</html>

HTTP 504: 504 Gateway Timeout (https://api.github.com/graphql)
";

#[test]
fn a_call_runs_gh_with_gh_debug_api_in_its_environment() {
    let command = command(
        Path::new("/usr/local/bin/gh"),
        &["api".into(), "user".into()],
    );

    assert_eq!(command.get_program(), OsStr::new("/usr/local/bin/gh"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [OsStr::new("api"), OsStr::new("user")]
    );
    let debug = command
        .get_envs()
        .find(|(name, _)| *name == OsStr::new("GH_DEBUG"));
    assert_eq!(
        debug,
        Some((OsStr::new("GH_DEBUG"), Some(OsStr::new("api"))))
    );
}

#[test]
fn a_call_hands_gh_its_arguments_and_answers_with_its_stdout() {
    let script = Script::answering(answered("octocat\n"));

    let stdout = gh_over(&script).call(&["api", "user"]);

    assert_eq!(stdout, Ok(b"octocat\n".to_vec()));
    assert_eq!(script.asked(), [["api", "user"]]);
}

#[test]
fn a_failure_keeps_the_gh_line_and_drops_the_traffic() {
    let stderr = "* Request to https://api.github.com/x\n< HTTP/2.0 404 Not Found\n<\n\
                  {\"message\":\"Not Found\"}\ngh: Not Found (HTTP 404)\n";

    assert_eq!(
        failed(stderr),
        ForgeError::Failed("gh: Not Found (HTTP 404)".into())
    );
}

#[test]
fn a_failure_with_no_traffic_is_kept_as_gh_said_it() {
    let plain = "gh: Not Found (HTTP 404)\n";

    assert_eq!(failed(plain), ForgeError::Failed(plain.trim_end().into()));
}

#[test]
fn a_failure_with_no_gh_line_keeps_its_error_and_none_of_the_bodies_around_it() {
    let stderr = "* Request to https://api.github.com/graphql\n> POST /graphql HTTP/1.1\n>\n\
                  {\n  \"query\": \"mutation { addComment }\",\n  \
                  \"body\": \"kelpie holds on a rate limit\"\n}\n\
                  < HTTP/2.0 502 Bad Gateway\n<\n\
                  {\"message\":\"rate limit text in a body\"}\n\
                  GraphQL: Could not resolve to a node with the global id of 'X'. (node)\n";

    assert_eq!(
        failed(stderr),
        ForgeError::Failed(
            "GraphQL: Could not resolve to a node with the global id of 'X'. (node)".into()
        )
    );
}

#[test]
fn a_graphql_failure_keeps_its_error_line_and_not_the_query_or_its_variables() {
    let stderr = "* Request to https://api.github.com/graphql\n> POST /graphql HTTP/1.1\n>\n\
                  GraphQL query:\nmutation($body: String!) { addComment(body: $body) }\n\
                  GraphQL variables: {\"body\":\"API rate limit exceeded\"}\n\
                  < HTTP/2.0 200 OK\n<\n\
                  GraphQL: Could not resolve to a node with the global id of 'X'. (node)\n";

    assert_eq!(
        failed(stderr),
        ForgeError::Failed(
            "GraphQL: Could not resolve to a node with the global id of 'X'. (node)".into()
        )
    );
}

#[test]
fn a_failure_that_printed_a_page_keeps_only_ghs_status_line() {
    let line = "HTTP 504: 504 Gateway Timeout (https://api.github.com/graphql)";

    assert_eq!(failed(PAGE), ForgeError::Failed(line.into()));
    let exchanged = format!("* Request to https://api.github.com/graphql\n<\n{PAGE}");
    assert_eq!(failed(&exchanged), failed(PAGE));
    assert!(failed(PAGE).passes(), "a 504 may be asked again");
}

#[test]
fn a_retry_after_header_becomes_a_cooldown() {
    let ForgeError::Limited { said, cooldown } = failed(SECONDARY) else {
        panic!("a secondary limit is limited");
    };

    assert_eq!(
        cooldown,
        Cooldown {
            retry_after: Some(75),
            reset: Some(Timestamp::from_secs(1_790_000_600)),
        }
    );
    assert!(said.starts_with("gh failed: gh: You have exceeded a secondary rate limit."));
}

#[test]
fn the_last_responses_headers_are_the_ones_read() {
    let stderr = "< HTTP/2.0 502 Bad Gateway\n< Retry-After: 5\n<\n\
                  < HTTP/2.0 403 Forbidden\n< Retry-After: 120\n<\n\
                  gh: API rate limit exceeded (HTTP 403)\n";

    let ForgeError::Limited { cooldown, .. } = failed(stderr) else {
        panic!("a rate limit is limited");
    };

    assert_eq!(cooldown.retry_after, Some(120));
}

#[test]
fn a_used_up_message_is_limited() {
    for said in [
        "gh: API rate limit exceeded for user ID 1. (HTTP 403)\n",
        "gh: API rate limit already exceeded for user ID 1. (HTTP 403)\n",
        "gh: You have exceeded a secondary rate limit. (HTTP 403)\n",
        "gh: Too Many Requests (HTTP 429)\n",
        "gh: API rate limit exceeded for user ID 1. (HTTP 502)\n",
    ] {
        let ForgeError::Limited { cooldown, .. } = failed(said) else {
            panic!("{said:?} is a used up limit");
        };
        assert_eq!(cooldown, Cooldown::default(), "no headers were printed");
    }
    assert!(matches!(
        failed("gh: Not Found (HTTP 404)\n"),
        ForgeError::Failed(_)
    ));
}

#[test]
fn a_missing_gh_is_a_spawn_error() {
    let script = Script::answering(Err(io::Error::from(io::ErrorKind::NotFound)));

    let called = gh_over(&script).call(&["api", "user"]);

    assert!(matches!(called, Err(ForgeError::Spawn(_))), "{called:?}");
}

#[test]
fn a_failed_call_is_what_failed_makes_of_its_stderr() {
    let script = Script::answering(refused(SECONDARY));

    let called = gh_over(&script).call(&["api", "graphql"]);

    assert_eq!(called, Err(failed(SECONDARY)));
}

#[test]
fn viewer_is_the_login_gh_api_user_names() {
    let script = Script::answering(answered("octocat\n"));

    assert_eq!(gh_over(&script).viewer(), Ok("octocat".to_owned()));
    assert_eq!(script.asked(), [["api", "user", "--jq", ".login"]]);
}

#[test]
fn viewer_with_no_login_is_a_failure_carrying_ghs_own_line() {
    let own = "To get started with GitHub CLI, please run:  gh auth login";
    let script = Script::answering(refused(&format!("{own}\n")));

    assert_eq!(
        gh_over(&script).viewer(),
        Err(ForgeError::Failed(own.into()))
    );
}

#[test]
fn a_viewer_that_is_not_one_word_is_unreadable() {
    for printed in ["", "\n", "two words\n"] {
        let script = Script::answering(answered(printed));

        let viewer = gh_over(&script).viewer();

        assert!(
            matches!(viewer, Err(ForgeError::Unreadable(_))),
            "{viewer:?}"
        );
    }
}

// Every limit but the three kelpie reads is left out
const LIMITS: &str = r#"{"resources":{
    "core":{"limit":5000,"used":0,"remaining":5000,"reset":1791644854},
    "search":{"limit":30,"used":0,"remaining":30,"reset":1791641314},
    "graphql":{"limit":5000,"used":7,"remaining":4993,"reset":1791644766},
    "scim":{"limit":15000,"used":0,"remaining":15000,"reset":1791644854}},
    "rate":{"limit":5000,"used":0,"remaining":5000,"reset":1791644854}}"#;

#[test]
fn the_rate_limit_reads_core_graphql_and_search_with_used_remaining_and_reset() {
    let script = Script::answering(answered(LIMITS));

    let limits = gh_over(&script).rate_limit().unwrap();

    let bucket = |used, remaining, reset| RateBucket {
        used,
        remaining,
        reset: Timestamp::from_secs(reset),
    };
    assert_eq!(limits.core, bucket(0, 5000, 1_791_644_854));
    assert_eq!(limits.graphql, bucket(7, 4993, 1_791_644_766));
    assert_eq!(limits.search, bucket(0, 30, 1_791_641_314));
    assert_eq!(script.asked(), [["api", "rate_limit"]]);
}

#[test]
fn a_rate_limit_answer_with_no_resources_is_unreadable() {
    let script = Script::answering(answered(r#"{"rate":{}}"#));

    let limits = gh_over(&script).rate_limit();

    assert!(
        matches!(limits, Err(ForgeError::Unreadable(_))),
        "{limits:?}"
    );
}
