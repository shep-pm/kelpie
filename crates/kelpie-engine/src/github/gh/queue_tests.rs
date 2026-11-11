//! The merge queue's standing, read through a scripted [`Runner`], so no test
//! needs `gh`, a login or the network
//!
//! [`Runner`]: super::Runner

use std::num::NonZeroU64;

use super::hold::Hold;
use super::queue::GithubQueue;
use super::tests::{Script, answered, gh_over};
use crate::github::ForgeSlug;
use crate::ticket::{ForgeError, MergeQueue, QueueStanding};

fn queue_over(script: &Script) -> GithubQueue {
    let repo = ForgeSlug::try_from("shep-pm/koji".to_owned()).unwrap();
    GithubQueue::new(Hold::new(gh_over(script)), repo)
}

fn number(n: u64) -> NonZeroU64 {
    NonZeroU64::new(n).unwrap()
}

fn reply(pr: &str) -> String {
    format!(r#"{{"data":{{"repository":{{"pullRequest":{pr}}}}}}}"#)
}

fn standing_of(pr: &str) -> Result<QueueStanding, ForgeError> {
    let script = Script::answering(answered(&reply(pr)));
    queue_over(&script).standing(number(7))
}

#[test]
fn a_pull_in_the_queue_reads_as_queued() {
    let script = Script::answering(answered(&reply(
        r#"{"isInMergeQueue":true,"autoMergeRequest":null,"timelineItems":{"nodes":[]}}"#,
    )));

    let standing = queue_over(&script).standing(number(7)).unwrap();

    assert_eq!(
        standing,
        QueueStanding {
            queued: true,
            armed: false,
            removals: 0,
            reason: None
        }
    );
    // One GraphQL call, with the repo and the number as typed variables
    let asked = script.asked();
    assert_eq!(asked.len(), 1);
    let args = &asked[0];
    assert_eq!(args[..3], ["api", "graphql", "-f"]);
    assert!(args[3].starts_with("query=query("), "{args:?}");
    assert_eq!(
        args[4..],
        ["-f", "owner=shep-pm", "-f", "name=koji", "-F", "number=7"]
    );
}

#[test]
fn an_armed_auto_merge_reads_as_armed() {
    let standing = standing_of(
        r#"{"isInMergeQueue":false,"autoMergeRequest":{"enabledAt":"2026-09-30T10:00:00Z"},
        "timelineItems":{"nodes":[]}}"#,
    )
    .unwrap();

    assert!(standing.armed && !standing.queued, "{standing:?}");
}

#[test]
fn each_removal_is_counted_and_the_latest_reason_kept() {
    let standing = standing_of(
        r#"{"isInMergeQueue":false,"autoMergeRequest":null,"timelineItems":{"nodes":[
        {"reason":"older"},{"reason":"Required status check \"test\" failed."}]}}"#,
    )
    .unwrap();

    assert_eq!(
        standing,
        QueueStanding {
            queued: false,
            armed: false,
            removals: 2,
            reason: Some("Required status check \"test\" failed.".into()),
        }
    );
}

#[test]
fn an_answer_in_another_shape_is_unreadable() {
    // No such pull request, and text that is not the reply at all
    assert!(matches!(
        standing_of("null"),
        Err(ForgeError::Unreadable(_))
    ));
    let script = Script::answering(answered("not json"));
    assert_eq!(
        queue_over(&script).standing(number(7)),
        Err(ForgeError::Unreadable("not json".into()))
    );
}
