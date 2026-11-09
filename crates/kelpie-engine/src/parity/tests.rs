use super::*;

/// A matrix text with the given providers and one row of `cells`
fn matrix_of(providers: &str, cells: &str) -> String {
    format!(
        "[harness]\nproviders = [{providers}]\n\n[harness.send]\nwhat = \"send a message\"\n{cells}\n"
    )
}

fn problems_of(text: &str) -> Vec<String> {
    read_matrix(text, &root()).problems
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_owned).collect()
}

fn repo_matrix() -> Matrix {
    let text = std::fs::read_to_string(root().join(MATRIX)).expect("docs/parity.toml reads");
    read_matrix(&text, &root())
}

#[test]
fn the_matrix_lists_exactly_the_harnesses_the_code_has() {
    let problems = provider_problems(&repo_matrix(), &named_by_schema::<Harness>());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_harness_the_matrix_lacks_or_invents_is_named() {
    let matrix = read_matrix(&matrix_of("\"claude-code\", \"pi\"", ""), &root());
    let code = BTreeSet::from(["claude-code".to_owned(), "goose".to_owned()]);
    let problems = provider_problems(&matrix, &code);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("lacks goose"), "{problems:?}");
    assert!(problems[1].contains("lists pi"), "{problems:?}");
}

#[test]
fn each_row_has_a_cell_for_every_harness() {
    let text = matrix_of("\"claude-code\", \"goose\"", "claude-code = \"yes\"");
    let problems = problems_of(&text);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].contains("harness.send has no cell for goose"),
        "{problems:?}"
    );
    assert!(problems[0].contains(CELL_FORMS), "{problems:?}");

    let full = matrix_of(
        "\"claude-code\", \"goose\"",
        "claude-code = \"yes\"\ngoose = { gap = 12 }",
    );
    assert_eq!(problems_of(&full), Vec::<String>::new());
}

#[test]
fn a_row_without_a_description_or_with_a_stranger_is_named() {
    let text =
        "[harness]\nproviders = [\"goose\"]\n\n[harness.send]\ngoose = \"yes\"\npi = \"yes\"\n";
    let problems = problems_of(text);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(
        problems
            .iter()
            .any(|p| p.contains("has `pi`, which is not a harness")),
        "{problems:?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("says in `what` what the feature is")),
        "{problems:?}"
    );
}

#[test]
fn a_malformed_cell_names_the_forms_it_may_take() {
    for cell in ["\"maybe\"", "3", "{ cannot = \"no way\" }"] {
        let problems = problems_of(&matrix_of("\"goose\"", &format!("goose = {cell}")));
        assert_eq!(problems.len(), 1, "{cell}: {problems:?}");
        assert!(problems[0].contains(CELL_FORMS), "{cell}: {problems:?}");
    }
    for cell in [
        "{ gap = 0 }",
        "{ gap = \"soon\" }",
        "{ gap = 4, doc = \"x\" }",
    ] {
        let problems = problems_of(&matrix_of("\"goose\"", &format!("goose = {cell}")));
        assert_eq!(problems.len(), 1, "{cell}: {problems:?}");
        assert!(
            problems[0].contains("{ gap = <n> }"),
            "{cell}: {problems:?}"
        );
    }
}

#[test]
fn a_cannot_cell_cites_a_file_that_says_it() {
    let cell = |doc: &str, says: &str| {
        problems_of(&matrix_of(
            "\"goose\"",
            &format!("goose = {{ cannot = \"why\", doc = \"{doc}\", says = \"{says}\" }}"),
        ))
    };
    assert_eq!(
        cell("docs/acp.md", "No cache counts."),
        Vec::<String>::new()
    );
    let missing = cell("docs/nowhere.md", "x");
    assert!(missing[0].contains("not a file in the repo"), "{missing:?}");
    let unsaid = cell("docs/acp.md", "goose reports every cache count");
    assert!(
        unsaid[0].contains("which docs/acp.md does not say"),
        "{unsaid:?}"
    );
}

#[test]
fn a_branch_on_some_harnesses_names_its_row() {
    let matrix = read_matrix(&matrix_of("\"claude-code\", \"goose\"", ""), &root());
    let untagged =
        lines("fn pick(h: Harness) {\n    if h == Harness::Goose {\n        local();\n    }\n}");
    let problems = branch_problems(&matrix, "a.rs", &untagged);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("a.rs:2 "), "{problems:?}");
    assert!(
        problems[0].contains("// parity: harness.<row>"),
        "{problems:?}"
    );

    let tagged = lines("// parity: harness.send\nif h == Harness::Goose {");
    assert_eq!(
        branch_problems(&matrix, "a.rs", &tagged),
        Vec::<String>::new()
    );

    let unknown = lines("// parity: harness.steer\nif h == Harness::Goose {");
    let problems = branch_problems(&matrix, "a.rs", &unknown);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("names no row"), "{problems:?}");
}

#[test]
fn a_wildcard_arm_after_a_harness_arm_is_a_branch() {
    let matrix = read_matrix(&matrix_of("\"claude-code\", \"goose\"", ""), &root());
    let untagged = lines("match h {\n    Harness::Goose => 1,\n    _ => 0,\n}");
    let problems = branch_problems(&matrix, "a.rs", &untagged);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("a.rs:3 "), "{problems:?}");
}

#[test]
fn a_catch_all_arm_with_a_binding_is_a_branch() {
    let matrix = read_matrix(&matrix_of("\"claude-code\", \"goose\"", ""), &root());
    let bound = lines("match h {\n    Harness::ClaudeCode => 1,\n    other => fail(other),\n}");
    let problems = branch_problems(&matrix, "a.rs", &bound);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].starts_with("a.rs:3 "), "{problems:?}");

    let tagged = lines(
        "// parity: harness.send\nmatch h {\n    Harness::ClaudeCode => 1,\n    other => fail(other),\n}",
    );
    assert_eq!(
        branch_problems(&matrix, "a.rs", &tagged),
        Vec::<String>::new()
    );
}

#[test]
fn an_arm_that_names_a_harness_or_a_guard_is_no_catch_all() {
    let matrix = read_matrix(&matrix_of("\"claude-code\", \"goose\"", ""), &root());
    let named = lines("match h {\n    Harness::ClaudeCode => 1,\n    Harness::Goose => 2,\n}");
    assert_eq!(
        branch_problems(&matrix, "a.rs", &named),
        Vec::<String>::new()
    );
    let guarded = lines("match h {\n    Harness::ClaudeCode => 1,\n    x if x.ready() => 2,\n}");
    assert_eq!(
        branch_problems(&matrix, "a.rs", &guarded),
        Vec::<String>::new()
    );
}

#[test]
fn a_name_that_only_starts_like_a_harness_is_no_branch() {
    let matrix = read_matrix(&matrix_of("\"claude-code\", \"goose\"", ""), &root());
    let other = lines("if kind == Harness::GooseDesktop {");
    assert_eq!(
        branch_problems(&matrix, "a.rs", &other),
        Vec::<String>::new()
    );
}

#[test]
fn an_inline_test_module_is_not_read_as_code() {
    let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    fn b() { if h == Harness::Goose {} }\n}\nfn c() {}\n";
    let blanked = blank_test_modules(text);
    assert_eq!(blanked.len(), 6);
    assert_eq!(blanked[0], "fn a() {}");
    assert!(blanked[1..5].iter().all(String::is_empty), "{blanked:?}");
    assert_eq!(blanked[5], "fn c() {}");
}

#[test]
fn the_repo_matrix_checks() {
    let matrix = repo_matrix();
    let mut problems = matrix.problems.clone();
    problems.extend(provider_problems(&matrix, &named_by_schema::<Harness>()));
    for path in source_files() {
        let lines =
            blank_test_modules(&std::fs::read_to_string(&path).expect("a source file reads"));
        problems.extend(branch_problems(&matrix, &relative(&path), &lines));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
