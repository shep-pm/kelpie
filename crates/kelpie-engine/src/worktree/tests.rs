//! The repos the worktree reads' tests build, with the maintainer's own git
//! config kept out of them
//!
//! Each test gets a folder of its own and a `git` that answers as a fresh
//! install would: no global or system config, no signing, and an identity a
//! commit can be made with.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::run::RunId;
use crate::tree::ProcessTree;

/// A folder of its own for one test's repo, removed when that test ends
#[derive(Debug)]
pub(crate) struct Dir(PathBuf);

impl Dir {
    /// A fresh folder under the temp dir, named with a run id so no two tests share one
    pub(crate) fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("kelpie-worktree-{name}-{}", RunId::new().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `git args` in `dir`, with the settings that keep the maintainer's own config
/// out of the test's repo: no global or system file, no signing, and an
/// identity a commit can be made with
pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_command(dir, args).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "git {args:?} in {}: {stderr}",
        dir.display()
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// `git args` in `dir`, its stdout byte for byte
pub(crate) fn git_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
    let output = git_command(dir, args).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "git {args:?} in {}: {stderr}",
        dir.display()
    );
    output.stdout
}

/// The exit code of `git args` in `dir`, `None` where it could not run at all
pub(crate) fn git_code(dir: &Path, args: &[&str]) -> Option<i32> {
    git_command(dir, args)
        .stderr(Stdio::null())
        .status()
        .ok()
        .and_then(|status| status.code())
}

/// `text` at `file` under `dir`, with the folders between made
pub(crate) fn write_in(dir: &Path, file: &str, text: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A process tree whose run file is in `dir`, for a test's git calls
///
/// The tree is what every git kelpie runs goes through (ADR 0001), so a test of
/// a read that falls back to git builds one.
pub(crate) fn process_tree(dir: &Path) -> ProcessTree {
    ProcessTree::new(RunId::new().unwrap(), dir.join("run.json")).unwrap()
}

// `git args` in `dir`, built but not run. Tests run git directly: the answer a
// test compares a read against must come from git itself.
fn git_command(dir: &Path, args: &[&str]) -> Command {
    let mut git = Command::new("git");
    git.args([
        "-c",
        "user.name=rig",
        "-c",
        "user.email=rig@example.invalid",
    ])
    .args(["-c", "commit.gpgsign=false"])
    .args(args)
    .current_dir(dir)
    .env("GIT_CONFIG_GLOBAL", "/dev/null")
    .env("GIT_CONFIG_NOSYSTEM", "1");
    git
}

#[test]
fn a_missing_ref_is_absent_but_a_folder_git_cannot_read_is_an_error() {
    let dir = Dir::new("has-ref");
    let tree = process_tree(dir.path());
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    assert!(super::has_ref(&tree, &repo, "refs/heads/kelpie/7").is_err());
    git(&repo, &["init", "--quiet"]);
    assert!(!super::has_ref(&tree, &repo, "refs/heads/kelpie/7").unwrap());
}

#[test]
fn a_status_names_every_changed_staged_renamed_and_new_path() {
    let status = "1 .M N... 100644 100644 100644 aaaa bbbb src/a b.rs\0\
                  1 AD N... 000000 100644 000000 0000 cccc staged.txt\0\
                  2 R. N... 100644 100644 100644 dddd eeee R100 new.rs\0old.rs\0\
                  u UU N... 100644 100644 100644 100644 ffff gggg hhhh clash.rs\0\
                  ? left.txt\0";
    assert_eq!(
        super::status_names(status),
        [
            "src/a b.rs",
            "staged.txt",
            "old.rs",
            "new.rs",
            "clash.rs",
            "left.txt"
        ]
    );
}
