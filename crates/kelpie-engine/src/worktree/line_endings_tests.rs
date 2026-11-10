use std::path::PathBuf;

use super::tests::{Dir, git, process_tree};
use super::{Remote, Start, prepare, trust_git_of, trusted};
use crate::tree::ProcessTree;

const SETTINGS: &[u8] = b"{\n  \"permissions\": {}\n}\n";

// A clone whose `main` holds a file with LF endings, whose `.gitattributes`
// says `text=auto` and whose own config asks for the CRLF checkouts Git for
// Windows asks for in its system config
struct Rig {
    _dir: Dir,
    tree: ProcessTree,
    repo: PathBuf,
    worktree: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let dir = Dir::new("line-endings");
        let root = dir.path().canonicalize().unwrap();
        git(
            &root,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&root, &["clone", "--quiet", "origin.git", "repo"]);
        let repo = root.join("repo");
        std::fs::write(repo.join(".gitattributes"), "* text=auto\n").unwrap();
        std::fs::create_dir(repo.join(".claude")).unwrap();
        std::fs::write(repo.join(".claude/settings.json"), SETTINGS).unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "--quiet", "-m", "init"]);
        git(&repo, &["push", "--quiet", "origin", "main"]);
        git(&repo, &["config", "core.autocrlf", "true"]);
        git(&repo, &["config", "core.eol", "crlf"]);
        let worktree = root.join("worktrees/7");
        Self {
            tree: process_tree(&root),
            _dir: dir,
            repo,
            worktree,
        }
    }

    fn prepared(&self) {
        prepare(
            &self.tree,
            &self.repo,
            &self.worktree,
            "kelpie/7",
            Start::Main,
            "main",
            &Remote::Origin,
            &self.worktree.with_file_name("build"),
        )
        .unwrap();
    }
}

#[test]
fn a_worktree_checks_a_file_out_as_committed_under_crlf_config() {
    let rig = Rig::new();
    rig.prepared();

    let checked_out = std::fs::read(rig.worktree.join(".claude/settings.json")).unwrap();
    assert_eq!(checked_out, SETTINGS);
}

#[test]
fn trusted_git_runs_with_core_autocrlf_off_and_core_eol_lf() {
    let rig = Rig::new();
    rig.prepared();

    let in_worktree = trusted(&rig.tree, &rig.repo, &rig.worktree).unwrap();
    assert_eq!(in_worktree(&["config", "core.autocrlf"]).unwrap(), "false");
    assert_eq!(in_worktree(&["config", "core.eol"]).unwrap(), "lf");
}

#[test]
fn a_program_given_the_trusted_git_runs_with_core_autocrlf_off_and_core_eol_lf() {
    let rig = Rig::new();
    rig.prepared();

    let config = |key: &str| {
        let mut program = std::process::Command::new("git");
        program.args(["config", key]);
        trust_git_of(&rig.tree, &mut program, &rig.repo, &rig.worktree).unwrap();
        let out = rig.tree.output(&mut program).unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    assert_eq!(config("core.autocrlf"), "false");
    assert_eq!(config("core.eol"), "lf");
}

#[test]
fn a_program_given_the_trusted_git_loses_the_variables_that_pick_another_index_or_objects() {
    let rig = Rig::new();
    rig.prepared();

    let mut program = std::process::Command::new("git");
    for name in [
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ] {
        program.env(name, "/elsewhere");
    }
    trust_git_of(&rig.tree, &mut program, &rig.repo, &rig.worktree).unwrap();

    let set: Vec<_> = program
        .get_envs()
        .filter(|(name, value)| name.to_string_lossy().starts_with("GIT_") && value.is_some())
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    for name in [
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_NAMESPACE",
    ] {
        assert!(!set.iter().any(|set| set == name), "{name} is still set");
    }
    assert!(set.iter().any(|set| set == "GIT_DIR"));
}
