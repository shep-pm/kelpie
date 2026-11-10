//! Confine: holds a worker's file writes to the folders it may write
//!
//! A sandbox covers what a shell command writes, and a harness's own file tools
//! write wherever the harness's process may. So each file a tool call writes is
//! judged before the call runs: it must resolve inside the folders, following
//! links, and must not be one of a harness's own files ([`crate::sandbox::fence`]).
//! A path that cannot be resolved is refused.
//!
//! Lifted from SK's `core/confine/` (`docs/lifted.md`). SK ran this as a hook
//! that read each harness's tool JSON on stdin. Kelpie calls it in process from
//! the ACP permission handler ([`crate::permission`]), where each adapter has
//! already read its own tool input into the paths here.
//!
//! # Quick start
//!
//! ```
//! use std::path::PathBuf;
//!
//! use kelpie_engine::confine::{Verdict, judge};
//!
//! let worktree = PathBuf::from("/work/ticket-1");
//! let folders = [worktree.clone()];
//!
//! let inside = judge(&worktree, &[PathBuf::from("src/lib.rs")], &folders);
//! assert_eq!(inside, Verdict::Allow);
//!
//! let outside = judge(&worktree, &[PathBuf::from("/etc/hosts")], &folders);
//! assert!(matches!(outside, Verdict::Refuse(_)));
//! ```

use std::path::{Component, Path, PathBuf};

use crate::sandbox::fence;

/// What the judge says of a call
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The call goes ahead
    Allow,
    /// The call is refused, and the worker is told why
    Refuse(String),
}

/// Judges the files a call writes against `folders`
///
/// A path that is not absolute is read from `cwd`. Every path must pass: the
/// first one that does not gives the verdict, so a call that writes many files,
/// such as a patch, is refused whole.
#[must_use]
pub fn judge(cwd: &Path, targets: &[PathBuf], folders: &[PathBuf]) -> Verdict {
    targets
        .iter()
        .map(|target| judge_path(&cwd.join(target), folders))
        .find(|verdict| *verdict != Verdict::Allow)
        .unwrap_or(Verdict::Allow)
}

/// Whether a write to `target` stays in `folders` and off every harness's own files
#[must_use]
pub fn judge_path(target: &Path, folders: &[PathBuf]) -> Verdict {
    let canonical: Vec<PathBuf> = folders
        .iter()
        .map(|f| std::fs::canonicalize(f).unwrap_or_else(|_| f.clone()))
        .collect();
    let within = |path: &Path| -> Vec<PathBuf> {
        folders
            .iter()
            .chain(&canonical)
            .filter_map(|f| path.strip_prefix(f).ok().map(Path::to_owned))
            .collect()
    };
    let Some(resolved) = resolve(target).filter(|r| canonical.iter().any(|f| r.starts_with(f)))
    else {
        let allowed: Vec<_> = folders.iter().map(|f| f.display().to_string()).collect();
        return Verdict::Refuse(format!(
            "{} is outside the folders this worker may write: {}",
            target.display(),
            allowed.join(", ")
        ));
    };
    // As written and as resolved, so a link to or from `.claude` changes nothing
    if let Some(harness) = within(target)
        .iter()
        .chain(&within(&resolved))
        .find_map(|p| fence::owner(p))
    {
        return Verdict::Refuse(format!(
            "{} is {harness}'s own configuration, which only the maintainer changes",
            target.display()
        ));
    }
    Verdict::Allow
}

/// The deepest part of `path` that exists, with links followed, and the rest
/// joined on
///
/// `None` when the rest climbs with `..`.
#[must_use]
pub fn resolve(path: &Path) -> Option<PathBuf> {
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            let mut resolved = real;
            for part in rest.iter().rev() {
                resolved.push(part);
            }
            return Some(resolved);
        }
        let name = existing.file_name()?;
        rest.push(name.to_owned());
        existing = existing.parent()?;
        if matches!(
            existing.components().next_back(),
            Some(Component::ParentDir)
        ) {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use super::*;

    struct World {
        _dir: tempfile::TempDir,
        root: PathBuf,
    }

    // Canonical from the start, since macOS's temporary folder is a symlink
    fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for folder in ["wt/src", "target", "home"] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        World { _dir: dir, root }
    }

    impl World {
        fn folders(&self) -> Vec<PathBuf> {
            vec![self.root.join("wt"), self.root.join("target")]
        }

        /// A write to `path`, from the worktree, judged against the worktree and its build folder
        fn judge(&self, path: impl Into<PathBuf>) -> Verdict {
            judge(&self.root.join("wt"), &[path.into()], &self.folders())
        }

        fn path(&self, p: &str) -> PathBuf {
            self.root.join(p)
        }
    }

    #[test]
    fn a_write_inside_a_folder_goes_ahead() {
        let w = world();
        for p in ["wt/src/lib.rs", "wt/new/deep/file.rs", "target/x"] {
            assert_eq!(w.judge(w.path(p)), Verdict::Allow, "{p}");
        }
        assert_eq!(w.judge("src/relative.rs"), Verdict::Allow);
    }

    #[test]
    fn a_file_and_a_folder_named_hold_the_writes_to_exactly_those() {
        let w = world();
        // A file and a folder: the glossary, and the folder of ADRs
        let only = [w.root.join("wt/GLOSSARY.md"), w.root.join("wt/docs/adr")];
        let judge_one = |p: &str| judge(&w.root.join("wt"), &[w.path(p)], &only);
        for p in [
            "wt/GLOSSARY.md",
            "wt/docs/adr/0001-x.md",
            "wt/docs/adr/a/b.md",
        ] {
            assert_eq!(judge_one(p), Verdict::Allow, "{p}");
        }
        for p in [
            "wt/src/lib.rs",
            "wt/GLOSSARY.md.bak",
            "wt/GLOSSARY-MAP.md",
            "wt/docs/guide.md",
            "wt/docs/adr/../guide.md",
            "wt/docs/adrs/0001.md",
            "target/x",
        ] {
            let verdict = judge_one(p);
            assert!(matches!(verdict, Verdict::Refuse(_)), "{p}: {verdict:?}");
        }
    }

    #[test]
    fn a_write_outside_is_refused_naming_the_folders() {
        let w = world();
        let Verdict::Refuse(why) = w.judge(w.path("home/.zshrc")) else {
            panic!("a write to the home folder went ahead");
        };
        assert!(why.contains("home/.zshrc is outside the folders"), "{why}");
        assert!(
            why.ends_with(&format!(
                "{}, {}",
                w.path("wt").display(),
                w.path("target").display()
            )),
            "{why}"
        );
    }

    #[test]
    fn climbing_out_with_dots_is_refused() {
        let w = world();
        for p in ["wt/../home/me", "wt/missing/../../home/me"] {
            let verdict = w.judge(w.path(p));
            assert!(matches!(verdict, Verdict::Refuse(_)), "{p}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_worktree_is_followed_and_refused() {
        let w = world();
        symlink(w.root.join("home"), w.root.join("wt/escape")).unwrap();
        let verdict = w.judge(w.path("wt/escape/.zshrc"));
        assert!(matches!(verdict, Verdict::Refuse(_)), "{verdict:?}");
    }

    // A reviewer's `write`: the tests folder named twice, and the file's absolute path
    #[cfg(unix)]
    #[test]
    fn a_write_through_a_link_in_the_tests_folder_to_the_worktree_is_refused() {
        let w = world();
        let tests = w.root.join("target/review-tests");
        std::fs::create_dir_all(&tests).unwrap();
        symlink(w.root.join("wt"), tests.join("code")).unwrap();
        let reviewer = |path: &str| {
            judge(
                &w.root.join("wt"),
                &[w.path(path)],
                &[tests.clone(), tests.clone()],
            )
        };

        assert_eq!(
            reviewer("target/review-tests/tests/flag.rs"),
            Verdict::Allow
        );
        for path in [
            "target/review-tests/code/src/lib.rs",
            "target/review-tests/code/tests/flag.rs",
        ] {
            let verdict = reviewer(path);
            assert!(matches!(verdict, Verdict::Refuse(_)), "{path}: {verdict:?}");
        }
    }

    #[test]
    fn claude_codes_own_files_are_refused_to_write_and_edit() {
        let w = world();
        std::fs::create_dir_all(w.root.join("wt/.claude")).unwrap();
        for p in [
            "wt/.claude/settings.local.json",
            "wt/.claude/settings.json",
            "wt/.claude/agents/escape.md",
            "wt/.CLAUDE/settings.json",
            "wt/src/.claude/skills/x/SKILL.md",
            "wt/.mcp.json",
            "wt/.MCP.json",
            "wt/.mcp.j\u{17f}on",
            "target/.claude/settings.json",
        ] {
            let Verdict::Refuse(why) = w.judge(w.path(p)) else {
                panic!("a write to {p} went ahead");
            };
            assert!(why.contains("Claude Code's own configuration"), "{why}");
        }
        assert_eq!(
            w.judge(".claude/settings.local.json"),
            w.judge(w.path("wt/.claude/settings.local.json")),
        );
        for p in ["wt/src/mcp.json", "wt/src/.mcp.json", "wt/claude/notes.md"] {
            assert_eq!(w.judge(w.path(p)), Verdict::Allow, "{p}");
        }
    }

    #[test]
    fn codexs_own_files_are_refused_to_write_and_edit() {
        let w = world();
        for p in [
            "wt/.codex/config.toml",
            "wt/.CODEX/config.toml",
            "wt/src/.Codex/config.toml",
            "wt/.co\u{301}dex/config.toml",
        ] {
            let Verdict::Refuse(why) = w.judge(w.path(p)) else {
                panic!("a write to {p} went ahead");
            };
            assert!(why.contains("Codex's own configuration"), "{why}");
        }
        assert_eq!(w.judge(w.path("wt/src/codex.rs")), Verdict::Allow);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_or_from_claudes_own_folder_is_refused() {
        let w = world();
        std::fs::create_dir_all(w.root.join("wt/.claude")).unwrap();
        symlink(w.root.join("wt/.claude"), w.root.join("wt/cfg")).unwrap();
        let verdict = w.judge(w.path("wt/cfg/settings.local.json"));
        assert!(matches!(verdict, Verdict::Refuse(_)), "{verdict:?}");

        let w = world();
        symlink(w.root.join("wt/src"), w.root.join("wt/.claude")).unwrap();
        let verdict = w.judge(w.path("wt/.claude/settings.json"));
        assert!(matches!(verdict, Verdict::Refuse(_)), "{verdict:?}");
    }

    #[test]
    fn a_call_that_writes_many_files_is_refused_for_any_one_outside() {
        let w = world();
        let cwd = w.root.join("wt");

        let inside = judge(
            &cwd,
            &[w.path("target/x"), "src/new.rs".into()],
            &w.folders(),
        );
        let one_outside = judge(
            &cwd,
            &["src/ok.rs".into(), w.path("home/.zshrc")],
            &w.folders(),
        );

        assert_eq!(inside, Verdict::Allow);
        assert!(matches!(one_outside, Verdict::Refuse(_)), "{one_outside:?}");
    }

    #[test]
    fn a_call_that_writes_no_file_goes_ahead() {
        assert_eq!(
            judge(Path::new("/wt"), &[], &[PathBuf::from("/wt")]),
            Verdict::Allow
        );
    }
}
