//! A commit's checks read into a head's status, from recorded `gh` output
//!
//! No test needs `gh`, a login or the network. `HEAD_CHECKS` is the answer to
//! the query of `checks::QUERY` for a head whose `test.yml` GitHub could not
//! read (SK's recording of it, in this query's shape), and the other answers are
//! built around it.

use super::checks::parse_head;
use crate::ci::{FailedCheck, HeadStatus, Queued};
use crate::ticket::ForgeError;

const HEAD_CHECKS: &str = include_str!("../../../tests/fixtures/gh-graphql-head-checks.json");

const GREEN: &str =
    r#"{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"SUCCESS"}"#;
const SKIPPED: &str =
    r#"{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SKIPPED"}"#;

// The answer for a commit whose rollup holds `contexts`, beside `suites`
fn reply(contexts: &str, suites: &str) -> Vec<u8> {
    format!(
        r#"{{"data":{{"repository":{{"object":{{
            "statusCheckRollup":{{"contexts":{{"nodes":[{contexts}]}}}},
            "checkSuites":{{"nodes":[{suites}]}}}}}}}}}}"#
    )
    .into_bytes()
}

// The status of a commit whose rollup holds `contexts` and has no unusual suite
fn rollup(contexts: &str) -> HeadStatus {
    parse_head(&reply(contexts, "")).unwrap()
}

fn failed(name: &str, url: Option<&str>) -> FailedCheck {
    FailedCheck {
        name: name.to_owned(),
        url: url.map(str::to_owned),
    }
}

const UNSTARTED: &str = ".github/workflows/test.yml: started no job";
const UNSTARTED_URL: &str = "https://github.com/shep-pm/shep-kelpie/actions/runs/38900000003";

// The recorded suites with the rollup's contexts swapped for `contexts`
fn recorded_over(contexts: &str) -> HeadStatus {
    let recorded: serde_json::Value = serde_json::from_str(HEAD_CHECKS).unwrap();
    let suites = &recorded["data"]["repository"]["object"]["checkSuites"]["nodes"];
    let suites = serde_json::to_string(suites).unwrap();
    let suites = suites.trim_start_matches('[').trim_end_matches(']');
    parse_head(&reply(contexts, suites)).unwrap()
}

#[test]
fn a_workflow_that_started_no_job_turns_a_green_rollup_red() {
    let red = HeadStatus::Failed(vec![failed(UNSTARTED, Some(UNSTARTED_URL))]);

    // The recording's own rollup is green
    assert_eq!(parse_head(HEAD_CHECKS.as_bytes()).unwrap(), red);
    assert_eq!(recorded_over(GREEN), red);
    assert_eq!(recorded_over(SKIPPED), red);
    assert_eq!(recorded_over(""), red);
}

#[test]
fn a_suite_held_for_approval_keeps_the_rollup_pending() {
    let held = r#"{"conclusion":"ACTION_REQUIRED","checkRuns":{"totalCount":0},
        "workflowRun":{"url":"https://github.com/shep-pm/shep-kelpie/actions/runs/42",
        "file":{"path":".github/workflows/test.yml"}}}"#;
    let red =
        r#"{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE"}"#;
    for contexts in [SKIPPED, GREEN, "", red] {
        let HeadStatus::Pending(queued) = parse_head(&reply(contexts, held)).unwrap() else {
            panic!("a held suite must keep the rollup pending");
        };
        assert_eq!(
            queued,
            [Queued {
                name: ".github/workflows/test.yml".to_owned(),
                run: Some(42),
                running: false,
                held: true,
            }]
        );
    }
}

#[test]
fn a_commit_github_does_not_know_has_no_checks() {
    let gone = br#"{"data":{"repository":{"object":null}}}"#;

    assert_eq!(parse_head(gone).unwrap(), HeadStatus::None);
}

#[test]
fn an_answer_in_another_shape_is_unreadable() {
    for stdout in [&br#"{"data":{}}"#[..], b"not json", b""] {
        assert!(
            matches!(parse_head(stdout), Err(ForgeError::Unreadable(_))),
            "{}",
            String::from_utf8_lossy(stdout)
        );
    }
}

#[test]
fn no_checks_is_not_green() {
    assert_eq!(rollup(""), HeadStatus::None);
}

#[test]
fn a_commit_with_no_rollup_has_no_checks() {
    let bare = br#"{"data":{"repository":{"object":{"statusCheckRollup":null,"checkSuites":{"nodes":[]}}}}}"#;

    assert_eq!(parse_head(bare).unwrap(), HeadStatus::None);
}

#[test]
fn a_red_check_names_itself_and_its_link() {
    let lint = r#"{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE",
        "detailsUrl":"https://github.com/shep-pm/kelpie/actions/runs/7/job/8"}"#;
    let status = r#"{"__typename":"StatusContext","context":"ci/other","state":"FAILURE",
        "targetUrl":"https://ci.example.com/jobs/4"}"#;

    assert_eq!(
        rollup(&format!("{GREEN},{lint},{status}")),
        HeadStatus::Failed(vec![
            failed(
                "lint",
                Some("https://github.com/shep-pm/kelpie/actions/runs/7/job/8")
            ),
            failed("ci/other", Some("https://ci.example.com/jobs/4")),
        ])
    );
}

#[test]
fn skipped_runs_pass_beside_a_run_that_ran_and_are_skipped_alone() {
    assert_eq!(rollup(&format!("{SKIPPED},{SKIPPED}")), HeadStatus::Skipped);
    assert_eq!(rollup(&format!("{SKIPPED},{GREEN}")), HeadStatus::Passed);
}

#[test]
fn a_running_check_holds_the_verdict_even_beside_a_failure() {
    let red =
        r#"{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE"}"#;
    for running in [
        r#"{"__typename":"CheckRun","name":"test","status":"IN_PROGRESS","conclusion":""}"#,
        r#"{"__typename":"CheckRun","name":"test","status":"QUEUED","conclusion":null}"#,
        r#"{"__typename":"StatusContext","context":"CodeRabbit","state":"PENDING"}"#,
    ] {
        assert!(matches!(
            rollup(&format!("{red},{running}")),
            HeadStatus::Pending(_)
        ));
    }
}

#[test]
fn a_cancelled_run_and_an_errored_status_fail() {
    let contexts = r#"
        {"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"CANCELLED"},
        {"__typename":"StatusContext","context":"ci/other","state":"ERROR"}"#;

    assert_eq!(
        rollup(contexts),
        HeadStatus::Failed(vec![failed("test", None), failed("ci/other", None)])
    );
}

#[test]
fn a_queued_check_run_is_read_with_its_workflow_run() {
    let queued = r#"{"__typename":"CheckRun","name":"windows-gnu","status":"QUEUED","conclusion":"",
        "detailsUrl":"https://github.com/shep-pm/shep/actions/runs/39697676985/job/1"}"#;
    // Still running, and listed as the gate cannot tell it from a stuck one
    let running = r#"{"__typename":"CheckRun","name":"docs","status":"IN_PROGRESS","conclusion":"",
        "completedAt":"0001-01-01T00:00:00Z",
        "detailsUrl":"https://github.com/shep-pm/shep/actions/runs/39697679104/job/2"}"#;

    assert_eq!(
        rollup(&format!("{queued},{running}")),
        HeadStatus::Pending(vec![
            Queued {
                name: "windows-gnu".to_owned(),
                run: Some(39_697_676_985),
                running: false,
                held: false,
            },
            Queued {
                name: "docs".to_owned(),
                run: Some(39_697_679_104),
                running: true,
                held: false,
            },
        ])
    );
}

#[test]
fn a_queued_check_run_whose_link_names_no_run_has_none() {
    let deploy = r#"{"__typename":"CheckRun","name":"deploy","status":"QUEUED","conclusion":"","detailsUrl":"https://ci.example.com/jobs/4"}"#;
    let build = r#"{"__typename":"CheckRun","name":"build","status":"QUEUED","conclusion":""}"#;

    let HeadStatus::Pending(queued) = rollup(&format!("{deploy},{build}")) else {
        panic!("a queued check is pending");
    };
    assert_eq!(
        queued.iter().map(|q| q.run).collect::<Vec<_>>(),
        [None, None]
    );
}

#[test]
fn a_finished_check_run_is_read_by_its_conclusion_whatever_its_status() {
    let red = r#"{"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":"FAILURE","completedAt":"2026-10-23T05:56:37Z"}"#;
    let skipped = r#"{"__typename":"CheckRun","name":"test","status":"QUEUED","conclusion":"SKIPPED","completedAt":"2026-10-23T05:56:37Z"}"#;
    let done = r#"{"__typename":"CheckRun","name":"typos","status":"IN_PROGRESS","conclusion":"SUCCESS","completedAt":"2026-10-23T05:56:37Z"}"#;

    assert_eq!(rollup(red), HeadStatus::Failed(vec![failed("lint", None)]));
    assert_eq!(rollup(skipped), HeadStatus::Skipped);
    assert_eq!(rollup(done), HeadStatus::Passed);
}

#[test]
fn a_running_check_run_is_pending_whatever_its_other_fields_say() {
    // GitHub fills an unfinished run's `completedAt` with the zero time
    for fields in [
        r#""completedAt":"0001-01-01T00:00:00Z","conclusion":"""#,
        r#""completedAt":"0001-01-01T00:00:00Z","conclusion":"SUCCESS""#,
        r#""completedAt":"2026-10-23T05:56:37Z","conclusion":"""#,
        r#""conclusion":"SUCCESS""#,
        r#""conclusion":null"#,
    ] {
        let run = format!(
            r#"{{"__typename":"CheckRun","name":"typos","status":"IN_PROGRESS",{fields}}}"#
        );
        assert!(matches!(rollup(&run), HeadStatus::Pending(_)), "{run}");
    }
}

// A check run of workflow `workflow`, named `name`, started at `started`. An
// empty `conclusion` is a run still going.
fn a_run(name: &str, workflow: &str, started: &str, conclusion: &str) -> String {
    let (status, completed_at) = match conclusion {
        "" => ("IN_PROGRESS", "0001-01-01T00:00:00Z"),
        _ => ("COMPLETED", started),
    };
    format!(
        concat!(
            r#"{{"__typename":"CheckRun","name":"{name}","status":"{status}","#,
            r#""conclusion":"{conclusion}","completedAt":"{completed_at}","#,
            r#""startedAt":"{started}","#,
            r#""checkSuite":{{"workflowRun":{{"workflow":{{"name":"{workflow}"}}}}}},"#,
            r#""detailsUrl":"https://github.com/o/r/actions/runs/2/job/2"}}"#,
        ),
        name = name,
        status = status,
        conclusion = conclusion,
        completed_at = completed_at,
        workflow = workflow,
        started = started,
    )
}

#[test]
fn a_cancelled_run_a_later_run_of_its_check_passed_is_dropped() {
    let cancelled = a_run("test", "test", "2026-11-07T20:25:54Z", "CANCELLED");
    let passed = a_run("test", "test", "2026-11-07T20:46:52Z", "SUCCESS");

    assert_eq!(rollup(&format!("{cancelled},{passed}")), HeadStatus::Passed);
}

#[test]
fn a_cancelled_run_with_a_later_run_still_going_is_pending() {
    let cancelled = a_run("test", "test", "2026-11-07T20:25:54Z", "CANCELLED");
    let running = a_run("test", "test", "2026-11-07T20:46:52Z", "");

    assert_eq!(
        rollup(&format!("{cancelled},{running}")),
        HeadStatus::Pending(vec![Queued {
            name: "test".to_owned(),
            run: Some(2),
            running: true,
            held: false,
        }])
    );
}

#[test]
fn a_cancelled_run_newer_than_a_pass_still_fails() {
    let passed = a_run("test", "test", "2026-11-07T20:25:54Z", "SUCCESS");
    let cancelled = a_run("test", "test", "2026-11-07T20:46:52Z", "CANCELLED");

    assert!(matches!(
        rollup(&format!("{passed},{cancelled}")),
        HeadStatus::Failed(_)
    ));
}

#[test]
fn a_cancelled_run_of_another_workflow_still_fails() {
    let cancelled = a_run("test", "test", "2026-11-07T20:25:54Z", "CANCELLED");
    let passed = a_run("test", "release", "2026-11-07T20:46:52Z", "SUCCESS");

    assert!(matches!(
        rollup(&format!("{cancelled},{passed}")),
        HeadStatus::Failed(_)
    ));
}

#[test]
fn a_cancelled_run_in_a_rollup_naming_no_workflow_keys_on_the_name() {
    let cancelled = r#"{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"CANCELLED","startedAt":"2026-11-07T20:25:54Z","completedAt":"2026-11-07T20:30:00Z"}"#;
    let passed = r#"{"__typename":"CheckRun","name":"test","status":"COMPLETED","conclusion":"SUCCESS","startedAt":"2026-11-07T20:46:52Z","completedAt":"2026-11-07T20:52:00Z"}"#;

    assert_eq!(rollup(&format!("{cancelled},{passed}")), HeadStatus::Passed);
}

#[test]
fn a_red_rollup_names_the_workflow_that_started_no_job_too() {
    let red =
        r#"{"__typename":"CheckRun","name":"lint","status":"COMPLETED","conclusion":"FAILURE"}"#;
    let unstarted = r#"{"conclusion":"FAILURE","checkRuns":{"totalCount":0},
        "workflowRun":{"file":{"path":".github/workflows/test.yml"}}}"#;

    assert_eq!(
        parse_head(&reply(red, unstarted)).unwrap(),
        HeadStatus::Failed(vec![failed("lint", None), failed(UNSTARTED, None)])
    );
}

#[test]
fn a_pending_rollup_stays_pending_beside_a_workflow_that_started_no_job() {
    let running =
        r#"{"__typename":"CheckRun","name":"docs","status":"IN_PROGRESS","conclusion":""}"#;
    let unstarted = r#"{"conclusion":"FAILURE","checkRuns":{"totalCount":0},
        "workflowRun":{"file":{"path":".github/workflows/test.yml"}}}"#;

    assert!(matches!(
        parse_head(&reply(running, unstarted)).unwrap(),
        HeadStatus::Pending(_)
    ));
}

#[test]
fn suites_with_check_runs_or_from_another_app_change_nothing() {
    let suites = r#"
        {"conclusion":"FAILURE","checkRuns":{"totalCount":0},"workflowRun":null},
        {"conclusion":"FAILURE","checkRuns":{"totalCount":3},"workflowRun":{"file":{"path":"a.yml"}}},
        {"conclusion":"SUCCESS","checkRuns":{"totalCount":0},"workflowRun":{"file":{"path":"b.yml"}}}"#;

    assert_eq!(
        parse_head(&reply(GREEN, suites)).unwrap(),
        HeadStatus::Passed
    );
}

#[test]
fn the_query_asks_for_a_head_by_its_hash_and_both_the_rollup_and_the_suites() {
    use crate::github::ForgeSlug;
    use crate::github::gh::hold::Hold;
    use crate::github::gh::tests::{Script, answered, gh_over};

    let script = Script::answering(answered(HEAD_CHECKS));
    let repo = ForgeSlug::try_from("shep-pm/kelpie".to_owned()).unwrap();

    super::checks::head_status(&Hold::new(gh_over(&script)), &repo, "abc123").unwrap();

    let asked = script.asked();
    assert_eq!(asked.len(), 1);
    let call = &asked[0];
    assert_eq!(call[..3], ["api", "graphql", "-f"]);
    assert!(call[3].contains("statusCheckRollup"), "{}", call[3]);
    assert!(call[3].contains("checkSuites"), "{}", call[3]);
    assert_eq!(
        call[4..],
        [
            "-f",
            "owner=shep-pm",
            "-f",
            "name=kelpie",
            "-f",
            "oid=abc123"
        ]
    );
}
