//! Ways round the judge that review found: each is refused, or answered, here

use super::*;

const FETCHED: &str = "kelpie fetches `origin/main`";

// A helper a worker writes runs under `git --exec-path=<dir>`, and the guard never reads it.
#[test]
fn git_given_another_exec_path_is_refused() {
    let tree = WorkerTree::new();
    for command in [
        "git --exec-path=d ls-remote origin",
        "git --exec-path=d status",
        "env git --exec-path=/x log",
    ] {
        let why = refusal(tree.bash(command));
        assert!(why.contains("--exec-path"), "{command}: {why}");
    }
    for command in ["git --exec-path", "git status"] {
        assert_eq!(tree.bash(command), Verdict::Allow, "{command}");
    }
}

// Renaming a section or editing the file writes keys the safe list never saw.
#[test]
fn config_renamed_or_edited_into_an_unsafe_section_is_refused() {
    let tree = WorkerTree::new();
    for command in [
        "git config --rename-section color core",
        "git config --global --rename-section color core",
        "git config --rename color core",
        "git config --edit",
        "git config -e",
        "git config --global --edit",
        "git config --ed",
        "git config edit",
    ] {
        let why = refusal(tree.bash(command));
        assert!(why.contains("config"), "{command}: {why}");
    }
    for command in [
        "git config --rename-section color user",
        "git config --remove-section color",
        "git config --get core.pager",
        "git config --list",
    ] {
        assert_eq!(tree.bash(command), Verdict::Allow, "{command}");
    }
}

// With `push.default=upstream` a bare push goes to the branch's upstream, which
// `git branch -u origin/main` makes the base branch.
#[test]
fn a_push_that_goes_to_the_upstream_is_refused() {
    for value in ["upstream", "tracking", "UPSTREAM"] {
        let tree = WorkerTree::new();
        tree.git(&["config", "push.default", value]);
        let why = refusal(tree.bash("git push"));
        assert!(why.contains("push.default"), "{value}: {why}");
    }
    let tree = WorkerTree::new();
    tree.git(&["config", "push.default", "current"]);
    assert_eq!(tree.bash("git push origin HEAD"), Verdict::Allow);
}

// An annotated tag's own message goes to the forge with it.
#[test]
fn a_pushed_tag_whose_message_names_the_home_folder_is_refused() {
    let tree = WorkerTree::new();
    tree.git(&["tag", "-a", "v1", "-m", "see /home/me/x"]);
    let why = refusal(tree.bash("git push origin v1"));
    assert!(why.contains("tag"), "{why}");
    assert!(!why.contains(HOME), "{why}");

    tree.git(&["tag", "-a", "v2", "-m", "a clean one"]);
    tree.git(&["tag", "v3"]);
    for command in ["git push origin v2", "git push origin refs/tags/v3"] {
        assert_eq!(tree.bash(command), Verdict::Allow, "{command}");
    }
}

#[test]
fn a_tag_of_a_tag_is_read_through_to_every_message() {
    let tree = WorkerTree::new();
    tree.git(&["tag", "-a", "inner", "-m", "see /home/me/x"]);
    tree.git(&["tag", "-a", "outer", "-m", "clean", "inner"]);
    let why = refusal(tree.bash("git push origin outer"));
    assert!(why.contains("the message of a tag"), "{why}");
}

// A tag message file the same call writes is not there when the guard reads
// the command, and `--mess` is an abbreviation git takes.
#[test]
fn a_tag_message_the_command_names_in_a_cut_short_option_is_read() {
    let tree = WorkerTree::new();
    tree.write("m", "from /home/me/x\n");
    for command in [
        "git tag -a v1 --mess='from /home/me/x'",
        "git tag -a v1 --mess 'from /home/me/x'",
        "git tag -a v1 --fil=m",
        "git commit --allow-empty --mes='fix: /home/me/x'",
    ] {
        let why = refusal(tree.bash(command));
        assert!(why.contains("a path on this machine"), "{command}: {why}");
    }
    refusal(
        tree.bash("printf 'see /home/me/x\\n' > m2 && git tag -a v1 -F m2 && git push origin v1"),
    );
}

// `git commit <path>` commits the path's working-tree content, staged or not.
#[test]
fn a_commit_of_named_paths_reads_their_unstaged_content() {
    let tree = WorkerTree::new();
    tree.write("README.md", "see /home/me/x\n");
    tree.write("other.md", "clean\n");
    for command in [
        "git commit -m 'docs: x' README.md",
        "git commit -m 'docs: x' -- README.md",
        "git commit -o README.md -m 'docs: x'",
        "git commit -i README.md -m 'docs: x'",
        "git commit README.md -m 'docs: x'",
        "git commit -qm 'docs: x' README.md",
        "git commit --pathspec-from-file=list -m 'docs: x'",
    ] {
        let why = refusal(tree.bash(command));
        assert!(why.contains("`README.md`"), "{command}: {why}");
    }
    for command in [
        "git commit -m 'docs: x' other.md",
        "git commit -m 'docs: x'",
        "git commit --allow-empty -m 'docs: x'",
    ] {
        assert_eq!(tree.bash(command), Verdict::Allow, "{command}");
    }
    // Paths are the folder's, which the read cannot tell from the root's.
    tree.write("sub/keep.md", "x\n");
    let why = refusal(tree.bash("cd sub && git commit -m 'docs: x' ../README.md"));
    assert!(why.contains("`README.md`"), "{why}");
}

// The clone's repo and its other worktrees share the refs a fetch writes.
#[test]
fn a_fetch_from_the_clone_or_another_of_its_worktrees_is_answered() {
    let tree = WorkerTree::new();
    git(
        &tree.repo(),
        &["worktree", "add", "--quiet", "-b", "kelpie/8", "../wt2"],
    );
    let other = tree.dir.path().join("wt2");
    for cwd in [tree.repo(), other] {
        for command in ["git fetch", "git pull"] {
            let why = refusal(tree.bash_in(&cwd, command));
            assert!(why.contains(FETCHED), "{cwd:?} {command}: {why}");
        }
    }
}
