//! `docs/parity.toml`, the harness parity matrix, checked against the code
//!
//! Each harness has each row's feature, has an open issue that adds it, or has
//! a file in the repo that says why it cannot (ADR 0006). The harnesses come
//! from the code, so a new one fails here until every row says what it can do.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::settings::Harness;

const MATRIX: &str = "docs/parity.toml";

/// The one table of the matrix, and the prefix of every row's id
const CATEGORY: &str = "harness";

// What each cell may say, for the messages that ask for one.
const CELL_FORMS: &str = "\"yes\", { gap = <the open issue that adds it> }, or \
     { cannot = \"why\", doc = \"<repo path>\", says = \"<words that file holds>\" }";

// How far above a branch its `parity:` comment may sit.
const TAG_REACH: usize = 3;

// How far above a wildcard `_ =>` arm a harness's arm makes it a branch.
const ARM_REACH: usize = 12;

#[derive(Debug, Default)]
struct Matrix {
    providers: Vec<String>,
    // Each row's id, `harness.<row>`.
    rows: BTreeSet<String>,
    // What is wrong with the file itself, found while reading it.
    problems: Vec<String>,
}

// The workspace root, two folders above this crate's own.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn strings(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// Reads `text`, citing files a cell names from `root`.
fn read_matrix(text: &str, root: &Path) -> Matrix {
    let mut matrix = Matrix::default();
    let table: toml::Table = match text.parse() {
        Ok(table) => table,
        Err(error) => {
            matrix
                .problems
                .push(format!("{MATRIX} is not TOML: {error}"));
            return matrix;
        }
    };
    for (category, value) in &table {
        if category != CATEGORY {
            matrix.problems.push(format!(
                "{MATRIX}: `{category}` is no category: only `{CATEGORY}` is checked"
            ));
            continue;
        }
        let Some(category_table) = value.as_table() else {
            matrix
                .problems
                .push(format!("{MATRIX}: `{CATEGORY}` is not a table"));
            continue;
        };
        matrix.providers = strings(category_table.get("providers"));
        for (name, row) in category_table {
            if name == "providers" {
                continue;
            }
            let Some(row) = row.as_table() else {
                matrix
                    .problems
                    .push(format!("{MATRIX}: `{CATEGORY}.{name}` is not a table"));
                continue;
            };
            matrix.rows.insert(format!("{CATEGORY}.{name}"));
            read_row(name, row, &matrix.providers, root, &mut matrix.problems);
        }
    }
    matrix
}

fn read_row(
    name: &str,
    row: &toml::Table,
    providers: &[String],
    root: &Path,
    problems: &mut Vec<String>,
) {
    let id = format!("{CATEGORY}.{name}");
    for (key, value) in row {
        if key == "what" {
            continue;
        }
        if providers.contains(key) {
            if let Err(why) = read_cell(value, root) {
                problems.push(format!("{MATRIX}: {id}.{key} {why}"));
            }
        } else {
            problems.push(format!(
                "{MATRIX}: {id} has `{key}`, which is not a harness the matrix lists"
            ));
        }
    }
    if !row.get("what").is_some_and(toml::Value::is_str) {
        problems.push(format!("{MATRIX}: {id} says in `what` what the feature is"));
    }
    for provider in providers.iter().filter(|p| !row.contains_key(*p)) {
        problems.push(format!(
            "{MATRIX}: {id} has no cell for {provider}: say {CELL_FORMS}"
        ));
    }
}

fn read_cell(value: &toml::Value, root: &Path) -> Result<(), String> {
    if value.as_str() == Some("yes") {
        return Ok(());
    }
    let Some(table) = value.as_table() else {
        return Err(format!("is neither of {CELL_FORMS}"));
    };
    if let Some(issue) = table.get("gap") {
        return match issue.as_integer() {
            Some(n) if n > 0 && table.len() == 1 => Ok(()),
            _ => Err("names its issue by number, and nothing else: { gap = <n> }".into()),
        };
    }
    let field = |key: &str| table.get(key).and_then(toml::Value::as_str);
    let (Some(_), Some(doc), Some(says)) = (field("cannot"), field("doc"), field("says")) else {
        return Err(format!("is neither of {CELL_FORMS}"));
    };
    let Ok(text) = std::fs::read_to_string(root.join(doc)) else {
        return Err(format!("cites {doc}, which is not a file in the repo"));
    };
    if !text.contains(says) {
        return Err(format!("quotes \"{says}\", which {doc} does not say"));
    }
    Ok(())
}

// Every string a schema allows: its `enum` values and `const`s.
fn schema_names(value: &Value, names: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(s)) = map.get("const") {
                names.insert(s.clone());
            }
            if let Some(Value::Array(values)) = map.get("enum") {
                names.extend(values.iter().filter_map(|v| v.as_str().map(str::to_owned)));
            }
            map.values().for_each(|v| schema_names(v, names));
        }
        Value::Array(values) => values.iter().for_each(|v| schema_names(v, names)),
        _ => {}
    }
}

// The names settings files and agent files use for each variant.
fn named_by_schema<T: schemars::JsonSchema>() -> BTreeSet<String> {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    let mut names = BTreeSet::new();
    schema_names(schema.as_value(), &mut names);
    names
}

// How the matrix's harness list differs from `code`'s.
fn provider_problems(matrix: &Matrix, code: &BTreeSet<String>) -> Vec<String> {
    let listed: BTreeSet<String> = matrix.providers.iter().cloned().collect();
    let mut problems = Vec::new();
    for missing in code.difference(&listed) {
        problems.push(format!(
            "{MATRIX}: `{CATEGORY}.providers` lacks {missing}, which the code has: add it, \
             and a cell for it in every row"
        ));
    }
    for gone in listed.difference(code) {
        problems.push(format!(
            "{MATRIX}: `{CATEGORY}.providers` lists {gone}, which the code does not have"
        ));
    }
    problems
}

// `claude-code` is `ClaudeCode` in the code.
fn variant(provider: &str) -> String {
    provider
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

// The `src` of every crate under `crates/` and of the app.
fn source_dirs() -> Vec<PathBuf> {
    let mut crates: Vec<_> = std::fs::read_dir(root().join("crates"))
        .expect("crates reads")
        .map(|e| e.expect("an entry").path().join("src"))
        .filter(|src| src.is_dir())
        .collect();
    crates.sort();
    crates.push(root().join("src-tauri/src"));
    crates
}

// Every `.rs` file the check reads: not a test file, not this module, and not
// a harness's own adapter, whose branches only tell its calls from another's.
fn source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in source_dirs() {
        collect_sources(&dir, &mut files);
    }
    files
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("src reads")
        .map(|e| e.expect("an entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name != "tests" && !is_stand_ins(&path) {
                collect_sources(&path, out);
            }
        } else if name.ends_with(".rs")
            && name != "tests.rs"
            && !name.ends_with("_tests.rs")
            && !is_stand_ins(&path)
            && !path.starts_with(root().join("crates/kelpie-engine/src/parity"))
            && !path.starts_with(root().join("crates/kelpie-engine/src/harness"))
        {
            out.push(path);
        }
    }
}

// Whether `path` is a crate's `src/test` or `src/test.rs`.
fn is_stand_ins(path: &Path) -> bool {
    let in_src = path
        .parent()
        .is_some_and(|dir| dir.file_name().is_some_and(|n| n == "src"));
    in_src && path.file_stem().is_some_and(|stem| stem == "test")
}

// A file's lines, its inline test modules blanked so line numbers hold.
fn blank_test_modules(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut i = 0;
    while i < lines.len() {
        let opens_tests = lines[i].trim() == "#[cfg(test)]"
            && lines
                .get(i + 1)
                .is_some_and(|l| l.trim_start().starts_with("mod ") && l.ends_with('{'));
        if !opens_tests {
            i += 1;
            continue;
        }
        let mut depth = 0_i32;
        for line in &mut lines[i..] {
            depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            let closed = depth == 0 && line.contains('}');
            line.clear();
            i += 1;
            if closed {
                break;
            }
        }
    }
    lines
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .display()
        .to_string()
}

// The rows a `parity:` comment on `line` names.
fn tags(line: &str) -> Vec<String> {
    let Some((_, rest)) = line.split_once("parity:") else {
        return Vec::new();
    };
    rest.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|t| t.contains('.'))
        .map(|t| t.trim_end_matches(['.', ';']).to_owned())
        .collect()
}

// Whether `line` names one of the harnesses, as `Harness::ClaudeCode`.
fn names_provider(line: &str, providers: &[String]) -> bool {
    providers.iter().any(|p| {
        let whole = format!("Harness::{}", variant(p));
        line.match_indices(&whole).any(|(at, _)| {
            let after = line[at + whole.len()..].chars().next();
            !after.is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    })
}

fn branches(line: &str) -> bool {
    ["==", "!=", "matches!", "if let"]
        .iter()
        .any(|b| line.contains(b))
        || (line.contains("let ") && line.contains(" else"))
}

// Each branch on some of the harnesses in one file's `lines` that does not name
// its row, and each `parity:` tag that names no row.
fn branch_problems(matrix: &Matrix, file: &str, lines: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let from = if code.trim_start().starts_with("_ =>") {
            let from = i.saturating_sub(ARM_REACH);
            if !lines[from..i]
                .iter()
                .any(|l| names_provider(l, &matrix.providers))
            {
                continue;
            }
            from.saturating_sub(TAG_REACH)
        } else if names_provider(code, &matrix.providers) && branches(code) {
            i.saturating_sub(TAG_REACH)
        } else {
            continue;
        };
        let place = format!("{file}:{}", i + 1);
        let named: Vec<String> = lines[from..=i].iter().flat_map(|l| tags(l)).collect();
        for tag in named.iter().filter(|t| !matrix.rows.contains(*t)) {
            problems.push(format!("{place}: `parity: {tag}` names no row in {MATRIX}"));
        }
        if !named.iter().any(|t| t.starts_with("harness.")) {
            problems.push(format!(
                "{place} branches on some harnesses without naming the feature: add \
                 `// parity: {CATEGORY}.<row>` on that line or the {TAG_REACH} above it, and \
                 the row to {MATRIX} with a cell for every harness"
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests;
