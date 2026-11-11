//! The gate's order, each step on plain values

use std::num::NonZeroU64;

use super::*;
use crate::ci::{FailedLog, HeadStatus, Queued};
use crate::settings::{CiWait, FixAttempts};
use crate::ticket::TicketId;

const HEAD: &str = "aaaaaaa1111111111111111111111111111111aa";
const OTHER: &str = "bbbbbbb2222222222222222222222222222222bb";

fn at(secs: u64) -> Timestamp {
    Timestamp::from_secs(secs)
}

fn limits() -> Limits {
    Limits {
        ci_holds_merge: true,
        fix_attempts: None,
        ci_wait: serde_json::from_value::<CiWait>(10.into()).unwrap(),
    }
}

fn pull(head: &str, checks: HeadStatus) -> OpenPull {
    OpenPull {
        number: NonZeroU64::new(71).unwrap(),
        branch: "kelpie/7".to_owned(),
        draft: true,
        closes: vec![],
        head: head.to_owned(),
        base: "main".to_owned(),
        labels: vec![],
        checks,
    }
}

fn look(checks: HeadStatus) -> Look {
    Look {
        pull: Pull::Open(pull(HEAD, checks)),
        base: Base::Current,
        head: HEAD.to_owned(),
        agents_files: vec![],
        base_runs_pull_workflow: true,
    }
}

// A kept ticket whose wait on `HEAD` began at `since`, with the labels and draft state seen
fn waiting(since: u64) -> Kept {
    Kept {
        seen: Some(Seen {
            labels: vec![],
            draft: true,
        }),
        wait: Some(Wait {
            head: HEAD.to_owned(),
            since: at(since),
        }),
        ..Kept::default()
    }
}

fn queued(name: &str, run: Option<u64>) -> Queued {
    Queued {
        name: name.to_owned(),
        run,
        running: false,
        held: false,
    }
}

fn failed(name: &str) -> FailedCheck {
    FailedCheck {
        name: name.to_owned(),
        url: Some(format!("https://github.com/o/r/actions/runs/9/job/{name}")),
    }
}

fn log(check: &str, run: u64, tail: &str) -> FailedLog {
    FailedLog {
        check: check.to_owned(),
        run,
        url: format!("https://github.com/o/r/actions/runs/{run}"),
        tail: tail.to_owned(),
    }
}

fn ruled(step: &Step) -> &Ruling {
    match step {
        Step::Rule(ruling) => ruling,
        other => panic!("expected a ruling, got {other:?}"),
    }
}

#[test]
fn a_pull_merged_by_hand_is_done() {
    let mut look = look(HeadStatus::None);
    look.pull = Pull::Merged;
    assert_eq!(
        pass(&look, &mut Kept::default(), &limits(), at(0)),
        Step::Done
    );
}

#[test]
fn a_closed_pull_is_the_closed_ruling() {
    let mut look = look(HeadStatus::None);
    look.pull = Pull::Closed;
    let step = pass(&look, &mut Kept::default(), &limits(), at(0));
    assert_eq!(step, Step::Rule(Ruling::Closed));
    assert_eq!(
        step.line().unwrap().text(),
        "The pull request was closed without merging."
    );
}

#[test]
fn a_new_head_restarts_the_wait() {
    // CI passed on `HEAD`, which has waited long enough.
    let mut kept = waiting(0);
    let green = look(HeadStatus::Passed);
    assert_eq!(pass(&green, &mut kept, &limits(), at(500)), Step::Yes);

    // A push moved the head, and the checks have had no time to register.
    let mut pushed = look(HeadStatus::Passed);
    pushed.pull = Pull::Open(pull(OTHER, HeadStatus::Passed));
    pushed.head = OTHER.to_owned();
    assert_eq!(
        pass(&pushed, &mut kept, &limits(), at(500)),
        Step::Wait(Line::WaitingForCi)
    );
    assert_eq!(
        kept.wait,
        Some(Wait {
            head: OTHER.to_owned(),
            since: at(500)
        })
    );
}

#[test]
fn a_lagging_base_waits_and_says_so_after_fifteen_minutes() {
    let mut lagging = look(HeadStatus::Passed);
    lagging.base = Base::Lagging;
    let mut kept = waiting(0);

    // The forge's head lags a push by a moment.
    assert_eq!(
        pass(&lagging, &mut kept, &limits(), at(1000)),
        Step::Wait(Line::WaitingForCi)
    );
    assert_eq!(
        pass(&lagging, &mut kept, &limits(), at(1000 + LAG_LIMIT - 1)),
        Step::Wait(Line::WaitingForCi)
    );
    let step = pass(&lagging, &mut kept, &limits(), at(1000 + LAG_LIMIT));
    let line = Line::BaseLagging {
        head: HEAD.to_owned(),
    };
    assert_eq!(step, Step::Wait(line.clone()));
    assert!(
        line.text().contains("aaaaaaa for 15 minutes"),
        "{}",
        line.text()
    );

    // Once the branch matches again the lag is forgotten.
    assert_eq!(
        pass(&look(HeadStatus::Passed), &mut kept, &limits(), at(5000)),
        Step::Yes
    );
    assert_eq!(kept.lag, None);
}

#[test]
fn a_label_or_draft_state_changed_by_someone_else_is_a_foreign_change() {
    let mut kept = waiting(0);
    let mut labelled = pull(HEAD, HeadStatus::Passed);
    labelled.labels = vec!["urgent".to_owned(), "bug".to_owned()];
    let mut look = look(HeadStatus::Passed);
    look.pull = Pull::Open(labelled);
    let step = pass(&look, &mut kept, &limits(), at(500));
    assert_eq!(
        step,
        Step::Rule(Ruling::ForeignChange {
            what: "the `urgent`, `bug` labels were added".to_owned()
        })
    );

    // Marked ready by someone, with the labels as they were.
    let mut ready = pull(HEAD, HeadStatus::Passed);
    ready.draft = false;
    look.pull = Pull::Open(ready);
    let step = pass(&look, &mut waiting(0), &limits(), at(500));
    assert_eq!(
        ruled(&step),
        &Ruling::ForeignChange {
            what: "it was marked ready for review".to_owned()
        }
    );

    // Kelpie marking it ready is no change.
    let mut mine = waiting(0);
    mine.marked_ready();
    assert_eq!(pass(&look, &mut mine, &limits(), at(500)), Step::Yes);
}

#[test]
fn the_first_pass_takes_the_labels_and_draft_state_it_sees_as_its_own() {
    let mut kept = Kept::default();
    let mut labelled = pull(HEAD, HeadStatus::Passed);
    labelled.labels = vec!["kelpie".to_owned()];
    let mut look = look(HeadStatus::Passed);
    look.pull = Pull::Open(labelled);
    assert_eq!(
        pass(&look, &mut kept, &limits(), at(500)),
        Step::Wait(Line::WaitingForCi)
    );
    assert_eq!(
        kept.seen,
        Some(Seen {
            labels: vec!["kelpie".to_owned()],
            draft: true
        })
    );
}

#[test]
fn a_head_neither_the_worker_nor_kelpie_pushed_is_a_foreign_change() {
    let mut look = look(HeadStatus::Passed);
    look.pull = Pull::Open(pull(OTHER, HeadStatus::Passed));
    let mut kept = waiting(0);
    kept.wait.as_mut().unwrap().head = OTHER.to_owned();
    let step = pass(&look, &mut kept, &limits(), at(500));
    assert_eq!(
        step,
        Step::Rule(Ruling::ForeignChange {
            what: "its head moved to bbbbbbb, a commit the worker did not push".to_owned()
        })
    );

    // A head kelpie pushed itself, in a catch-up, is not foreign.
    kept.pushed(OTHER);
    assert_eq!(pass(&look, &mut kept, &limits(), at(500)), Step::Yes);
}

#[test]
fn a_harness_file_in_the_diff_is_the_agents_files_ruling() {
    let mut look = look(HeadStatus::Passed);
    look.agents_files = vec![".claude/settings.json".to_owned(), "CLAUDE.md".to_owned()];
    let step = pass(&look, &mut waiting(0), &limits(), at(500));
    assert_eq!(
        ruled(&step).reason(),
        "The branch changes a harness's own files: .claude/settings.json, CLAUDE.md."
    );
}

#[test]
fn a_branch_behind_its_base_is_caught_up() {
    let mut look = look(HeadStatus::Failed(vec![failed("test")]));
    look.base = Base::Behind;
    let step = pass(&look, &mut waiting(0), &limits(), at(500));
    assert_eq!(step, Step::CatchUp);
    assert_eq!(step.line(), Some(Line::CatchingUp));
}

#[test]
fn a_conflict_is_a_fix_turn_up_to_three_then_the_conflict_ruling() {
    let files = vec!["work.txt".to_owned()];
    let mut kept = Kept::default();
    for turn in 1..=CONFLICT_TURNS {
        let (head, base) = (format!("head{turn}"), format!("base{turn}"));
        let step = conflicted(&kept, &head, &base, &files);
        let Step::Conflict(prompt) = step else {
            panic!("turn {turn}: {step:?}");
        };
        assert!(
            prompt.starts_with("Your pull request conflicts with its base in work.txt."),
            "{prompt}"
        );
        assert!(prompt.contains("do not force-push"), "{prompt}");
        kept.conflict_sent(&head, &base);
    }
    assert_eq!(kept.conflict.as_ref().unwrap().turns, 3);
    let step = conflicted(&kept, "head4", "base4", &files);
    assert_eq!(
        step,
        Step::Rule(Ruling::Conflict {
            files: files.clone()
        })
    );

    // The worker was already sent for this base, or this head, and did not resolve it.
    let mut kept = Kept::default();
    kept.conflict_sent("head1", "base1");
    assert!(matches!(
        conflicted(&kept, "head2", "base1", &files),
        Step::Rule(_)
    ));
    assert!(matches!(
        conflicted(&kept, "head1", "base2", &files),
        Step::Rule(_)
    ));
}

#[test]
fn ci_that_does_not_hold_the_merge_skips_to_the_yes() {
    let mut limits = limits();
    limits.ci_holds_merge = false;
    let look = look(HeadStatus::Failed(vec![failed("test")]));
    assert_eq!(pass(&look, &mut waiting(500), &limits, at(500)), Step::Yes);
}

#[test]
fn checks_settle_for_two_minutes_after_a_push() {
    let look = look(HeadStatus::Passed);
    let wait = Step::Wait(Line::WaitingForCi);
    assert_eq!(pass(&look, &mut waiting(1000), &limits(), at(1000)), wait);
    assert_eq!(
        pass(
            &look,
            &mut waiting(1000),
            &limits(),
            at(1000 + CHECKS_SETTLE - 1)
        ),
        wait
    );
    assert_eq!(
        pass(
            &look,
            &mut waiting(1000),
            &limits(),
            at(1000 + CHECKS_SETTLE)
        ),
        Step::Yes
    );
}

#[test]
fn no_checks_and_no_pull_request_workflow_on_the_base_is_skipped() {
    let mut look = look(HeadStatus::None);
    look.base_runs_pull_workflow = false;
    assert_eq!(
        pass(&look, &mut waiting(0), &limits(), at(CHECKS_SETTLE)),
        Step::Yes
    );
}

#[test]
fn no_checks_past_the_ci_wait_with_a_workflow_is_the_no_checks_ruling() {
    let look = look(HeadStatus::None);
    // The CI wait is ten minutes.
    assert_eq!(
        pass(&look, &mut waiting(0), &limits(), at(599)),
        Step::Wait(Line::WaitingForCi)
    );
    let step = pass(&look, &mut waiting(0), &limits(), at(600));
    assert_eq!(
        step,
        Step::Rule(Ruling::NoChecks {
            head: HEAD.to_owned()
        })
    );
}

#[test]
fn a_run_stuck_queued_past_the_ci_wait_is_rerun_once_then_the_check_queued_ruling() {
    let stuck = HeadStatus::Pending(vec![queued("test", Some(9))]);
    let look = look(stuck);
    let mut kept = waiting(0);
    assert_eq!(
        pass(&look, &mut kept, &limits(), at(599)),
        Step::Wait(Line::WaitingForCi)
    );
    assert_eq!(
        pass(&look, &mut kept, &limits(), at(600)),
        Step::Rerun(vec![9])
    );

    // The rerun started, so the wait begins again, and the run is not rerun twice.
    kept.reran_queued(HEAD, 9, at(600));
    assert_eq!(
        pass(&look, &mut kept, &limits(), at(1199)),
        Step::Wait(Line::WaitingForCi)
    );
    let step = pass(&look, &mut kept, &limits(), at(1200));
    assert_eq!(
        step,
        Step::Rule(Ruling::CheckQueued {
            head: HEAD.to_owned(),
            checks: vec!["test".to_owned()],
            held: false,
            run: Some(9),
        })
    );
    assert_eq!(
        ruled(&step).reason(),
        "test stayed queued at aaaaaaa after its run (9) was rerun."
    );
}

#[test]
fn a_run_that_is_running_after_its_rerun_is_waited_on_to_the_jobs_own_timeout() {
    let mut running = queued("test", Some(9));
    running.running = true;
    let look = look(HeadStatus::Pending(vec![running]));
    let mut kept = waiting(0);
    kept.reran_queued(HEAD, 9, at(0));
    assert_eq!(
        pass(&look, &mut kept, &limits(), at(JOB_TIMEOUT - 1)),
        Step::Wait(Line::WaitingForCi)
    );
    assert!(matches!(
        pass(&look, &mut kept, &limits(), at(JOB_TIMEOUT)),
        Step::Rule(Ruling::CheckQueued { .. })
    ));
}

#[test]
fn checks_held_for_approval_are_ruled_on_and_never_rerun() {
    let mut held = queued("test", Some(9));
    held.held = true;
    let look = look(HeadStatus::Pending(vec![held]));
    let step = pass(&look, &mut waiting(0), &limits(), at(CHECKS_SETTLE));
    assert!(
        ruled(&step)
            .reason()
            .starts_with("test at aaaaaaa wait for your approval"),
        "{step:?}"
    );
}

#[test]
fn passed_or_skipped_on_the_latest_base_is_the_yes() {
    for checks in [HeadStatus::Passed, HeadStatus::Skipped] {
        let step = pass(&look(checks), &mut waiting(0), &limits(), at(500));
        assert_eq!(step, Step::Yes);
        assert_eq!(step.line(), Some(Line::ReadyToMerge));
    }
}

#[test]
fn red_checks_go_to_the_red_path_linking_the_failing_run() {
    let checks = vec![failed("lint"), failed("test")];
    let step = pass(
        &look(HeadStatus::Failed(checks.clone())),
        &mut waiting(0),
        &limits(),
        at(500),
    );
    assert_eq!(step, Step::Red(checks));
    assert_eq!(
        step.line(),
        Some(Line::FixingRed {
            link: Some("https://github.com/o/r/actions/runs/9/job/lint".to_owned())
        })
    );
    assert_eq!(step.line().unwrap().text(), "Fixing red CI");
}

#[test]
fn the_same_head_red_again_with_nothing_pushed_is_still_red() {
    let checks = vec![failed("lint")];
    let logs = Logs::Read(vec![log("lint", 9, "error in src/lib.rs")]);
    let changed = vec!["src/lib.rs".to_owned()];
    let mut kept = Kept::default();
    assert!(matches!(
        red(HEAD, &checks, &logs, &changed, &kept, &limits()),
        Red::FixTurn(_)
    ));

    kept.fix_turn_sent(HEAD, true);
    let answer = red(HEAD, &checks, &logs, &changed, &kept, &limits());
    let Red::Rule(ruling) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(
        ruling.reason(),
        "CI failed again at aaaaaaa, and the worker pushed no fix: lint."
    );
    assert_eq!(
        Line::of(&ruling).link(),
        Some("https://github.com/o/r/actions/runs/9/job/lint")
    );

    // A new head is a new red run.
    assert!(matches!(
        red(OTHER, &checks, &logs, &changed, &kept, &limits()),
        Red::FixTurn(_)
    ));
}

#[test]
fn logs_naming_no_changed_file_rerun_once_per_head() {
    let checks = vec![failed("test")];
    let logs = Logs::Read(vec![
        log("test", 9, "flaky: connection reset"),
        log("lint", 8, "also unrelated"),
        log("test again", 9, "same run"),
    ]);
    let changed = vec!["src/lib.rs".to_owned()];
    let mut kept = Kept::default();
    assert_eq!(
        red(HEAD, &checks, &logs, &changed, &kept, &limits()),
        Red::Rerun(vec![9, 8])
    );

    // Reran, and red again: the failure repeats, and the worker is told so.
    kept.reran_failed(HEAD, at(100));
    let Red::FixTurn(prompt) = red(HEAD, &checks, &logs, &changed, &kept, &limits()) else {
        panic!("a fix turn");
    };
    assert!(
        prompt.contains("the failure repeats, so do not call it a flake"),
        "{prompt}"
    );

    // A log that names a changed file is the worker's to fix, and so is one
    // nothing was read from.
    let named = Logs::Read(vec![log("test", 9, "panicked at src/lib.rs:4")]);
    let fresh = Kept::default();
    assert!(matches!(
        red(HEAD, &checks, &named, &changed, &fresh, &limits()),
        Red::FixTurn(_)
    ));
    assert!(matches!(
        red(
            HEAD,
            &checks,
            &Logs::Refused("no".into()),
            &changed,
            &fresh,
            &limits()
        ),
        Red::FixTurn(_)
    ));
    assert!(matches!(
        red(
            HEAD,
            &checks,
            &Logs::Read(vec![]),
            &changed,
            &fresh,
            &limits()
        ),
        Red::FixTurn(_)
    ));
    assert!(matches!(
        red(HEAD, &checks, &logs, &[], &fresh, &limits()),
        Red::FixTurn(_)
    ));
}

#[test]
fn past_the_fix_attempts_is_still_red_and_zero_rules_on_the_first_red_run() {
    let checks = vec![failed("lint")];
    let logs = Logs::Read(vec![log("lint", 9, "error in src/lib.rs")]);
    let changed = vec!["src/lib.rs".to_owned()];
    let capped = |turns: u32| Limits {
        fix_attempts: Some(serde_json::from_value::<FixAttempts>(turns.into()).unwrap()),
        ..limits()
    };

    // 0 rules on the first red run.
    let answer = red(HEAD, &checks, &logs, &changed, &Kept::default(), &capped(0));
    let Red::Rule(ruling) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(
        ruling.reason(),
        "CI failed at aaaaaaa: lint. The worker has had no fix turn on red runs, and the \
         project's fix attempts are used up."
    );

    // 2 allows two fix turns, each on a head of its own, and rules on the third red head.
    let mut kept = Kept::default();
    for head in ["h1", "h2"] {
        assert!(matches!(
            red(head, &checks, &logs, &changed, &kept, &capped(2)),
            Red::FixTurn(_)
        ));
        kept.fix_turn_sent(head, true);
    }
    let answer = red("h3", &checks, &logs, &changed, &kept, &capped(2));
    assert!(matches!(
        &answer,
        Red::Rule(Ruling::StillRed {
            fix_turns: Some(2),
            ..
        })
    ));
    let Red::Rule(ruling) = answer else {
        unreachable!()
    };
    assert!(
        ruling.reason().contains("2 fix turns"),
        "{}",
        ruling.reason()
    );

    // No cap is every red head a fix turn.
    for head in ["h3", "h4", "h5"] {
        assert!(matches!(
            red(head, &checks, &logs, &changed, &kept, &limits()),
            Red::FixTurn(_)
        ));
        kept.fix_turn_sent(head, true);
    }
}

#[test]
fn a_fix_turn_carries_the_checks_name_and_its_log() {
    let checks = vec![failed("lint"), failed("test")];
    let logs = Logs::Read(vec![log("lint", 9, "error[E0308]: mismatched types")]);
    let Red::FixTurn(prompt) = red(HEAD, &checks, &logs, &[], &Kept::default(), &limits()) else {
        panic!("a fix turn");
    };
    assert!(
        prompt.starts_with("CI failed on your pull request at aaaaaaa: lint, test."),
        "{prompt}"
    );
    assert_eq!(
        prompt.matches("error[E0308]: mismatched types").count(),
        1,
        "{prompt}"
    );
    assert!(
        prompt.contains("--- lint (run 9, https://github.com/o/r/actions/runs/9)"),
        "{prompt}"
    );
    assert!(prompt.contains("starting from `origin`'s head"), "{prompt}");
    assert!(prompt.contains("force-push"), "{prompt}");
    assert!(prompt.contains("hand-over file"), "{prompt}");

    let Red::FixTurn(unread) = red(
        HEAD,
        &checks,
        &Logs::Refused("rate limited".into()),
        &[],
        &Kept::default(),
        &limits(),
    ) else {
        panic!("a fix turn");
    };
    assert!(
        unread.contains("could not read it for you (rate limited)"),
        "{unread}"
    );
}

#[test]
fn a_queue_removal_goes_back_uncounted_naming_the_merge_queue() {
    let mut kept = Kept::default();
    let reason = "Required status check \"test\" failed.";
    let Red::FixTurn(prompt) = removed(&kept, HEAD, reason) else {
        panic!("a fix turn");
    };
    assert!(
        prompt.starts_with(&format!(
            "The merge queue removed your pull request at aaaaaaa: {reason}."
        )),
        "{prompt}"
    );
    assert!(
        prompt.contains("gh run view <run id> --log-failed"),
        "{prompt}"
    );

    // Sent back, and not counted against the fix attempts.
    kept.fix_turn_sent(HEAD, false);
    assert_eq!(kept.fix_turns, 0);
}

#[test]
fn a_second_removal_at_the_same_head_is_merge_refused() {
    let mut kept = Kept::default();
    kept.fix_turn_sent(HEAD, false);
    let answer = removed(&kept, HEAD, "the queue's test failed");
    let Red::Rule(ruling) = answer else {
        panic!("{answer:?}");
    };
    assert_eq!(
        ruling,
        Ruling::MergeRefused {
            head: HEAD.to_owned(),
            why: "the queue's test failed".to_owned()
        }
    );
    assert_eq!(
        ruling.reason(),
        "Kelpie could not merge at aaaaaaa: the queue's test failed."
    );
    // A removal at another head goes back to the worker.
    assert!(matches!(removed(&kept, OTHER, "again"), Red::FixTurn(_)));
}

#[test]
fn the_kept_facts_survive_a_reload_of_gate_json() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("project").join(FILE);
    let ticket: TicketId = "7".parse().unwrap();
    let local: TicketId = "k3".parse().unwrap();

    let mut book = Book::load(file.clone()).unwrap();
    assert_eq!(book.get(ticket), None);
    let kept = book.entry(ticket);
    kept.pushed(HEAD);
    kept.seen = Some(Seen {
        labels: vec!["bug".to_owned()],
        draft: true,
    });
    kept.wait = Some(Wait {
        head: HEAD.to_owned(),
        since: at(1_700_000_000),
    });
    kept.reran_failed(OTHER, at(1_700_000_100));
    kept.reran_queued(HEAD, 9, at(1_700_000_200));
    kept.fix_turn_sent(HEAD, true);
    kept.conflict_sent(HEAD, OTHER);
    kept.lag = Some(Lag {
        head: OTHER.to_owned(),
        since: at(1_700_000_300),
    });
    kept.said_yes(HEAD);
    kept.merge_refused();
    kept.removals_seen(2);
    book.entry(local).pushed(OTHER);
    book.save().unwrap();

    let written = std::fs::read_to_string(&file).unwrap();
    assert!(written.contains("\"7\""), "{written}");
    assert!(!file.with_extension("json.partial").exists());

    let loaded = Book::load(file).unwrap();
    assert_eq!(loaded.get(ticket), book.get(ticket));
    assert_eq!(loaded.get(local), book.get(local));
    let kept = loaded.get(ticket).unwrap();
    assert!(kept.yes_holds(HEAD) && !kept.yes_holds(OTHER));
    assert_eq!(
        (kept.fix_turns, kept.merge_refusals, kept.queue_removals),
        (1, 1, 2)
    );
    assert_eq!(kept.queued.as_ref().unwrap().runs, [9]);
}

#[test]
fn a_gate_json_that_is_not_the_facts_is_refused_and_a_forgotten_ticket_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(FILE);
    std::fs::write(&file, "[1, 2]").unwrap();
    assert_eq!(
        Book::load(file.clone()).unwrap_err().kind(),
        std::io::ErrorKind::InvalidData
    );

    std::fs::remove_file(&file).unwrap();
    let ticket: TicketId = "7".parse().unwrap();
    let mut book = Book::load(file).unwrap();
    book.entry(ticket).said_yes(HEAD);
    book.forget(ticket);
    assert_eq!(book.get(ticket), None);
}

#[test]
fn the_limits_come_from_the_settings() {
    let dir = tempfile::tempdir().unwrap();
    let config = crate::settings::ConfigDir::at(dir.path());
    let resolved = crate::settings::Settings::load(&config, "web").unwrap();
    let limits = Limits::from(&resolved);
    assert!(limits.ci_holds_merge);
    assert_eq!(limits.fix_attempts, None);
    assert_eq!(limits.ci_wait.duration().as_secs(), 3600);
}
