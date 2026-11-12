use serde_json::json;

use super::{Finding, Severity};
use super::{ROUND_FAILURES, Review, ReviewStage, both_looks};

fn finding(severity: Severity, file: &str, line: u32, what: &str, why: &str) -> Finding {
    Finding::new(severity, file, line, what, why)
}

#[test]
fn both_looks_keep_each_finding_once_worst_first() {
    let first = vec![
        finding(Severity::Low, "a.rs", 1, "nit", "style"),
        finding(Severity::High, "b.rs", 2, "bad", "breaks"),
    ];
    let more = vec![
        // The same finding, said differently: kept once, as the first look had it
        finding(Severity::High, "b.rs", 2, "bad", "crashes"),
        finding(Severity::Medium, "c.rs", 3, "meh", "slow"),
        finding(Severity::Low, "d.rs", 4, "another nit", "style"),
    ];

    let all = both_looks(first, more);

    let seen: Vec<_> = all.iter().map(|f| (f.severity, f.file.as_str())).collect();
    assert_eq!(
        seen,
        [
            (Severity::High, "b.rs"),
            (Severity::Medium, "c.rs"),
            (Severity::Low, "a.rs"),
            (Severity::Low, "d.rs"),
        ]
    );
    assert_eq!(all[0].why, "breaks");
}

#[test]
fn the_next_round_clears_the_stage_and_the_failures() {
    let mut review = Review::first();
    review.round = 1;
    review.reviewer = Some("rust-eyes".to_owned());
    review.ran = vec!["kelpie:reviewer".to_owned()];
    review.failures = ROUND_FAILURES - 1;
    review.unread = false;
    review.reading = Some("c0ffee".to_owned());
    review.stage = ReviewStage::Pushing;

    let next = review.next_round();

    assert_eq!(next.round, 2);
    assert_eq!(next.stage, ReviewStage::Round);
    assert_eq!(next.reviewer, None);
    assert_eq!(next.failures, 0);
    assert_eq!(next.reading, None);
    // The reviewer that just ran joins the ones the pass has run, once
    assert_eq!(next.ran, ["kelpie:reviewer", "rust-eyes"]);
    assert!(!next.unread);
}

#[test]
fn a_pass_round_trips_through_json() {
    let sent = finding(Severity::High, "a.rs", 3, "bad", "breaks");
    let stages = [
        ReviewStage::Round,
        ReviewStage::Found {
            findings: vec![sent.clone()],
        },
        ReviewStage::Fixing {
            head: "c0ffee".to_owned(),
            sent: vec![sent.clone()],
            deferred_before: Vec::new(),
        },
        ReviewStage::Pushing,
        ReviewStage::SecondLook { first: vec![sent] },
    ];
    for stage in stages {
        let review = Review {
            round: 2,
            stage,
            reviewer: Some("rust-eyes".to_owned()),
            ran: vec!["kelpie:reviewer".to_owned()],
            failures: 1,
            unread: true,
            reading: Some("c0ffee".to_owned()),
        };

        let text = serde_json::to_string(&review).unwrap();
        let back: Review = serde_json::from_str(&text).unwrap();

        assert_eq!(back, review, "{text}");
    }

    // The saved shape: a first round holds nothing a later one adds
    assert_eq!(
        serde_json::to_value(Review::first()).unwrap(),
        json!({ "round": 1, "stage": { "stage": "round" }, "unread": true })
    );
}
