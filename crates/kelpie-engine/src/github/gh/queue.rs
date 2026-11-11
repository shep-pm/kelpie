//! Where a pull request stands in the merge queue, read from GitHub
//!
//! `gh pr view --json` has no merge queue field, so the standing is one GraphQL
//! read, as the login `gh` holds, through a [`Hold`].

use std::num::NonZeroU64;

use serde::Deserialize;

use super::hold::Hold;
use super::unreadable;
use crate::github::ForgeSlug;
use crate::ticket::{ForgeError, MergeQueue, QueueStanding};

// The removals are timeline events, and the latest one's reason is all GitHub
// says about why. The filtered `totalCount` counts every timeline item, so the
// nodes are counted.
const QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { \
    repository(owner: $owner, name: $name) { pullRequest(number: $number) { \
    isInMergeQueue autoMergeRequest { enabledAt } \
    timelineItems(itemTypes: [REMOVED_FROM_MERGE_QUEUE_EVENT], last: 100) { \
    nodes { ... on RemovedFromMergeQueueEvent { reason } } } } } }";

/// The merge queue of one GitHub repo's base branch, read through a [`Hold`]
#[derive(Debug)]
pub struct GithubQueue {
    hold: Hold,
    repo: ForgeSlug,
}

impl GithubQueue {
    /// The merge queue of `repo`, asked through `hold`
    #[must_use]
    pub fn new(hold: Hold, repo: ForgeSlug) -> Self {
        Self { hold, repo }
    }
}

impl MergeQueue for GithubQueue {
    fn standing(&self, number: NonZeroU64) -> Result<QueueStanding, ForgeError> {
        let (owner, name) = self.repo.as_str().split_once('/').unwrap_or_default();
        let query = format!("query={QUERY}");
        let owner = format!("owner={owner}");
        let name = format!("name={name}");
        let number = format!("number={number}");
        let out = self.hold.call(&[
            "api", "graphql", "-f", &query, "-f", &owner, "-f", &name, "-F", &number,
        ])?;
        parse_standing(&out)
    }
}

fn parse_standing(stdout: &[u8]) -> Result<QueueStanding, ForgeError> {
    #[derive(Deserialize)]
    struct Reply {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        repository: Repository,
    }
    #[derive(Deserialize)]
    struct Repository {
        #[serde(rename = "pullRequest")]
        pull_request: Option<Pr>,
    }
    #[derive(Deserialize)]
    struct Pr {
        #[serde(rename = "isInMergeQueue")]
        queued: bool,
        #[serde(rename = "autoMergeRequest")]
        auto_merge: Option<serde_json::Value>,
        #[serde(rename = "timelineItems")]
        removals: Removals,
    }
    #[derive(Deserialize)]
    struct Removals {
        nodes: Vec<Removal>,
    }
    #[derive(Deserialize)]
    struct Removal {
        reason: Option<String>,
    }
    let reply: Reply = serde_json::from_slice(stdout).map_err(|_| unreadable(stdout))?;
    let pr = reply
        .data
        .repository
        .pull_request
        .ok_or_else(|| unreadable(stdout))?;
    Ok(QueueStanding {
        queued: pr.queued,
        armed: pr.auto_merge.is_some(),
        removals: u32::try_from(pr.removals.nodes.len()).unwrap_or(u32::MAX),
        reason: pr.removals.nodes.into_iter().last().and_then(|r| r.reason),
    })
}
