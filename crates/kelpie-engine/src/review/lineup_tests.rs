use std::fs;

use super::lineup::{Lineup, RULE_READS, matches};
use super::{ROUND_FAILURES, Review};
use crate::agents::{Agents, FOLDER};
use crate::settings::{ConfigDir, Reviewers};

/// The lineup of `names` over reviewer files written as `(file name, text)`
fn lineup(files: &[(&str, &str)], names: &[&str]) -> Lineup {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join(FOLDER);
    fs::create_dir_all(&folder).unwrap();
    for (file, text) in files {
        fs::write(folder.join(file), text).unwrap();
    }
    let agents = Agents::load(&ConfigDir::at(dir.path())).unwrap();
    let reviewers: Reviewers = serde_json::from_value(serde_json::json!(names)).unwrap();
    Lineup::of(&reviewers, &agents)
}

const PLAIN: &str = "---\nrole: reviewer\n---\nRead it.\n";

fn three() -> Lineup {
    lineup(
        &[("a.md", PLAIN), ("b.md", PLAIN), ("c.md", PLAIN)],
        &["a", "b", "c"],
    )
}

fn ran(names: &[&str]) -> Review {
    Review {
        ran: names.iter().map(|n| (*n).to_owned()).collect(),
        ..Review::first()
    }
}

fn names<'a>(due: &[&'a super::lineup::Listed]) -> Vec<&'a str> {
    due.iter().map(|r| r.name.as_str()).collect()
}

fn files(files: &[&str]) -> Vec<String> {
    files.iter().map(|f| (*f).to_owned()).collect()
}

#[test]
fn each_reviewer_is_due_once_a_pass_in_the_projects_order() {
    let lineup = three();

    assert_eq!(names(&lineup.due(&ran(&[]), &[])), ["a", "b", "c"]);
    assert_eq!(names(&lineup.due(&ran(&["a"]), &[])), ["b", "c"]);
    assert_eq!(names(&lineup.due(&ran(&["a", "c"]), &[])), ["b"]);
    assert!(lineup.due(&ran(&["a", "b", "c"]), &[]).is_empty());
}

#[test]
fn a_reviewer_whose_paths_the_diff_does_not_touch_is_not_due() {
    let rust = "---\nrole: reviewer\npaths:\n  - \"src/**/*.rs\"\n---\nRead it.\n";
    let lineup = lineup(&[("rust.md", rust), ("any.md", PLAIN)], &["rust", "any"]);

    let docs = lineup.due(&ran(&[]), &files(&["docs/a.md"]));
    let code = lineup.due(&ran(&[]), &files(&["docs/a.md", "src/deep/lib.rs"]));

    assert_eq!(names(&docs), ["any"]);
    assert_eq!(names(&code), ["rust", "any"]);
}

#[test]
fn a_glob_keeps_single_stars_inside_one_folder() {
    assert!(matches("*.lock", "Cargo.lock"));
    assert!(!matches("*.lock", "sub/Cargo.lock"));
    assert!(matches("**/*.lock", "sub/deeper/Cargo.lock"));
    assert!(matches("**/*.lock", "Cargo.lock"));
    assert!(matches("docs/**", "docs/a/b.md"));
    assert!(matches("a?c", "abc"));
    assert!(!matches("a?c", "a/c"));
    assert!(!matches("Cargo.lock", "Cargo.lockx"));
}

#[test]
fn a_pass_with_two_reads_has_enough() {
    let lineup = three();

    assert_eq!(RULE_READS, 2);
    assert!(!lineup.enough_read(&ran(&[]), &[], RULE_READS));
    assert!(!lineup.enough_read(&ran(&["a"]), &[], RULE_READS));
    assert!(lineup.enough_read(&ran(&["a", "b"]), &[], RULE_READS));
}

#[test]
fn a_reviewer_passed_over_after_three_failures_is_no_read() {
    let lineup = three();
    let mut round = Review {
        reviewer: Some("b".to_owned()),
        ..ran(&["a"])
    };
    for _ in 1..ROUND_FAILURES {
        assert!(!round.fail(), "it is tried again until the third failure");
    }
    assert!(round.fail(), "the third failure in a row passes it over");
    let passed_over = vec!["b".to_owned()];

    let after = round.next_round();

    // It ran, so it is not due again, but it is no read
    assert_eq!(names(&lineup.due(&after, &[])), ["c"]);
    assert_eq!(lineup.reads(&after, &passed_over), 1);
    assert!(!lineup.enough_read(&after, &passed_over, RULE_READS));
}

#[test]
fn a_listed_name_with_no_reviewer_file_is_left_out_with_its_reason() {
    let lineup = lineup(&[("a.md", PLAIN)], &["a", "ghost"]);

    assert_eq!(names(&lineup.due(&ran(&[]), &[])), ["a"]);
    let [left] = lineup.left_out() else {
        panic!("one name has no file: {:?}", lineup.left_out());
    };
    assert_eq!(left.name, "ghost");
    assert!(left.why.contains("ghost.md"), "{}", left.why);
    assert!(left.why.contains("no reviewer"), "{}", left.why);
}

#[test]
fn a_pass_that_ends_short_names_who_was_missed() {
    let lineup = lineup(&[("a.md", PLAIN), ("b.md", PLAIN)], &["a", "b", "ghost"]);
    let passed_over = vec!["b".to_owned()];
    let review = Review {
        unread: false,
        ..ran(&["a", "b"])
    };

    let short = lineup.pass_unread(&review, &passed_over, RULE_READS);

    let Some(Some(why)) = short else {
        panic!("one of two reads is short: {short:?}");
    };
    assert!(why.starts_with("1 of 2 reviewers read it: "), "{why}");
    assert!(
        why.contains("b was passed over after its calls kept failing"),
        "{why}"
    );
    assert!(why.contains("ghost is left out"), "{why}");

    // With its reads in, the pass is not short, whoever else was missed
    let read = Review {
        unread: false,
        ..ran(&["a", "b"])
    };
    assert_eq!(lineup.pass_unread(&read, &[], RULE_READS), None);
}

#[test]
fn a_pass_that_read_less_for_want_of_a_longer_list_is_not_short_with_no_one_missed() {
    let lineup = lineup(&[("a.md", PLAIN)], &["a"]);
    let review = Review {
        unread: false,
        ..ran(&["a"])
    };

    assert_eq!(lineup.pass_unread(&review, &[], RULE_READS), None);
}

#[test]
fn an_empty_reviewers_list_leaves_the_pass_unread_with_no_reason_of_a_failure() {
    let lineup = lineup(&[], &[]);

    assert!(lineup.due(&Review::first(), &[]).is_empty());
    assert_eq!(
        lineup.pass_unread(&Review::first(), &[], RULE_READS),
        Some(None)
    );
}
