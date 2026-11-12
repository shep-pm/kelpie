use super::*;

#[test]
fn well_formed_lines_parse_in_order() {
    let text = "HIGH|src/lib.rs:42|does the bad thing|breaks prod\n\
                LOW|src/main.rs:0|a name is misleading|a reader trips on it";
    assert_eq!(
        parse_findings(text),
        vec![
            Finding::new(
                Severity::High,
                "src/lib.rs",
                42,
                "does the bad thing",
                "breaks prod"
            ),
            Finding::new(
                Severity::Low,
                "src/main.rs",
                0,
                "a name is misleading",
                "a reader trips on it"
            ),
        ]
    );
}

#[test]
fn blank_and_malformed_lines_are_skipped() {
    let text = format!(
        "\n{CLEAN}\nnot a finding at all\nMEDIUM|only|two|fields|extra\nMEDIUM|a.rs:no-number|what|why"
    );
    assert_eq!(parse_findings(&text), vec![]);
}

#[test]
fn severities_order_low_to_high() {
    assert!(Severity::Low < Severity::Medium);
    assert!(Severity::Medium < Severity::High);
}

#[test]
fn a_finding_is_the_same_by_its_file_line_and_what_and_not_its_why_or_severity() {
    let finding = |severity, why: &str| {
        Finding::new(severity, "src/lib.rs", 3, "the flag is read first", why)
    };
    let one = finding(Severity::High, "a caller sees nothing");
    assert!(one.is_same_as(&finding(Severity::Low, "worded again")));
    assert!(!one.is_same_as(&Finding {
        line: 4,
        ..one.clone()
    }));
    assert!(!one.is_same_as(&Finding {
        file: "src/b.rs".into(),
        ..one.clone()
    }));
    assert!(!one.is_same_as(&Finding {
        what: "another thing".into(),
        ..one.clone()
    }));
    assert!(one.is_same_as(&Finding {
        test: Some("tests/a.rs".into()),
        ..one.clone()
    }));
}

#[test]
fn a_fifth_field_naming_a_relative_path_is_the_findings_test() {
    let found = parse_findings("HIGH|src/lib.rs:3|it panics|a caller sees it|tests/panics.rs");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].test.as_deref(), Some("tests/panics.rs"));
    assert_eq!(found[0].why, "a caller sees it");
}

#[test]
fn a_fifth_field_that_is_not_a_relative_path_stays_part_of_why() {
    for fifth in [
        "../tests/a.rs",
        "tests/../../a.rs",
        "/etc/passwd",
        "a test.rs",
    ] {
        let line = format!("HIGH|src/lib.rs:3|it panics|a caller sees it|{fifth}");
        let found = parse_findings(&line);
        assert_eq!(found.len(), 1, "{fifth}");
        assert_eq!(found[0].test, None, "{fifth}");
        assert_eq!(found[0].why, format!("a caller sees it|{fifth}"), "{fifth}");
    }
}

#[test]
fn a_four_field_line_has_no_test() {
    let found = parse_findings("HIGH|src/lib.rs:3|it panics|a caller sees it");
    assert_eq!(found[0].test, None);
}

#[test]
fn a_reply_that_says_clean_has_no_findings() {
    assert_eq!(read_review("I read the diff.\n\nCLEAN\n"), Ok(vec![]));
}

#[test]
fn a_reply_with_neither_findings_nor_clean_is_unreadable() {
    for reply in [
        "",
        "  \n",
        "The diff looks fine to me.\n",
        "this is not CLEAN",
    ] {
        assert_eq!(
            read_review(reply),
            Err(reply.trim().to_owned()),
            "{reply:?}"
        );
    }
}
