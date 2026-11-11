//! The open pull requests of a repo, and the tickets each closes
//!
//! One paged GraphQL query reads each pull request's number, head branch and
//! commit, base branch, labels, draft flag, closing references and the checks of
//! its last commit, so the gate reads a pull request once per pass. A ticket is in Review while a pull request
//! closing it is open (`docs/design/tickets.md`, The board).

use serde::Deserialize;

use super::checks::{COMMIT_FIELDS, Commit};
use super::hold::Hold;
use super::unreadable;
use crate::ci::HeadStatus;
use crate::github::ForgeSlug;
use crate::ticket::{ForgeError, OpenPull, TicketId};

// A page holds 100, so each 100 more open pull requests cost one more query
//
// The last commit's checks are the commit read of `checks::COMMIT_FIELDS`, so a
// head's status reads the same either way. A page of 100 pull requests, each with
// its rollup, stays well under GitHub's node limit.
const QUERY_HEAD: &str = "query($owner: String!, $name: String!, $after: String) { \
    repository(owner: $owner, name: $name) { \
    pullRequests(states: OPEN, first: 100, after: $after, \
    orderBy: {field: CREATED_AT, direction: DESC}) { \
    pageInfo { hasNextPage endCursor } nodes { \
    number headRefName headRefOid baseRefName isDraft labels(first: 50) { nodes { name } } \
    closingIssuesReferences(first: 50) { nodes { number repository { name owner { login } } } } \
    commits(last: 1) { nodes { commit { ";
const QUERY_TAIL: &str = " } } } } } } }";

// As many pages as hold a thousand pull requests, past which the read stops
const MAX_PAGES: usize = 10;

/// Every open pull request of `repo`, newest first
pub(super) fn open_pulls(hold: &Hold, repo: &ForgeSlug) -> Result<Vec<OpenPull>, ForgeError> {
    let query = format!("query={QUERY_HEAD}{COMMIT_FIELDS}{QUERY_TAIL}");
    let owner = format!("owner={}", repo.owner());
    let name = format!("name={}", repo.name());
    let mut pulls = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut args = vec!["api", "graphql", "-f", &query, "-f", &owner, "-f", &name];
        let cursor = after.as_ref().map(|cursor| format!("after={cursor}"));
        if let Some(cursor) = &cursor {
            args.extend(["-f", cursor]);
        }
        let page = parse_page(&hold.call(&args)?, repo)?;
        pulls.extend(page.pulls);
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    Ok(pulls)
}

// One page, and the cursor of the next when there is one
struct Page {
    pulls: Vec<OpenPull>,
    next: Option<String>,
}

// GraphQL answers 200 with an `errors` list for a query it could not run, and
// `gh` passes some of those on as a success
fn parse_page(stdout: &[u8], repo: &ForgeSlug) -> Result<Page, ForgeError> {
    #[derive(Deserialize)]
    struct Reply {
        data: Option<Data>,
        #[serde(default)]
        errors: Vec<Problem>,
    }
    #[derive(Deserialize)]
    struct Problem {
        message: String,
    }
    #[derive(Deserialize)]
    struct Data {
        repository: Option<Repository>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Repository {
        pull_requests: Connection,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Connection {
        page_info: PageInfo,
        nodes: Vec<Node>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PageInfo {
        has_next_page: bool,
        end_cursor: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Node {
        number: u64,
        head_ref_name: String,
        head_ref_oid: String,
        base_ref_name: String,
        is_draft: bool,
        labels: Labels,
        closing_issues_references: Closing,
        commits: Commits,
    }
    #[derive(Deserialize)]
    struct Labels {
        nodes: Vec<Label>,
    }
    #[derive(Deserialize)]
    struct Label {
        name: String,
    }
    #[derive(Deserialize)]
    struct Commits {
        nodes: Vec<Tip>,
    }
    #[derive(Deserialize)]
    struct Tip {
        commit: Commit,
    }
    #[derive(Deserialize)]
    struct Closing {
        nodes: Vec<Closes>,
    }
    #[derive(Deserialize)]
    struct Closes {
        number: u64,
        repository: Home,
    }
    #[derive(Deserialize)]
    struct Home {
        name: String,
        owner: Owner,
    }
    #[derive(Deserialize)]
    struct Owner {
        login: String,
    }
    let reply: Reply = serde_json::from_slice(stdout).map_err(|_| unreadable(stdout))?;
    if !reply.errors.is_empty() {
        let said: Vec<_> = reply.errors.into_iter().map(|e| e.message).collect();
        return Err(ForgeError::Failed(said.join("; ")));
    }
    let listed = reply
        .data
        .and_then(|data| data.repository)
        .ok_or_else(|| unreadable(stdout))?
        .pull_requests;
    let next = listed
        .page_info
        .has_next_page
        .then_some(listed.page_info.end_cursor)
        .flatten();
    // A pull request can close tickets of other repos, which are not this board's
    let here = |home: &Home| {
        home.owner.login.eq_ignore_ascii_case(repo.owner())
            && home.name.eq_ignore_ascii_case(repo.name())
    };
    let pulls = listed
        .nodes
        .into_iter()
        .filter_map(|node| {
            Some(OpenPull {
                number: std::num::NonZeroU64::new(node.number)?,
                branch: node.head_ref_name,
                draft: node.is_draft,
                head: node.head_ref_oid,
                base: node.base_ref_name,
                labels: node.labels.nodes.into_iter().map(|l| l.name).collect(),
                checks: (node.commits.nodes.into_iter().next())
                    .map_or(HeadStatus::None, |tip| tip.commit.head_status()),
                closes: node
                    .closing_issues_references
                    .nodes
                    .into_iter()
                    .filter(|closes| here(&closes.repository))
                    .filter_map(|closes| TicketId::from_issue(closes.number))
                    .collect(),
            })
        })
        .collect();
    Ok(Page { pulls, next })
}
