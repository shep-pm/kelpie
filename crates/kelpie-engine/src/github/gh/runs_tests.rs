//! The failed runs of a head, the end of their logs, and the rerun, from recorded `gh` output
//!
//! No test needs `gh`, a login or the network. The run listings are the shape
//! `gh run list --json` prints, the fixtures are SK's recordings of `gh run
//! view`, and a scripted `gh` stands in for the calls.

use super::actions::GithubActions;
use super::hold::Hold;
use super::runs::{LOG_BYTES, RunLog, failed_runs, failure_tail, read_run, tail};
use super::tests::{Script, answered, gh_over};
use crate::ci::{Ci, FailedLog};
use crate::github::ForgeSlug;
use crate::ticket::ForgeError;

fn repo() -> ForgeSlug {
    ForgeSlug::try_from("shep-pm/kelpie".to_owned()).unwrap()
}

fn actions_over(script: &Script) -> GithubActions {
    GithubActions::new(Hold::new(gh_over(script)), repo())
}

// The listing `gh run list --json databaseId,conclusion,headSha,event,workflowName,url`
// prints for runs of `(id, conclusion, event, head)`, newest first
fn listing(runs: &[(u64, &str, &str, &str)]) -> String {
    let runs: Vec<String> = runs
        .iter()
        .map(|(id, conclusion, event, head)| {
            format!(
                r#"{{"conclusion":"{conclusion}","databaseId":{id},"event":"{event}","headSha":"{head}",
                "url":"https://github.com/shep-pm/kelpie/actions/runs/{id}","workflowName":"test"}}"#
            )
        })
        .collect();
    format!("[{}]", runs.join(","))
}

fn ids(runs: Vec<super::runs::FailedRun>) -> Vec<u64> {
    runs.into_iter().map(|r| r.id).collect()
}

fn listed() -> String {
    listing(&[
        (4, "failure", "merge_group", "queue"),
        (3, "failure", "pull_request", "abc"),
        (2, "success", "pull_request", "abc"),
        (1, "failure", "pull_request", "old"),
    ])
}

#[test]
fn a_pull_requests_failed_runs_are_those_of_its_head() {
    assert_eq!(
        ids(failed_runs(listed().as_bytes(), "abc", false).unwrap()),
        [3]
    );
}

#[test]
fn every_failed_run_of_a_head_is_listed_not_only_the_ones_whose_logs_are_read() {
    let runs = listing(&[
        (4, "failure", "pull_request", "abc"),
        (3, "failure", "pull_request", "abc"),
        (2, "failure", "pull_request", "abc"),
        (1, "failure", "pull_request", "abc"),
    ]);

    assert_eq!(
        ids(failed_runs(runs.as_bytes(), "abc", false).unwrap()),
        [4, 3, 2, 1]
    );
}

#[test]
fn the_queues_failed_runs_are_those_of_the_merge_group_event() {
    assert_eq!(
        ids(failed_runs(listed().as_bytes(), "abc", true).unwrap()),
        [4]
    );
}

// A job `timeout-minutes` stops ends `cancelled`, and so does its run
// (`gh-run-view-jobs-timed-out.json`), so a run is read for a log whichever of
// the three it concluded
#[test]
fn a_run_ended_by_its_timeout_is_listed_for_a_log() {
    let runs = listing(&[
        (9, "success", "merge_group", "queue"),
        (8, "cancelled", "pull_request", "abc"),
        (7, "timed_out", "pull_request", "abc"),
        (6, "success", "pull_request", "abc"),
        (5, "failure", "pull_request", "abc"),
        (4, "cancelled", "pull_request", "old"),
        (3, "timed_out", "merge_group", "queue"),
        (10, "cancelled", "merge_group", "queue"),
        (2, "skipped", "pull_request", "abc"),
    ]);

    let of_head = failed_runs(runs.as_bytes(), "abc", false).unwrap();
    assert_eq!(ids(of_head), [8, 7, 5]);
    // The queue's cancelled runs are other groups' that GitHub stopped
    let of_queue = failed_runs(runs.as_bytes(), "abc", true).unwrap();
    assert_eq!(ids(of_queue), [3]);
}

#[test]
fn a_listing_gh_did_not_print_as_json_is_unreadable() {
    assert!(matches!(
        failed_runs(b"not json", "abc", false),
        Err(ForgeError::Unreadable(_))
    ));
}

// What `gh run view <id> --log-failed` prints of run `id`: a line the head's
// failure is named in
fn failed_log_of(id: u64) -> String {
    format!("test\tRun cargo test\t2026-10-13T10:00:00.0000000Z error: boom in run {id}\n")
}

#[test]
fn at_most_three_failed_logs_are_read() {
    let runs = listing(&[
        (50, "failure", "pull_request", "abc"),
        (40, "failure", "pull_request", "abc"),
        (30, "failure", "pull_request", "abc"),
        (20, "failure", "pull_request", "abc"),
        (10, "failure", "pull_request", "abc"),
    ]);
    // The script has no answer for a fourth log: a fourth read would panic
    let script = Script::answering(answered(&runs));
    for id in [50, 40, 30] {
        script.then(answered(&failed_log_of(id)));
    }

    let logs = actions_over(&script).logs("abc", false).unwrap();

    assert_eq!(logs.iter().map(|l| l.run).collect::<Vec<_>>(), [50, 40, 30]);
    assert_eq!(
        logs[0],
        FailedLog {
            check: "test".to_owned(),
            run: 50,
            url: "https://github.com/shep-pm/kelpie/actions/runs/50".to_owned(),
            tail: "test\tRun cargo test\terror: boom in run 50".to_owned(),
        }
    );
    let asked = script.asked();
    assert_eq!(asked.len(), 4);
    assert_eq!(
        asked[0],
        [
            "run",
            "list",
            "--repo",
            "shep-pm/kelpie",
            "--limit",
            "50",
            "--json",
            "databaseId,conclusion,headSha,event,workflowName,url"
        ]
    );
    assert_eq!(
        asked[1],
        [
            "run",
            "view",
            "50",
            "--repo",
            "shep-pm/kelpie",
            "--log-failed"
        ]
    );
}

#[test]
fn the_merge_queues_logs_are_those_of_its_runs() {
    let script = Script::answering(answered(&listed()));
    script.then(answered(&failed_log_of(4)));

    let logs = actions_over(&script).logs("abc", true).unwrap();

    assert_eq!(logs.iter().map(|l| l.run).collect::<Vec<_>>(), [4]);
}

#[test]
fn a_head_with_no_failed_run_has_no_logs() {
    let script = Script::answering(answered(&listing(&[(2, "success", "pull_request", "abc")])));

    assert_eq!(actions_over(&script).logs("abc", false), Ok(Vec::new()));
}

#[test]
fn the_rerun_asks_gh_to_rerun_that_run_alone() {
    let script = Script::answering(answered(""));

    actions_over(&script).rerun(42).unwrap();

    assert_eq!(
        script.asked(),
        [["run", "rerun", "42", "--repo", "shep-pm/kelpie"]]
    );
}

#[test]
fn a_rerun_gh_refuses_is_the_error_it_gave() {
    let script = Script::answering(super::tests::refused("run 42 cannot be rerun"));

    assert!(matches!(
        actions_over(&script).rerun(42),
        Err(ForgeError::Failed(_))
    ));
}

const STAMP: &str = "test\tBuild\t2026-10-13T10:00:00.0000000Z";

#[test]
fn a_failed_logs_tail_has_no_cargo_progress_lines() {
    let mut log = String::new();
    for n in 0..400 {
        log.push_str(&format!(
            "{STAMP}    Compiling crate{n} v0.1.0\n\
             {STAMP}   Downloaded dep{n} v1.0.0\n\
             {STAMP}  Downloading crates ...\n\
             {STAMP}     Checking crate{n} v0.1.0\n\
             {STAMP}        Fresh crate{n} v0.1.0\n"
        ));
    }
    log.push_str(&format!("{STAMP} error[E0308]: mismatched types\n"));
    log.push_str(&format!("{STAMP}  --> src/lib.rs:1:1\n"));

    let kept = failure_tail(&log, LOG_BYTES);

    assert_eq!(
        kept,
        "test\tBuild\terror[E0308]: mismatched types\ntest\tBuild\t --> src/lib.rs:1:1"
    );
}

#[test]
fn coloured_progress_lines_are_dropped_and_colour_is_stripped_from_the_rest() {
    let log = format!(
        "{STAMP} \x1b[1m\x1b[92m   Compiling\x1b[0m foo v0.1.0\n\
         {STAMP} \x1b[1m\x1b[91merror\x1b[0m: could not compile `foo`\n"
    );

    assert_eq!(
        failure_tail(&log, LOG_BYTES),
        "test\tBuild\terror: could not compile `foo`"
    );
}

#[test]
fn a_line_with_a_tab_in_its_text_loses_its_timestamp() {
    let log = format!("{STAMP} ---- a stdout ----\tx\n");

    assert_eq!(
        failure_tail(&log, LOG_BYTES),
        "test\tBuild\t---- a stdout ----\tx"
    );
}

#[test]
fn a_failure_line_that_starts_with_a_progress_word_stays() {
    let log = format!(
        "{STAMP} Checking invariants after merge: left != right\n\
         {STAMP} Checking formatting...\n\
         {STAMP}   Downloading v1 of the fixture failed\n"
    );

    let kept = failure_tail(&log, LOG_BYTES);

    assert_eq!(kept.lines().count(), 3, "{kept}");
}

#[test]
fn a_log_keeps_its_end_from_a_line_start_and_never_splits_a_character() {
    assert_eq!(tail("a\nb\n", 100), "a\nb");
    let log = "first line\nsecond line\nthird é line\n";
    assert_eq!(tail(log, 16), "third é line");
    assert_eq!(tail("é".repeat(10).as_str(), 5), "éé");
}

// A monorepo's test step runs every package at once: the failing package's
// block comes early and more than `LOG_BYTES` of a sibling passing follows it.
// The log is synthetic, written by hand in the shape of vitest output under
// `gh run view --log-failed`, since the real runs seen are not kept.
const INTERLEAVED: &str = include_str!("../../../tests/fixtures/gh-run-log-interleaved.txt");

#[test]
fn an_interleaved_monorepo_log_leads_with_the_failing_packages_test() {
    let kept = failure_tail(INTERLEAVED, LOG_BYTES);

    assert!(kept.starts_with("Failures:\n"), "{kept}");
    assert!(
        kept.contains("FAIL  src/a.test.ts > suite > case"),
        "{kept}"
    );
    assert!(kept.contains("Error: Test timed out in 5000ms."), "{kept}");
    let (failures, end) = kept.split_once("\nLog end:\n").unwrap();
    assert!(failures.contains("Error: Test timed out in 5000ms."));
    assert!(end.contains("feature109.test.ts"), "{end}");
    assert!(!end.contains("Error: Test timed out"), "{end}");
    assert!(kept.len() <= LOG_BYTES, "{}", kept.len());
}

#[test]
fn a_log_with_no_failure_lines_keeps_only_its_tail() {
    let line = "test\tBuild\t2026-10-13T10:00:00.0000000Z test a_test ... ok\n";
    let log = line.repeat(300);

    let kept = failure_tail(&log, LOG_BYTES);

    let cleaned = log.replace("2026-10-13T10:00:00.0000000Z ", "");
    assert_eq!(kept, tail(cleaned.trim_end(), LOG_BYTES));
    assert!(!kept.contains("Failures:"));
    assert!(!kept.contains("Log end:"));
}

#[test]
fn the_failure_blocks_stay_within_half_the_budget() {
    let mut log = String::new();
    for n in 0..100 {
        log.push_str(&format!("test\tBuild\t FAIL  src/f{n}.test.ts > case\n"));
        for m in 0..15 {
            log.push_str(&format!(
                "test\tBuild\tdetail {n} line {m} of the failure\n"
            ));
        }
    }
    log.push_str(&"test\tBuild\tpassing output of a sibling package\n".repeat(200));

    let kept = failure_tail(&log, LOG_BYTES);

    let (failures, end) = kept.split_once("\nLog end:\n").unwrap();
    let blocks = failures.strip_prefix("Failures:\n").unwrap();
    assert!(blocks.len() <= LOG_BYTES / 2, "{}", blocks.len());
    assert!(blocks.starts_with("test\tBuild\t FAIL  src/f0.test.ts"));
    assert!(end.contains("passing output of a sibling package"));
    assert!(kept.len() <= LOG_BYTES, "{}", kept.len());
}

#[test]
fn a_failure_line_that_repeats_is_collected_once() {
    let gap = "j\ts\tpassing\n".repeat(30);
    let log =
        format!("j\ts\t FAIL  a > b\nj\ts\tdetail\n{gap}j\ts\t FAIL  a > b\nj\ts\tlater\n{gap}")
            + &"j\ts\tpassing\n".repeat(600);

    let kept = failure_tail(&log, LOG_BYTES);

    let (failures, _) = kept.split_once("\nLog end:\n").unwrap();
    assert_eq!(failures.matches(" FAIL  a > b").count(), 1, "{failures}");
    assert!(failures.contains("j\ts\tdetail"));
    assert!(!failures.contains("later"));
}

#[test]
fn the_exit_code_line_is_no_failure_line() {
    let log = "j\ts\t##[error]Process completed with exit code 1.\n".to_owned()
        + &"j\ts\tpassing\n".repeat(600);

    let kept = failure_tail(&log, LOG_BYTES);

    assert!(!kept.contains("Failures:"), "{kept}");
}

#[test]
fn every_kind_of_failure_line_is_found() {
    for line in [
        " FAIL  a > b",
        "Error: boom",
        "error[E0061]: this function takes 2 arguments but 1 argument was supplied",
        "error: could not compile `foo` (lib) due to 1 previous error",
        "error: test failed, to rerun pass `--lib`",
        "AssertionError: left != right",
        "src/a.ts(1,2): error TS2322: no",
        "##[error]The operation was canceled.",
        "thread 'x' panicked at src/lib.rs:1:1:",
        "test result: FAILED. 0 passed; 1 failed",
        "---- worker::tests::a stdout ----",
    ] {
        let log = format!("j\ts\t{line}\n") + &"j\ts\tpassing\n".repeat(600);

        let kept = failure_tail(&log, LOG_BYTES);

        assert!(
            kept.starts_with(&format!("Failures:\nj\ts\t{line}")),
            "{line}: {kept}"
        );
    }
}

// Run 38275563600 of SK, read with gh 2.96: its `test-macos` job
// (`timeout-minutes: 20`) was stopped in `Run cargo test --locked`, and its
// `--log-failed` answer is empty. The log is the first 3 and last 60 lines, with
// the runner folder written `/home/me`.
const JOBS: &[u8] = include_bytes!("../../../tests/fixtures/gh-run-view-jobs-timed-out.json");
const JOB_LOG: &str = include_str!("../../../tests/fixtures/gh-run-log-timed-out.txt");

const RUN: u64 = 38_275_563_600;
const NOTE: &str = "note: job `test-macos` was cancelled in step `Run cargo test --locked` \
                    after 20m27s: its `timeout-minutes` ended it, so this is a slow job and \
                    not a failing test";

// What `gh` answered for run 38275563600 to `args`, `failed` being its `--log-failed`
fn answer(args: &[&str], failed: &str, job_log: &str) -> Result<Vec<u8>, ForgeError> {
    let id = RUN.to_string();
    let repo = "shep-pm/kelpie";
    let reply = match args {
        ["run", "view", run, "--repo", r, "--log-failed"] if (*run, *r) == (&id, repo) => failed,
        ["run", "view", run, "--json", "jobs", "--repo", r] if (*run, *r) == (&id, repo) => {
            return Ok(JOBS.to_vec());
        }
        [
            "run",
            "view",
            run,
            "--job",
            "114900834180",
            "--log",
            "--repo",
            r,
        ] if (*run, *r) == (&id, repo) => job_log,
        other => panic!("gh was asked {other:?}"),
    };
    Ok(reply.as_bytes().to_vec())
}

// Reads run 38275563600 as `gh` answers with `failed` and `job_log`, and returns
// what was read with each call `gh` was asked
fn read(failed: &str, job_log: &str) -> (RunLog, Vec<String>) {
    let mut asked = Vec::new();
    let read = read_run(&repo(), RUN, true, &mut |args: &[&str]| {
        asked.push(args.join(" "));
        answer(args, failed, job_log)
    })
    .unwrap();
    (read, asked)
}

#[test]
fn a_run_that_failed_keeps_reading_log_failed() {
    let failed = "test\tRun cargo test\t2026-10-13T10:00:00.0000000Z error: boom";

    let (read, asked) = read(failed, JOB_LOG);

    assert_eq!(read.text, "test\tRun cargo test\terror: boom");
    assert!(!read.text.starts_with("note:"));
    assert_eq!(
        asked,
        [format!("run view {RUN} --repo shep-pm/kelpie --log-failed")]
    );
}

#[test]
fn an_empty_failed_log_falls_back_to_the_cancelled_jobs_log() {
    let (read, asked) = read("", JOB_LOG);

    assert!(read.text.starts_with(NOTE));
    assert_eq!(
        asked,
        [
            format!("run view {RUN} --repo shep-pm/kelpie --log-failed"),
            format!("run view {RUN} --json jobs --repo shep-pm/kelpie"),
            format!("run view {RUN} --job 114900834180 --log --repo shep-pm/kelpie"),
        ]
    );
}

#[test]
fn a_timed_out_tail_names_the_job_the_step_and_its_elapsed_time() {
    let (read, _) = read("", JOB_LOG);

    let mut lines = read.text.lines();
    assert_eq!(lines.next(), Some(NOTE));
    let kept: Vec<&str> = lines.collect();
    assert_eq!(kept[0], "Failures:");
    assert!(kept.contains(&"Log end:"));
    assert!(
        (kept.iter())
            .filter(|l| !matches!(**l, "Failures:" | "Log end:"))
            .all(|l| l.starts_with("test-macos\tUNKNOWN STEP\t"))
    );
    assert!(
        kept.contains(&"test-macos\tUNKNOWN STEP\t##[error]The operation was canceled."),
        "{kept:?}"
    );
}

#[test]
fn a_timed_out_tail_is_cut_to_the_same_bytes_as_a_failed_one() {
    let line = "test-macos\tUNKNOWN STEP\t2026-10-12T16:33:08.5367760Z test é::a_test ... ok\n";
    let long = line.repeat(1000);

    let (read, _) = read("", &long);

    let kept = read.text.strip_prefix(&format!("{NOTE}\n")).unwrap();
    assert!(kept.len() <= LOG_BYTES, "{}", kept.len());
    assert_eq!(kept, failure_tail(&long, LOG_BYTES));
    assert!(kept.starts_with("test-macos\tUNKNOWN STEP\ttest é::a_test"));
}

#[test]
fn a_run_cancelled_in_no_job_has_no_log_to_read() {
    let no_jobs = br#"{"jobs":[{"databaseId":1,"name":"lint","conclusion":"success","startedAt":"2026-10-12T16:12:47Z","completedAt":"2026-10-12T16:13:44Z","steps":[]}]}"#;

    let read = read_run(&repo(), 5, true, &mut |args: &[&str]| match args {
        [.., "--log-failed"] => Ok(Vec::new()),
        _ => Ok(no_jobs.to_vec()),
    })
    .unwrap();

    assert_eq!(read.text, "");
    assert!(!read.text.starts_with("note:"));
}

#[test]
fn a_run_that_failed_is_never_asked_for_its_jobs() {
    let mut asked = 0;

    let read = read_run(&repo(), 5, false, &mut |_: &[&str]| {
        asked += 1;
        Ok(Vec::new())
    })
    .unwrap();

    assert_eq!(read.text, "");
    assert_eq!(asked, 1);
}

#[test]
fn a_job_listing_gh_cannot_answer_leaves_the_log_empty() {
    let read = read_run(&repo(), 5, true, &mut |args: &[&str]| match args {
        [.., "--log-failed"] => Ok(Vec::new()),
        _ => Err(ForgeError::Failed("rate limited".to_owned())),
    })
    .unwrap();

    assert_eq!(read.text, "");
    assert!(!read.text.starts_with("note:"));
}
