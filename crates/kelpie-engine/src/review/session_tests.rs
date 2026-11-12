//! A read on the fake ACP agent, so no test starts a harness or a model

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::json;

use super::reviewer_prompt;
use super::session::{Opening, ReviewProject, SessionReviewer, diff_against, write_tool};
use kelpie_protocol::ToolStatus;

use super::{Finding, Read, Reviewer, ReviewerError, Severity};
use crate::agents::{Binding, Origin, Sourced};
use crate::run::RunId;
use crate::session::fake::{FakeAgent, Turn};
use crate::session::{Session, SessionEvent};
use crate::settings::Harness;
use crate::worker::Paths;
use crate::worktree::tests::{Dir, git, process_tree, write_in};

/// A data folder of its own for one test, removed when it ends
struct Rig {
    root: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("kelpie-review-{}", RunId::new().unwrap()));
        let worktree = root.join("data/projects/web/worktrees/7");
        fs::create_dir_all(&worktree).unwrap();
        Self { root }
    }

    fn paths(&self) -> Paths {
        Paths::new(self.root.join("data"), "web")
    }

    fn worktree(&self) -> PathBuf {
        self.paths().worktree("7")
    }

    fn tests(&self) -> PathBuf {
        self.paths().review_state_dir("7").join("tests")
    }

    /// The reviewer of ticket 7 whose reads open sessions on `agent`
    fn reviewer(&self, agent: &FakeAgent, binding: Binding) -> SessionReviewer {
        SessionReviewer::new(
            self.paths(),
            "7",
            binding,
            ReviewProject {
                folder: self.root.join("project"),
                git_common_dir: self.root.join("project/.git"),
                network_allowlist: Vec::new(),
                reads: crate::sandbox::Reads::Project,
            },
            Box::new(on(agent)),
        )
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// An opener whose every session talks to `agent`, as the app's talks to a harness
fn on(agent: &FakeAgent) -> impl Fn(&Opening<'_>) -> Result<Session, String> + Send + Sync + use<> {
    let agent = agent.clone();
    move |opening| Ok(agent.session(opening.worktree.to_owned(), None, opening.choice.clone()))
}

fn binding(harness: Option<Harness>, model: Option<&str>, effort: Option<&str>) -> Binding {
    Binding {
        harness: Sourced {
            value: harness,
            from: Origin::Project,
        },
        model: Sourced {
            value: model.map(str::to_owned),
            from: Origin::Project,
        },
        effort: Sourced {
            value: effort.map(str::to_owned),
            from: Origin::Project,
        },
        body: Sourced {
            value: None,
            from: Origin::BuiltIn,
        },
    }
}

/// One read of the rig's worktree, and what its sink heard
fn read(
    reviewer: &SessionReviewer,
    worktree: &Path,
    tests: Option<&Path>,
    prompt: &str,
) -> (Result<Vec<Finding>, ReviewerError>, Vec<SessionEvent>) {
    let heard = Mutex::new(Vec::new());
    let sink = |event| heard.lock().unwrap().push(event);
    let answer = reviewer.read(&Read {
        prompt,
        worktree,
        tests,
        sink: &sink,
    });
    (answer, heard.into_inner().unwrap())
}

#[test]
fn each_read_is_a_fresh_session_never_the_workers() {
    let rig = Rig::new();
    let agent = FakeAgent::new()
        .can_resume()
        .can_load()
        .turn(Turn::new().text("CLEAN"))
        .turn(Turn::new().text("CLEAN"));
    let reviewer = rig.reviewer(&agent, binding(None, None, None));

    let (first, _) = read(&reviewer, &rig.worktree(), None, "read it");
    let (second, _) = read(&reviewer, &rig.worktree(), None, "read it again");

    assert_eq!(first, Ok(vec![]));
    assert_eq!(second, Ok(vec![]));
    assert_eq!(agent.launches(), 2, "each read starts its own harness");
    let methods = agent.methods();
    assert_eq!(
        methods.iter().filter(|m| *m == "session/new").count(),
        2,
        "{methods:?}"
    );
    assert!(
        !methods
            .iter()
            .any(|m| m == "session/load" || m == "session/resume"),
        "a read never takes up a session that was kept: {methods:?}"
    );
    // The two reads were told only what they were sent: nothing of an earlier turn
    let prompts = agent.params("session/prompt");
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].to_string().contains("read it again"));
}

#[test]
fn a_reply_ending_clean_is_no_findings_and_prose_alone_is_unreadable() {
    let rig = Rig::new();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("I found nothing wrong.\nCLEAN\n"))
        .turn(Turn::new().text("HIGH|src/lib.rs:4|it panics|a caller sees it\n"))
        .turn(Turn::new().text("Looks fine to me."));
    let reviewer = rig.reviewer(&agent, binding(None, None, None));
    let ask = |text| read(&reviewer, &rig.worktree(), None, text).0;

    assert_eq!(ask("one"), Ok(vec![]));
    assert_eq!(
        ask("two"),
        Ok(vec![Finding::new(
            Severity::High,
            "src/lib.rs",
            4,
            "it panics",
            "a caller sees it"
        )])
    );
    assert_eq!(
        ask("three"),
        Err(ReviewerError::Unreadable("Looks fine to me.".to_owned()))
    );
}

#[test]
fn a_read_runs_on_the_reviewers_binding_with_the_reviewer_rows_over_it() {
    let rig = Rig::new();
    let agent = FakeAgent::new()
        .offers_models("default", &["default", "project-model"])
        .offers_efforts("medium", &["medium", "low"])
        .turn(Turn::new().text("CLEAN"));
    // The binding the settings and the agent file give: the project's model and
    // the file's effort
    let reviewer = rig.reviewer(
        &agent,
        binding(
            Some(Harness::ClaudeCode),
            Some("project-model"),
            Some("low"),
        ),
    );

    let (answer, _) = read(&reviewer, &rig.worktree(), None, "read it");

    assert_eq!(answer, Ok(vec![]));
    let set = agent.params("session/set_config_option");
    assert_eq!(
        set,
        [
            json!({"sessionId": "fake-1", "configId": "model", "value": "project-model"}),
            json!({"sessionId": "fake-1", "configId": "effort", "value": "low"}),
        ]
    );
}

/// The status each tool step of `heard` ended on, by its id
fn steps(heard: &[SessionEvent]) -> Vec<(String, ToolStatus, Option<String>)> {
    heard
        .iter()
        .filter_map(|event| match event {
            SessionEvent::ToolStep {
                id, status, reason, ..
            } if *status != ToolStatus::Pending => Some((id.clone(), *status, reason.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn a_shell_call_from_a_reviewer_is_refused() {
    let rig = Rig::new();
    let agent = FakeAgent::new().turn(
        Turn::new()
            .permission_to_run("t1", "Run cargo test", "cargo test")
            .permission_to_run("t2", "Run git status", "git status")
            .text("CLEAN"),
    );
    // With a test command too: the tests folder lets it write, never run
    let reviewer = rig.reviewer(&agent, binding(None, None, None));
    let tests = rig.tests();

    let (answer, heard) = read(&reviewer, &rig.worktree(), Some(&tests), "read it");

    assert_eq!(answer, Ok(vec![]));
    let steps = steps(&heard);
    assert_eq!(steps.len(), 2, "{heard:?}");
    for (id, status, reason) in steps {
        assert_eq!(status, ToolStatus::Refused, "{id}");
        assert!(reason.is_some_and(|why| why.contains("guard")), "{id}");
    }
    assert_eq!(
        agent.params("session/request_permission"),
        vec![json!({"outcome": {"outcome": "selected", "optionId": "reject"}}); 2]
    );
}

#[test]
fn a_write_outside_the_tests_folder_is_refused() {
    let rig = Rig::new();
    let tests = rig.tests();
    let inside = tests.join("tests/flag.rs");
    let agent = FakeAgent::new().turn(
        Turn::new()
            .permission_to_write("in", "Write a test", inside.to_str().unwrap())
            .permission_to_write("src", "Write src/lib.rs", "src/lib.rs")
            .permission_to_write(
                "worktree",
                "Write the worktree",
                rig.worktree().join("tests/flag.rs").to_str().unwrap(),
            )
            .permission_to_write(
                "state",
                "Write the state folder",
                rig.paths()
                    .review_state_dir("7")
                    .join("note")
                    .to_str()
                    .unwrap(),
            )
            .text("CLEAN"),
    );
    let reviewer = rig.reviewer(&agent, binding(None, None, None));

    let (answer, heard) = read(&reviewer, &rig.worktree(), Some(&tests), "read it");

    assert_eq!(answer, Ok(vec![]));
    let steps: Vec<_> = steps(&heard)
        .into_iter()
        .map(|(id, status, _)| (id, status))
        .collect();
    assert_eq!(
        steps,
        [
            ("in".to_owned(), ToolStatus::Done),
            ("src".to_owned(), ToolStatus::Refused),
            ("worktree".to_owned(), ToolStatus::Refused),
            ("state".to_owned(), ToolStatus::Refused),
        ]
    );
}

#[test]
fn with_no_test_command_every_write_is_refused() {
    let rig = Rig::new();
    let agent = FakeAgent::new().turn(
        Turn::new()
            .permission_to_write(
                "tests",
                "Write a test",
                rig.tests().join("tests/flag.rs").to_str().unwrap(),
            )
            .permission_to_write("src", "Write src/lib.rs", "src/lib.rs")
            .text("CLEAN"),
    );
    let reviewer = rig.reviewer(&agent, binding(None, None, None));

    let (answer, heard) = read(&reviewer, &rig.worktree(), None, "read it");

    assert_eq!(answer, Ok(vec![]));
    let statuses: Vec<_> = steps(&heard)
        .into_iter()
        .map(|(_, status, _)| status)
        .collect();
    assert_eq!(statuses, [ToolStatus::Refused, ToolStatus::Refused]);
}

#[test]
fn a_second_look_is_another_fresh_session_shown_the_first_list() {
    let rig = Rig::new();
    let first = Finding::new(Severity::Medium, "src/a.rs", 9, "it loops", "a hang");
    let agent = FakeAgent::new()
        .turn(Turn::new().text("MEDIUM|src/a.rs:9|it loops|a hang\n"))
        .turn(Turn::new().text("CLEAN"));
    let reviewer = rig.reviewer(&agent, binding(None, None, None));
    let body = "Review {{DIFF}}";
    let diff = "+fn a() {}";
    let ask = |first: Option<&[Finding]>| {
        let prompt = reviewer_prompt(body, "abc123", diff, "", first, None);
        read(&reviewer, &rig.worktree(), None, &prompt).0
    };

    let found = ask(None).unwrap();
    assert_eq!(found, std::slice::from_ref(&first));
    let second = ask(Some(&found));

    assert_eq!(second, Ok(vec![]));
    assert_eq!(
        agent.launches(),
        2,
        "the second look is a session of its own"
    );
    let prompts = agent.params("session/prompt");
    let (one, two) = (prompts[0].to_string(), prompts[1].to_string());
    assert!(!one.contains("MEDIUM|src/a.rs:9"), "{one}");
    assert!(two.contains("MEDIUM|src/a.rs:9|it loops|a hang"), "{two}");
    assert!(two.contains("first reader's findings"), "{two}");
}

#[test]
fn a_session_cut_off_or_stopped_is_not_a_read() {
    let rig = Rig::new();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("HIGH|a.rs:1|x|y\n").stop_at_token_limit())
        .turn(Turn::new().text("working").stop_cancelled())
        .turn(Turn::new().text("working").exit());
    let reviewer = rig.reviewer(&agent, binding(None, None, None));
    let ask = |text| read(&reviewer, &rig.worktree(), None, text).0;

    // What it said before the limit is no review: the read was cut short
    assert!(matches!(ask("one"), Err(ReviewerError::CutOff(_))));
    assert_eq!(ask("two"), Err(ReviewerError::Stopped));
    assert!(matches!(ask("three"), Err(ReviewerError::Failed(_))));
}

#[test]
fn a_read_keeps_its_own_log_and_streams_its_events_to_the_sink() {
    let rig = Rig::new();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("CLEAN"))
        .turn(Turn::new().text("CLEAN"));
    let reviewer = rig.reviewer(&agent, binding(None, None, None));

    let (_, heard) = read(&reviewer, &rig.worktree(), None, "one");
    let _ = read(&reviewer, &rig.worktree(), None, "two");

    assert!(
        matches!(heard.first(), Some(SessionEvent::Started { .. })),
        "{heard:?}"
    );
    assert!(
        matches!(heard.last(), Some(SessionEvent::TurnEnded { .. })),
        "{heard:?}"
    );
    let log = |round| crate::session::log::read(rig.paths().review_log_file("7", round)).unwrap();
    assert_eq!(log(1).len(), heard.len() + 1, "the prompt, then each event");
    assert_eq!(log(2).len(), log(1).len());
    assert!(!rig.paths().review_log_file("7", 3).exists());
}

#[test]
fn a_session_that_cannot_open_is_a_read_that_cannot_start() {
    let rig = Rig::new();
    let reviewer = SessionReviewer::new(
        rig.paths(),
        "7",
        binding(None, None, None),
        ReviewProject {
            folder: rig.root.join("project"),
            git_common_dir: rig.root.join("project/.git"),
            network_allowlist: Vec::new(),
            reads: crate::sandbox::Reads::Project,
        },
        Box::new(|_| Err("no bridge for it".to_owned())),
    );

    let (answer, heard) = read(&reviewer, &rig.worktree(), None, "one");

    assert_eq!(
        answer,
        Err(ReviewerError::Start("no bridge for it".to_owned()))
    );
    assert!(heard.is_empty());
}

#[test]
fn the_write_tool_comes_from_the_harness() {
    assert_eq!(write_tool(Some(Harness::ClaudeCode)), "Write");
    assert_eq!(write_tool(Some(Harness::Goose)), "text_editor");
}

#[test]
fn the_diff_never_follows_a_worktree_git_file_pointed_elsewhere() {
    let dir = Dir::new("review-diff");
    let repo = dir.path().join("repo");
    git(dir.path(), &["init", "--quiet", "-b", "main", "repo"]);
    write_in(&repo, "lib.rs", "fn one() {}\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "first"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let worktree = dir.path().join("work");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "kelpie/7",
            worktree.to_str().unwrap(),
        ],
    );
    write_in(&worktree, "lib.rs", "fn one() {}\nfn two() {}\n");
    git(&worktree, &["commit", "--quiet", "-am", "the change"]);
    let tree = process_tree(dir.path());

    let diff = diff_against(&tree, &repo, &worktree, &base).unwrap();
    assert!(diff.contains("+fn two() {}"), "{diff}");

    // A repo of the worker's own, with another change, where the worktree's `.git` now points
    let planted = dir.path().join("planted");
    git(dir.path(), &["init", "--quiet", "-b", "main", "planted"]);
    write_in(&planted, "lib.rs", "fn planted() {}\n");
    git(&planted, &["add", "."]);
    git(&planted, &["commit", "--quiet", "-m", "planted"]);
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", planted.join(".git").display()),
    )
    .unwrap();
    assert_eq!(
        git(&worktree, &["log", "-1", "--format=%s"]),
        "planted",
        "a git that finds the worktree's own dirs reads the planted repo"
    );

    let again = diff_against(&tree, &repo, &worktree, &base).unwrap();
    assert_eq!(
        again, diff,
        "kelpie reads the clone's own dirs, never the planted ones"
    );
}

#[test]
fn the_diff_runs_no_external_driver_and_no_text_conversion() {
    let dir = Dir::new("review-diff-driver");
    let repo = dir.path().join("repo");
    git(dir.path(), &["init", "--quiet", "-b", "main", "repo"]);
    write_in(&repo, ".gitattributes", "*.txt diff=loud\n");
    write_in(&repo, "a.txt", "one\n");
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "first"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let worktree = dir.path().join("work");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "kelpie/7",
            worktree.to_str().unwrap(),
        ],
    );
    write_in(&worktree, "a.txt", "two\n");
    git(&worktree, &["commit", "--quiet", "-am", "the change"]);
    // The checkout's own config names a driver and a conversion that would run a command
    let marker = dir.path().join("ran");
    git(
        &repo,
        &[
            "config",
            "diff.loud.command",
            &format!("touch {}; true", marker.display()),
        ],
    );
    git(
        &repo,
        &[
            "config",
            "diff.loud.textconv",
            &format!("touch {}; cat", marker.display()),
        ],
    );
    let tree = process_tree(dir.path());

    let diff = diff_against(&tree, &repo, &worktree, &base).unwrap();

    assert!(diff.contains("+two"), "{diff}");
    assert!(!marker.exists(), "a driver of the repo's config ran");
}
