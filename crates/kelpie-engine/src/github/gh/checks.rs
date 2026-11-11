//! A commit's checks, read into where CI stands on its head
//!
//! GitHub records two things on a commit. The status check rollup holds each
//! check run and each commit status an outside service posted, and the commit's
//! check suites hold one entry per workflow run. A workflow file GitHub cannot
//! read still records a failed run on the commit, but with no check run, so the
//! rollup never names it, and a workflow held for approval leaves no check run
//! either. The suites name both, so the rollup is read beside them: a workflow
//! that started no job turns the rollup red, and a held one keeps it pending.
//!
//! One GraphQL query reads both for a head (SK's `gh/mod.rs` rollup and
//! `gh/suites.rs`, `docs/lifted.md`).

use serde::Deserialize;

use super::hold::Hold;
use super::unreadable;
use crate::ci::{FailedCheck, HeadStatus, Queued};
use crate::github::ForgeSlug;
use crate::ticket::ForgeError;

// The rollup's first hundred contexts, and the first fifty suites. A check run
// names its workflow through its suite's run, since `CheckRun` names none of its
// own. The suites count their check runs by `totalCount`, which is all
// `Suites::fold` reads of them.
const QUERY: &str = "query($owner: String!, $name: String!, $oid: GitObjectID!) { \
    repository(owner: $owner, name: $name) { object(oid: $oid) { ... on Commit { \
    statusCheckRollup { contexts(first: 100) { nodes { __typename \
    ... on CheckRun { name status conclusion completedAt startedAt detailsUrl \
    checkSuite { workflowRun { workflow { name } } } } \
    ... on StatusContext { context state targetUrl } } } } \
    checkSuites(first: 50) { nodes { conclusion checkRuns(first: 1) { totalCount } \
    workflowRun { url file { path } } } } \
    } } } }";

/// Where CI stands on `head` of `repo`
pub(super) fn head_status(
    hold: &Hold,
    repo: &ForgeSlug,
    head: &str,
) -> Result<HeadStatus, ForgeError> {
    let query = format!("query={QUERY}");
    let owner = format!("owner={}", repo.owner());
    let name = format!("name={}", repo.name());
    let oid = format!("oid={head}");
    let stdout = hold.call(&[
        "api", "graphql", "-f", &query, "-f", &owner, "-f", &name, "-f", &oid,
    ])?;
    parse_head(&stdout)
}

// A head GitHub does not know has no object, and so no checks
pub(super) fn parse_head(stdout: &[u8]) -> Result<HeadStatus, ForgeError> {
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
        object: Option<Commit>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Commit {
        status_check_rollup: Option<Rollup>,
        #[serde(default)]
        check_suites: Suites,
    }
    #[derive(Deserialize)]
    struct Rollup {
        contexts: Contexts,
    }
    #[derive(Deserialize)]
    struct Contexts {
        nodes: Vec<Check>,
    }
    let reply: Reply = serde_json::from_slice(stdout).map_err(|_| unreadable(stdout))?;
    let Some(commit) = reply.data.repository.object else {
        return Ok(HeadStatus::None);
    };
    let contexts = commit
        .status_check_rollup
        .map_or_else(Vec::new, |rollup| rollup.contexts.nodes);
    Ok(commit.check_suites.fold(status(&contexts)))
}

// GitHub has two kinds of check: an Actions-style check run, and a commit
// status posted by an outside service such as CodeRabbit.
#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum Check {
    #[serde(rename_all = "camelCase")]
    CheckRun {
        name: String,
        status: Option<String>,
        conclusion: Option<String>,
        completed_at: Option<String>,
        started_at: Option<String>,
        details_url: Option<String>,
        check_suite: Option<CheckSuite>,
    },
    #[serde(rename_all = "camelCase")]
    StatusContext {
        context: String,
        state: Option<String>,
        target_url: Option<String>,
    },
}

// A check run's suite, whose run names the workflow. A check run no workflow
// posted, such as a review bot's, has no run.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CheckSuite {
    workflow_run: Option<SuiteRun>,
}

#[derive(Deserialize)]
struct SuiteRun {
    workflow: Option<SuiteWorkflow>,
}

#[derive(Deserialize)]
struct SuiteWorkflow {
    name: String,
}

enum Outcome {
    Pending,
    Passed,
    Skipped,
    Failed,
}

// What GitHub writes as the `completedAt` of a check run that has not finished
const NOT_COMPLETED: &str = "0001-01-01T00:00:00Z";

impl Check {
    // The check's name, its link, and how it stands
    fn outcome(&self) -> (&str, Option<&str>, Outcome) {
        match self {
            Self::CheckRun {
                name,
                conclusion,
                details_url,
                ..
            } => {
                let outcome = if self.finished() {
                    match conclusion.as_deref() {
                        Some("SKIPPED") => Outcome::Skipped,
                        Some("SUCCESS" | "NEUTRAL") => Outcome::Passed,
                        _ => Outcome::Failed,
                    }
                } else {
                    Outcome::Pending
                };
                (name, details_url.as_deref(), outcome)
            }
            Self::StatusContext {
                context,
                state,
                target_url,
            } => {
                let outcome = match state.as_deref() {
                    Some("SUCCESS") => Outcome::Passed,
                    Some("FAILURE" | "ERROR") => Outcome::Failed,
                    _ => Outcome::Pending,
                };
                (context, target_url.as_deref(), outcome)
            }
        }
    }

    // A check run is finished when its status says so, or when it has a time
    // it completed at and a conclusion: GitHub sometimes leaves the status at
    // `IN_PROGRESS` or `QUEUED` after the run ended.
    fn finished(&self) -> bool {
        match self {
            Self::CheckRun {
                status,
                conclusion,
                completed_at,
                ..
            } => {
                status.as_deref() == Some("COMPLETED")
                    || (conclusion.as_deref().is_some_and(|c| !c.is_empty())
                        && (completed_at.as_deref())
                            .is_some_and(|at| !at.is_empty() && at != NOT_COMPLETED))
            }
            Self::StatusContext { .. } => false,
        }
    }

    // The check run that is still queued or in progress, which the gate reruns
    // once if its workflow run finished: GitHub leaves a job of a finished run in
    // either state for good, and a commit status has no run.
    fn queued(&self) -> Option<Queued> {
        match self {
            Self::CheckRun {
                name,
                status: Some(status),
                details_url,
                ..
            } if matches!(status.as_str(), "QUEUED" | "IN_PROGRESS") && !self.finished() => {
                Some(Queued {
                    name: name.clone(),
                    run: details_url.as_deref().and_then(run_of),
                    running: status == "IN_PROGRESS",
                    held: false,
                })
            }
            _ => None,
        }
    }

    // The workflow a check run belongs to
    fn workflow(&self) -> Option<&str> {
        match self {
            Self::CheckRun {
                check_suite: Some(suite),
                ..
            } => Some(&suite.workflow_run.as_ref()?.workflow.as_ref()?.name),
            _ => None,
        }
    }

    // A run of the same check as `other`: the same name under the same workflow.
    // A rollup naming no workflow keys on the name alone.
    fn same_check(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::CheckRun { name, .. },
                Self::CheckRun {
                    name: other_name, ..
                },
            ) => name == other_name && self.workflow() == other.workflow(),
            _ => false,
        }
    }

    // The time a check run started, which GitHub writes for every one
    fn started(&self) -> Option<jiff::Timestamp> {
        match self {
            Self::CheckRun { started_at, .. } => started_at
                .as_deref()
                .and_then(|at| at.parse::<jiff::Timestamp>().ok()),
            Self::StatusContext { .. } => None,
        }
    }

    // Ended because a later run of its workflow replaced it
    fn cancelled(&self) -> bool {
        matches!(self, Self::CheckRun { conclusion, .. }
            if conclusion.as_deref() == Some("CANCELLED"))
    }

    // Whether a later run of the same check stands in for this cancelled one
    fn superseded_by(&self, other: &Self) -> bool {
        let (Some(at), Some(later)) = (self.started(), other.started()) else {
            return false;
        };
        self.cancelled() && self.same_check(other) && later > at
    }
}

// The workflow run a check run's link, `.../actions/runs/<id>/job/<id>`, names
pub(super) fn run_of(link: &str) -> Option<u64> {
    let (_, tail) = link.split_once("/actions/runs/")?;
    tail.split('/').next()?.parse().ok()
}

// A cancelled run of a check that ran again after it on the head. A concurrency
// group cancels the run it supersedes. A closed and reopened pull request keeps
// its first runs in the rollup beside the new ones.
fn superseded(check: &Check, runs: &[&Check]) -> bool {
    runs.iter().copied().any(|other| check.superseded_by(other))
}

// The whole rollup is read before a verdict, so a red head names every failure.
fn status(contexts: &[Check]) -> HeadStatus {
    if contexts.is_empty() {
        return HeadStatus::None;
    }
    let all: Vec<&Check> = contexts.iter().collect();
    let rollup: Vec<&Check> = all
        .iter()
        .copied()
        .filter(|c| !superseded(c, &all))
        .collect();
    let mut failed = Vec::new();
    let mut ran = false;
    let mut pending = false;
    for check in &rollup {
        match check.outcome() {
            (_, _, Outcome::Pending) => pending = true,
            (name, url, Outcome::Failed) => failed.push(FailedCheck {
                name: name.to_owned(),
                url: url.map(str::to_owned),
            }),
            (_, _, Outcome::Passed) => ran = true,
            (_, _, Outcome::Skipped) => {}
        }
    }
    if pending {
        return HeadStatus::Pending(rollup.iter().filter_map(|c| c.queued()).collect());
    }
    if !failed.is_empty() {
        return HeadStatus::Failed(failed);
    }
    if ran {
        HeadStatus::Passed
    } else {
        HeadStatus::Skipped
    }
}

/// A commit's check suites, as `QUERY` asks for them
#[derive(Deserialize, Default)]
struct Suites {
    nodes: Vec<Suite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Suite {
    conclusion: Option<String>,
    check_runs: Count,
    // None for a suite another App posts, such as a review bot's
    workflow_run: Option<Run>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Count {
    total_count: u64,
}

#[derive(Deserialize)]
struct Run {
    url: Option<String>,
    file: Option<File>,
}

#[derive(Deserialize)]
struct File {
    path: String,
}

impl Suites {
    // The suites that named a workflow and ended with `conclusions`, no check run
    // among them
    fn without_check_runs<'a>(
        &'a self,
        conclusions: &'a [&str],
    ) -> impl Iterator<Item = (&'a Run, &'a File)> {
        self.nodes
            .iter()
            .filter(|s| s.check_runs.total_count == 0)
            .filter(|s| {
                s.conclusion
                    .as_deref()
                    .is_some_and(|c| conclusions.contains(&c))
            })
            .filter_map(|s| {
                let run = s.workflow_run.as_ref()?;
                Some((run, run.file.as_ref()?))
            })
    }

    // Each workflow that failed with no check run, by its file
    fn unstarted(&self) -> Vec<FailedCheck> {
        self.without_check_runs(&["FAILURE", "STARTUP_FAILURE"])
            .map(|(run, file)| FailedCheck {
                name: format!("{}: started no job", file.path),
                url: run.url.clone(),
            })
            .collect()
    }

    fn held(&self) -> Vec<Queued> {
        self.without_check_runs(&["ACTION_REQUIRED"])
            .map(|(run, file)| Queued {
                name: file.path.clone(),
                run: run.url.as_deref().and_then(run_of),
                running: false,
                held: true,
            })
            .collect()
    }

    // `status` with held workflows pending and workflows that could not start failed
    fn fold(&self, status: HeadStatus) -> HeadStatus {
        let held = self.held();
        if !held.is_empty() {
            let mut queued = match status {
                HeadStatus::Pending(queued) => queued,
                _ => Vec::new(),
            };
            queued.extend(held);
            return HeadStatus::Pending(queued);
        }
        let unstarted = self.unstarted();
        if unstarted.is_empty() {
            return status;
        }
        match status {
            HeadStatus::Pending(_) => status,
            HeadStatus::Failed(mut failed) => {
                failed.extend(unstarted);
                HeadStatus::Failed(failed)
            }
            HeadStatus::None | HeadStatus::Passed | HeadStatus::Skipped => {
                HeadStatus::Failed(unstarted)
            }
        }
    }
}
