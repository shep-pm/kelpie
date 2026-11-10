use std::path::PathBuf;

use agent_client_protocol::v1::schema::{ToolCallUpdate, ToolCallUpdateFields, ToolKind};
use serde_json::json;

use super::*;
use crate::harness::{claude, goose};
use crate::worktree::tests::process_tree;

struct World {
    _dir: tempfile::TempDir,
    worktree: PathBuf,
}

/// A worktree in a folder of its own, canonical from the start since macOS's temporary folder is a link
fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let worktree = dir.path().canonicalize().unwrap().join("wt");
    std::fs::create_dir_all(worktree.join("src")).unwrap();
    World {
        _dir: dir,
        worktree,
    }
}

impl World {
    fn judge(&self) -> Judge {
        Judge::new(&self.worktree, vec![self.worktree.clone()])
    }

    /// A judge that reads shell commands against a project whose base branch is `main`
    fn judge_with_project(&self) -> Judge {
        let tree = Arc::new(process_tree(self._dir.path()));
        let project = Project::new(
            tree,
            self._dir.path().join("repo"),
            "main",
            Some(PathBuf::from("/home/me")),
            LocalPaths::new([Path::new("/home/me")]),
        );
        self.judge().with_project(project)
    }
}

fn shell(command: &str) -> Call {
    Call::Shell {
        command: command.to_owned(),
    }
}

fn write(path: impl Into<PathBuf>) -> Call {
    Call::Write { path: path.into() }
}

#[test]
fn a_write_inside_the_worktree_is_allowed() {
    let w = world();

    for path in [w.worktree.join("src/lib.rs"), "src/new/deep.rs".into()] {
        assert_eq!(w.judge().judge(&write(&path)), Verdict::Allow, "{path:?}");
    }
}

#[test]
fn a_write_to_a_harnesss_own_file_is_refused() {
    let w = world();

    let Verdict::Refuse(why) = w.judge().judge(&write(".claude/settings.local.json")) else {
        panic!("a write to Claude Code's settings went ahead");
    };

    assert!(why.contains("Claude Code's own configuration"), "{why}");
}

#[test]
fn a_write_outside_the_worktree_is_refused_naming_it() {
    let w = world();

    let Verdict::Refuse(why) = w.judge().judge(&write("/etc/hosts")) else {
        panic!("a write outside the worktree went ahead");
    };

    assert!(why.contains("/etc/hosts is outside the folders"), "{why}");
}

#[test]
fn a_shell_call_is_refused_when_the_judge_has_no_project() {
    let w = world();

    let Verdict::Refuse(why) = w.judge().judge(&shell("ls")) else {
        panic!("a shell command went ahead");
    };

    assert!(why.contains("no project"), "{why}");
}

#[test]
fn a_shell_call_is_judged_by_guard() {
    let w = world();

    assert_eq!(
        w.judge_with_project().judge(&shell("ls -l")),
        Verdict::Allow
    );
    let Verdict::Refuse(why) = w.judge_with_project().judge(&shell("pkill node")) else {
        panic!("a process was stopped by name");
    };
    assert!(why.contains("by name or pattern"), "{why}");
}

#[test]
fn a_gh_command_is_refused_until_the_github_slice() {
    let w = world();

    for command in [
        "gh pr create --title 'fix: x'",
        "env GH_PAGER= gh pr view 3",
    ] {
        let Verdict::Refuse(why) = w.judge_with_project().judge(&shell(command)) else {
            panic!("{command} went ahead");
        };
        assert!(why.contains("no `gh` command"), "{command}: {why}");
    }
}

#[test]
fn a_commit_adding_the_home_folder_is_refused_in_the_handler() {
    let w = world();
    let Verdict::Refuse(why) = w
        .judge_with_project()
        .judge(&shell("git commit -m 'docs: see /home/me/x'"))
    else {
        panic!("a message naming the home folder went ahead");
    };

    assert!(why.contains("a path on this machine"), "{why}");
}

#[test]
fn a_write_that_names_no_file_is_refused() {
    let w = world();

    let verdict = w.judge().judge(&write(""));

    assert!(matches!(verdict, Verdict::Refuse(_)), "{verdict:?}");
}

#[test]
fn a_call_that_is_neither_a_write_nor_a_shell_goes_ahead() {
    let w = world();

    assert_eq!(w.judge().judge(&Call::Other), Verdict::Allow);
}

/// A tool call as a bridge sends it in a permission request
fn tool(kind: ToolKind, raw_input: serde_json::Value) -> ToolCallUpdate {
    ToolCallUpdate::new(
        "t1",
        ToolCallUpdateFields::new().kind(kind).raw_input(raw_input),
    )
}

#[test]
fn claude_code_names_the_file_it_writes_as_file_path_or_notebook_path() {
    for (input, path) in [
        (json!({"file_path": "/wt/a.rs", "content": "x"}), "/wt/a.rs"),
        (json!({"notebook_path": "/wt/n.ipynb"}), "/wt/n.ipynb"),
    ] {
        assert_eq!(claude::read_call(&tool(ToolKind::Edit, input)), write(path));
    }
}

#[test]
fn claude_code_names_the_command_it_runs() {
    let call = claude::read_call(&tool(ToolKind::Execute, json!({"command": "ls -la"})));

    assert_eq!(
        call,
        Call::Shell {
            command: "ls -la".to_owned()
        }
    );
}

#[test]
fn goose_names_the_file_it_writes_as_path() {
    let call = goose::read_call(&tool(
        ToolKind::Edit,
        json!({"command": "write", "path": "/wt/a.rs", "file_text": "x"}),
    ));

    assert_eq!(call, write("/wt/a.rs"));
}

#[test]
fn a_write_names_its_file_by_the_locations_when_the_input_does_not() {
    let mut update = tool(ToolKind::Edit, json!({"unexpected": true}));
    update.fields.locations = Some(vec![
        agent_client_protocol::v1::schema::ToolCallLocation::new("/wt/b.rs"),
    ]);

    assert_eq!(goose::read_call(&update), write("/wt/b.rs"));
    assert_eq!(claude::read_call(&update), write("/wt/b.rs"));
}

#[test]
fn a_write_whose_file_cannot_be_read_names_none() {
    for update in [
        tool(ToolKind::Edit, json!({"unexpected": true})),
        tool(ToolKind::Delete, json!(null)),
    ] {
        assert_eq!(claude::read_call(&update), write(""));
    }
}

#[test]
fn a_write_that_names_two_files_names_none() {
    let mut update = tool(ToolKind::Move, json!({"path": "/wt/a.rs"}));
    update.fields.locations = Some(vec![
        agent_client_protocol::v1::schema::ToolCallLocation::new("/wt/a.rs"),
        agent_client_protocol::v1::schema::ToolCallLocation::new("/outside/a.rs"),
    ]);

    assert_eq!(goose::read_call(&update), write(""));
}

#[test]
fn a_read_or_a_search_is_neither_a_write_nor_a_shell() {
    for kind in [
        ToolKind::Read,
        ToolKind::Search,
        ToolKind::Think,
        ToolKind::Fetch,
    ] {
        assert_eq!(
            claude::read_call(&tool(kind, json!({"file_path": "/wt/a.rs"}))),
            Call::Other
        );
    }
}

#[test]
fn a_call_of_no_kind_or_kind_other_that_names_a_file_is_a_write() {
    let kindless = |input| ToolCallUpdate::new("t1", ToolCallUpdateFields::new().raw_input(input));
    for (update, reader) in [
        (
            kindless(json!({"path": "/etc/hosts"})),
            goose::read_call as fn(&ToolCallUpdate) -> Call,
        ),
        (
            kindless(json!({"file_path": "/etc/hosts"})),
            claude::read_call,
        ),
        (
            tool(ToolKind::Other, json!({"path": "/etc/hosts"})),
            goose::read_call,
        ),
        (
            tool(ToolKind::Other, json!({"file_path": "/etc/hosts"})),
            claude::read_call,
        ),
    ] {
        assert_eq!(reader(&update), write("/etc/hosts"));
    }

    let mut located = tool(ToolKind::Other, json!(null));
    located.fields.locations = Some(vec![
        agent_client_protocol::v1::schema::ToolCallLocation::new("/etc/hosts"),
    ]);
    assert_eq!(goose::read_call(&located), write("/etc/hosts"));
}

#[test]
fn a_call_of_kind_other_that_names_a_command_is_a_shell_call() {
    let call = claude::read_call(&tool(ToolKind::Other, json!({"command": "rm -rf /"})));

    assert!(matches!(call, Call::Shell { .. }), "{call:?}");
}

#[test]
fn a_call_of_kind_other_that_names_no_file_goes_ahead() {
    // A tool of kelpie's own MCP server, say
    let call = claude::read_call(&tool(ToolKind::Other, json!({"title": "x"})));

    assert_eq!(call, Call::Other);
}
