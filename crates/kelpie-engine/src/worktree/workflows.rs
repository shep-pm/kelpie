//! The workflows `origin/main` holds

use std::path::Path;

use super::{WorktreeError, git};
use crate::tree::ProcessTree;

const FOLDER: &str = ".github/workflows";

/// Whether any workflow `origin/main` holds runs on a pull request, as the
/// base was last fetched
///
/// Reads the base and not the branch, so a branch cannot switch CI off by
/// deleting a workflow. A base with no workflows folder has none.
///
/// # Errors
///
/// [`WorktreeError`] naming the git command that failed.
pub fn runs_on_pull_requests(
    tree: &ProcessTree,
    repo: &Path,
    base: &str,
) -> Result<bool, WorktreeError> {
    let tip = format!("origin/{base}");
    let listed = git(
        tree,
        repo,
        ["ls-tree", "--name-only", &tip, "--", &format!("{FOLDER}/")],
    )?;
    for path in listed.lines().filter(|path| is_workflow_file(path)) {
        let text = git(tree, repo, ["show", &format!("{tip}:{path}")])?;
        if runs_on_a_pull_request(&text, base) {
            return Ok(true);
        }
    }
    Ok(false)
}

// The workflow is read as text, in the forms GitHub's documentation shows,
// with no YAML parser: SK's reading, whole.

fn is_workflow_file(name: &str) -> bool {
    name.ends_with(".yml") || name.ends_with(".yaml")
}

// Whether a workflow's checks show on a work branch's pull request: a pull
// request event runs it, or a push that reaches a work branch does, `base`
// being the branch the pull request is against.
fn runs_on_a_pull_request(text: &str, base: &str) -> bool {
    let text = without_comments(text);
    let names_event = text
        .replace("github.event.pull_request", "")
        .contains("pull_request");
    names_event || runs_on_push(&text, base)
}

// Whether a push to a work branch runs a workflow, by the filters under its
// `push:` key. GitHub runs a push that sets only `tags` or `tags-ignore` on
// tags alone. With `branches`, a work branch is run when the list matches more
// than `base`. With `branches-ignore`, it is run unless a pattern ignores it.
fn runs_on_push(text: &str, base: &str) -> bool {
    let triggers = text.split("\njobs:").next().unwrap_or("");
    let words = || triggers.split(|c: char| !(c.is_alphanumeric() || c == '_'));
    if !words().any(|word| word == "push") {
        return false;
    }
    let list = |key| push_list(triggers, key);
    if let Some(branches) = list("branches") {
        branches.iter().any(|branch| branch != base)
    } else if let Some(ignored) = list("branches-ignore") {
        // Which work branch is not known here: one named by a ticket's id.
        let sample = format!("{}1", crate::worker::start::BRANCH_PREFIX);
        !ignored.iter().any(|pattern| glob_matches(pattern, &sample))
    } else {
        list("tags").is_none() && list("tags-ignore").is_none()
    }
}

// Whether a branch filter's `pattern` matches `name`: `*` matches within a
// path segment, `**` across them, and the other characters are themselves.
fn glob_matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_prefix('*') {
        Some(rest) => {
            let (rest, across) = rest.strip_prefix('*').map_or((rest, false), |r| (r, true));
            (0..=name.len())
                .filter(|&at| name.is_char_boundary(at))
                .take_while(|&at| across || !name[..at].contains('/'))
                .any(|at| glob_matches(rest, &name[at..]))
        }
        None => match (pattern.chars().next(), name.chars().next()) {
            (Some(p), Some(n)) if p == n => {
                glob_matches(&pattern[p.len_utf8()..], &name[n.len_utf8()..])
            }
            (None, None) => true,
            _ => false,
        },
    }
}

// The list under `key` in the `push:` key, when it has one.
fn push_list(triggers: &str, key: &str) -> Option<Vec<String>> {
    let key = format!("{key}:");
    let lines: Vec<&str> = triggers.lines().collect();
    let at = lines.iter().position(|l| l.trim().starts_with("push:"))?;
    let base = indent(lines[at]);
    let inline = lines[at].trim().trim_start_matches("push:");
    let block = lines[at + 1..]
        .iter()
        .take_while(|l| l.trim().is_empty() || indent(l) > base);
    let mut found = None;
    let mut listing = false;
    for line in std::iter::once(&inline).chain(block) {
        let trimmed = line.trim().trim_start_matches('{').trim_start();
        if let Some(rest) = trimmed.strip_prefix(key.as_str()) {
            listing = rest.trim().is_empty();
            found.get_or_insert_with(Vec::new).extend(entries(rest));
        } else if listing && trimmed.starts_with('-') {
            found.get_or_insert_with(Vec::new).extend(entries(trimmed));
        } else if !trimmed.is_empty() {
            listing = false;
        }
    }
    found
}

// The names in a list, inline or as one `- name` line.
fn entries(text: &str) -> Vec<String> {
    let text = text.trim();
    let text = text.strip_prefix('-').unwrap_or(text);
    text.split(|c: char| c.is_whitespace() || "[]{},'\"".contains(c))
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn without_comments(text: &str) -> String {
    let lines = text.lines().map(|l| l.split('#').next().unwrap_or(""));
    lines.collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::tests::{Dir, git, process_tree, write_in};

    const PUSH_MAIN: &str = "on:\n  push:\n    branches: [main]\njobs:\n  t:\n";
    const PULL_REQUEST: &str = "on: pull_request\njobs:\n  t:\n";

    // A repo whose `origin/main` holds `files` in the workflows folder
    fn repo_with(dir: &Dir, files: &[(&str, &str)]) -> std::path::PathBuf {
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet", "-b", "main"]);
        write_in(&repo, "README.md", "a project\n");
        for (name, text) in files {
            write_in(&repo, &format!("{FOLDER}/{name}"), text);
        }
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "first"]);
        git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        repo
    }

    fn runs(files: &[(&str, &str)], base: &str) -> Result<bool, WorktreeError> {
        let dir = Dir::new("workflows");
        let repo = repo_with(&dir, files);
        runs_on_pull_requests(&process_tree(dir.path()), &repo, base)
    }

    #[test]
    fn a_base_runs_on_pull_requests_when_one_workflow_does() {
        let runs = runs(
            &[("release.yml", PUSH_MAIN), ("ci.yaml", PULL_REQUEST)],
            "main",
        );
        assert_eq!(runs, Ok(true));
    }

    #[test]
    fn a_base_whose_workflows_run_on_main_alone_does_not() {
        let runs = runs(
            &[("release.yml", PUSH_MAIN), ("notes.md", PULL_REQUEST)],
            "main",
        );
        assert_eq!(runs, Ok(false));
    }

    #[test]
    fn a_base_with_no_workflows_folder_does_not() {
        assert_eq!(runs(&[], "main"), Ok(false));
    }

    #[test]
    fn a_base_that_is_not_fetched_is_an_error() {
        assert!(runs(&[("ci.yml", PULL_REQUEST)], "trunk").is_err());
    }

    #[test]
    fn a_push_to_any_branch_but_the_base_counts() {
        let on_push =
            "on:\n  push:\n    branches:\n      - main\n      - 'kelpie/**'\njobs:\n  t:\n";
        assert_eq!(runs(&[("ci.yml", on_push)], "main"), Ok(true));
    }

    #[test]
    fn a_comment_naming_a_pull_request_does_not_count() {
        let commented = "on:\n  push:\n    branches: [main]\n# pull_request later\njobs:\n  t:\n";
        assert_eq!(runs(&[("ci.yml", commented)], "main"), Ok(false));
    }

    #[test]
    fn a_push_filtered_by_tags_alone_does_not_count() {
        let release = "on:\n  push:\n    tags: ['v*']\njobs:\n  t:\n";
        let inline = "on:\n  push: { tags-ignore: [wip] }\njobs:\n  t:\n";
        assert_eq!(runs(&[("release.yml", release)], "main"), Ok(false));
        assert_eq!(runs(&[("release.yml", inline)], "main"), Ok(false));
    }

    #[test]
    fn a_push_ignoring_the_work_branches_does_not_count() {
        let ignoring = "on:\n  push:\n    branches-ignore: ['kelpie/**']\njobs:\n  t:\n";
        assert_eq!(runs(&[("ci.yml", ignoring)], "main"), Ok(false));
    }

    #[test]
    fn a_push_ignoring_other_branches_counts() {
        let ignoring = "on:\n  push:\n    branches-ignore:\n      - 'docs/**'\njobs:\n  t:\n";
        assert_eq!(runs(&[("ci.yml", ignoring)], "main"), Ok(true));
    }
}
