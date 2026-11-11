//! The runs of a head that failed, the end of their logs, and the rerun of a run
//!
//! A worker's token pushes and does nothing else, so `gh run view` is refused
//! it. Kelpie reads the failed runs' logs as the login `gh` acts as and puts the
//! end of each in the worker's prompt (SK's `gh/runs/mod.rs` and `timeout.rs`,
//! `docs/lifted.md`). Every call is asked through the [`Hold`] the caller holds.

use std::collections::HashSet;

use serde::Deserialize;

use super::hold::Hold;
use super::unreadable;
use crate::ci::FailedLog;
use crate::github::ForgeSlug;
use crate::ticket::ForgeError;

/// How many failed runs are read
pub(super) const RUNS: usize = 3;

/// How much of the end of each log is kept, in bytes: where a failure says why
pub(super) const LOG_BYTES: usize = 6000;

/// The logs of the runs of `head` that failed, or of the merge queue's, at most [`RUNS`]
pub(super) fn failed_logs(
    hold: &Hold,
    repo: &ForgeSlug,
    head: &str,
    merge_group: bool,
) -> Result<Vec<FailedLog>, ForgeError> {
    // A head is asked for by its commit, since the repo's newest runs on every
    // branch would push its older failed run out of the list
    let (filter, value) = match merge_group {
        true => ("--event", "merge_group"),
        false => ("--commit", head),
    };
    let listed = hold.call(&[
        "run",
        "list",
        "--repo",
        repo.as_str(),
        filter,
        value,
        "--limit",
        "50",
        "--json",
        "databaseId,conclusion,headSha,event,workflowName,url",
    ])?;
    let mut logs = Vec::new();
    for run in failed_runs(&listed, head, merge_group)?
        .into_iter()
        .take(RUNS)
    {
        let tail = match read_run(repo, run.id, run.conclusion != "failure", &mut |args| {
            hold.call(args)
        }) {
            Ok(read) => read.text,
            // A limit used up is every later call's answer too
            Err(error @ (ForgeError::Held(_) | ForgeError::Limited { .. })) => return Err(error),
            // A log GitHub no longer keeps must not hide the other runs' logs
            Err(error) => format!("note: the log of this run could not be read: {error}"),
        };
        logs.push(FailedLog {
            check: run.workflow,
            run: run.id,
            url: run.url,
            tail,
        });
    }
    Ok(logs)
}

/// Starts run `run` again, every job of it
pub(super) fn rerun(hold: &Hold, repo: &ForgeSlug, run: u64) -> Result<(), ForgeError> {
    let id = run.to_string();
    hold.call(&["run", "rerun", &id, "--repo", repo.as_str()])
        .map(drop)
}

/// A run that ended red
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FailedRun {
    pub(super) id: u64,
    /// How the forge worded its end
    pub(super) conclusion: String,
    /// The workflow's name
    pub(super) workflow: String,
    pub(super) url: String,
}

// How a run concludes when kelpie reads its log: a job `timeout-minutes` stops
// ends `cancelled`, and a run can also conclude `timed_out`
const READ_FOR_A_LOG: [&str; 3] = ["failure", "timed_out", "cancelled"];

// Every run of `head`, or of the merge queue, that ended red, newest first
pub(super) fn failed_runs(
    listed: &[u8],
    head: &str,
    merge_group: bool,
) -> Result<Vec<FailedRun>, ForgeError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Run {
        database_id: u64,
        conclusion: String,
        head_sha: String,
        event: String,
        workflow_name: String,
        url: String,
    }
    let runs: Vec<Run> = serde_json::from_slice(listed).map_err(|_| unreadable(listed))?;
    // A run a concurrency group cancelled for a later run of its workflow on the
    // head was stopped by that run, not by a timeout, so the later run is read
    let replaced: Vec<(String, String, u64)> = runs
        .iter()
        .filter(|r| r.conclusion == "cancelled")
        .filter(|r| {
            (runs.iter()).any(|later| {
                later.workflow_name == r.workflow_name
                    && later.head_sha == r.head_sha
                    && later.database_id > r.database_id
            })
        })
        .map(|r| (r.workflow_name.clone(), r.head_sha.clone(), r.database_id))
        .collect();
    Ok(runs
        .into_iter()
        .filter(|r| {
            let key = (r.workflow_name.clone(), r.head_sha.clone(), r.database_id);
            !replaced.contains(&key)
        })
        .filter(|r| READ_FOR_A_LOG.contains(&r.conclusion.as_str()))
        .filter(|r| match merge_group {
            // GitHub cancels the runs of the groups queued behind one it
            // removes, and nothing ties a queue run to this head, so a
            // `cancelled` one is no sign of a timeout here
            true => r.event == "merge_group" && r.conclusion != "cancelled",
            false => r.head_sha == head,
        })
        .map(|r| FailedRun {
            id: r.database_id,
            conclusion: r.conclusion,
            workflow: r.workflow_name,
            url: r.url,
        })
        .collect())
}

/// What kelpie read of one run's log, which says in its first line when a job's timeout cut the run short
#[derive(Debug)]
pub(super) struct RunLog {
    pub(super) text: String,
}

// The end of `run`'s failed log. Where `--log-failed` answers nothing and the
// run may have been cut short (`cut_short`: it did not conclude `failure`), the
// log is the cancelled job's, after a line saying its timeout ended it. A
// fallback that cannot be read leaves the empty log, as before it existed.
pub(super) fn read_run(
    repo: &ForgeSlug,
    run: u64,
    cut_short: bool,
    ask: &mut impl FnMut(&[&str]) -> Result<Vec<u8>, ForgeError>,
) -> Result<RunLog, ForgeError> {
    let id = run.to_string();
    let repo = repo.as_str();
    let log = ask(&["run", "view", &id, "--repo", repo, "--log-failed"])?;
    let tail = failure_tail(&String::from_utf8_lossy(&log), LOG_BYTES);
    if !tail.is_empty() || !cut_short {
        return Ok(RunLog { text: tail });
    }
    Ok(cancelled_log(&id, repo, ask)
        .ok()
        .flatten()
        .unwrap_or(RunLog { text: tail }))
}

// The log of the job of run `id` that a timeout cancelled, if it has one
fn cancelled_log(
    id: &str,
    repo: &str,
    ask: &mut impl FnMut(&[&str]) -> Result<Vec<u8>, ForgeError>,
) -> Result<Option<RunLog>, ForgeError> {
    let jobs = ask(&["run", "view", id, "--json", "jobs", "--repo", repo])?;
    let Some(job) = cancelled_job(&jobs)? else {
        return Ok(None);
    };
    let job_id = job.id.to_string();
    let log = ask(&["run", "view", id, "--job", &job_id, "--log", "--repo", repo])?;
    let tail = failure_tail(&String::from_utf8_lossy(&log), LOG_BYTES);
    Ok(Some(RunLog {
        text: format!("{}\n{tail}", job.note()).trim_end().to_owned(),
    }))
}

// The job of a run that ended `cancelled`, and the step it was cancelled in. A
// job stopped by its `timeout-minutes` ends so, and `gh run view --log-failed`
// answers nothing for its run: the listing names the job and the step, and
// `gh run view --job` reads that job's log.
#[derive(Debug, PartialEq, Eq)]
struct CancelledJob {
    // The id `gh run view --job` takes
    id: u64,
    name: String,
    step: String,
    // Seconds from the job's start to its finish, where the listing gave both
    elapsed: Option<i64>,
}

impl CancelledJob {
    // The line that tells a reader the job ran out of time and no test failed
    fn note(&self) -> String {
        let after = self.elapsed.map_or(String::new(), |secs| match secs {
            0..=59 => format!(" after {secs}s"),
            _ => format!(" after {}m{:02}s", secs / 60, secs % 60),
        });
        format!(
            "note: job `{}` was cancelled in step `{}`{after}: its `timeout-minutes` ended it, \
             so this is a slow job and not a failing test",
            self.name, self.step
        )
    }
}

// The first job of `listing` (`gh run view <id> --json jobs`) that ended
// `cancelled` in a step that did too, or none
fn cancelled_job(listing: &[u8]) -> Result<Option<CancelledJob>, ForgeError> {
    #[derive(Deserialize)]
    struct Listing {
        jobs: Vec<Job>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Job {
        database_id: u64,
        name: String,
        conclusion: String,
        started_at: Option<String>,
        completed_at: Option<String>,
        steps: Vec<Step>,
    }
    #[derive(Deserialize)]
    struct Step {
        name: String,
        conclusion: String,
    }
    let listing: Listing = serde_json::from_slice(listing).map_err(|_| unreadable(listing))?;
    Ok(listing.jobs.into_iter().find_map(|job| {
        if job.conclusion != "cancelled" {
            return None;
        }
        let step = job
            .steps
            .into_iter()
            .find(|s| s.conclusion == "cancelled")?;
        let stamp = |at: Option<String>| at?.parse::<jiff::Timestamp>().ok();
        let elapsed = match (stamp(job.started_at), stamp(job.completed_at)) {
            (Some(start), Some(end)) => Some(end.as_second() - start.as_second()),
            _ => None,
        };
        Some(CancelledJob {
            id: job.database_id,
            name: job.name,
            step: step.name,
            elapsed,
        })
    }))
}

// What of `log` a worker reads, in at most `max` bytes: the failure lines of
// the log with the lines after them, then its end. The log's progress lines,
// GitHub's timestamps and terminal colour codes are gone, so the end is the
// failure and not the build before it. A log that fits whole is kept whole.
pub(super) fn failure_tail(log: &str, max: usize) -> String {
    let kept: Vec<String> = log
        .lines()
        .filter_map(|line| {
            // `gh run view --log-failed` puts the job and step before the text
            let mut parts = line.splitn(3, '\t');
            let (names, text) = match (parts.next(), parts.next(), parts.next()) {
                (Some(job), Some(step), Some(text)) => (format!("{job}\t{step}\t"), text),
                _ => (String::new(), line),
            };
            let text = strip_colour(strip_timestamp(text));
            (!is_progress(&text)).then(|| format!("{names}{text}"))
        })
        .collect();
    let cleaned = kept.join("\n");
    if cleaned.len() <= max {
        return cleaned.trim_end().to_owned();
    }
    let failures = failure_blocks(&kept, max / 2);
    if failures.is_empty() {
        return tail(&cleaned, max).to_owned();
    }
    let head = format!("Failures:\n{failures}\nLog end:\n");
    format!("{head}{}", tail(&cleaned, max.saturating_sub(head.len())))
}

// How many lines after a failure line belong to it
const FAILURE_LINES: usize = 15;

// The lines of `lines` that hold a failure and the `FAILURE_LINES` after each,
// in log order and whole lines, within `max` bytes
fn failure_blocks(lines: &[String], max: usize) -> String {
    let mut seen = HashSet::new();
    let mut blocks = String::new();
    let mut next = 0;
    for (at, line) in lines.iter().enumerate() {
        if !is_failure(line) || !seen.insert(line.as_str()) {
            continue;
        }
        let end = (at + FAILURE_LINES + 1).min(lines.len());
        for line in &lines[at.max(next)..end] {
            let added = line.len() + usize::from(!blocks.is_empty());
            if blocks.len() + added > max {
                return blocks.trim_end().to_owned();
            }
            if !blocks.is_empty() {
                blocks.push('\n');
            }
            blocks.push_str(line);
        }
        next = end;
    }
    blocks.trim_end().to_owned()
}

// Whether `line` says a test or a step failed
fn is_failure(line: &str) -> bool {
    if line.contains("##[error]Process completed with exit code") {
        return false;
    }
    let test_output = line
        .split_once("---- ")
        .is_some_and(|(_, rest)| rest.contains("stdout ----"));
    test_output
        || [
            " FAIL ",
            "Error:",
            "error[E",
            "error: could not compile",
            "error: test failed",
            "AssertionError",
            "error TS",
            "##[error]",
            "panicked at",
            "test result: FAILED",
        ]
        .iter()
        .any(|marker| line.contains(marker))
}

// The verbs cargo prints as it builds, with the width it right-aligns them to
const PROGRESS_WIDTH: usize = 12;
const PROGRESS: [&str; 5] = [
    "Compiling",
    "Downloaded",
    "Downloading",
    "Checking",
    "Fresh",
];

// A line cargo prints as it builds: its verb right-aligned to 12 columns,
// then a crate and its version, or the `crates` a download counts. A test's
// own line that starts with the same word is not indented so, and stays.
fn is_progress(text: &str) -> bool {
    let word = text.trim_start_matches(' ');
    let indent = text.len() - word.len();
    PROGRESS.iter().any(|verb| {
        indent + verb.len() == PROGRESS_WIDTH
            && word
                .strip_prefix(verb)
                .and_then(|rest| rest.strip_prefix(' '))
                .is_some_and(|rest| {
                    rest.split(' ').any(|w| {
                        w == "crates"
                            || w.strip_prefix('v')
                                .is_some_and(|v| v.starts_with(|c: char| c.is_ascii_digit()))
                    })
                })
    })
}

// `text` without the `2026-10-13T10:00:00.0000000Z ` GitHub puts before it,
// and the byte order mark the first line of a log starts with
fn strip_timestamp(text: &str) -> &str {
    let text = text.trim_start_matches('\u{feff}');
    match text.split_once(' ') {
        Some((stamp, rest)) if is_timestamp(stamp) => rest,
        _ => text,
    }
}

// `2026-10-13T10:00:00.0000000Z`
fn is_timestamp(word: &str) -> bool {
    let b = word.as_bytes();
    b.len() >= 20
        && b.ends_with(b"Z")
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[10] == b'T'
        && b[11..b.len() - 1]
            .iter()
            .all(|c| c.is_ascii_digit() || matches!(c, b':' | b'.'))
}

// `text` without its ANSI escape sequences, such as the colour cargo writes
// under `CARGO_TERM_COLOR=always`
fn strip_colour(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.next_if_eq(&'[').is_some() {
            // A sequence ends at its first byte from `@` to `~`
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

// The last `max` bytes of `log`, from the start of a line where one is near
pub(super) fn tail(log: &str, max: usize) -> &str {
    if log.len() <= max {
        return log.trim_end();
    }
    let mut from = log.len() - max;
    while !log.is_char_boundary(from) {
        from += 1;
    }
    let cut = &log[from..];
    cut.split_once('\n')
        .map_or(cut, |(_, rest)| rest)
        .trim_end()
}
