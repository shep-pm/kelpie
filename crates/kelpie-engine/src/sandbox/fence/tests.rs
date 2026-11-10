//! The fence's tests: what each harness fences, and the check of a worktree against `main`

#[cfg(unix)]
use std::os::unix::fs::symlink;

use super::*;
use crate::sandbox;
use crate::worktree::clone;
use crate::worktree::tests::{Dir, git, process_tree};

const SETTINGS: &str = ".claude/settings.json";

// A clone whose `main` holds a settings file, and a worktree on
// `kelpie/7` cut from it, as kelpie cuts one.
struct World {
    _dir: Dir,
    root: PathBuf,
    tree: ProcessTree,
}

impl World {
    fn new() -> Self {
        let dir = Dir::new("fence");
        let root = dir.path().canonicalize().unwrap();
        let tree = process_tree(&root);
        git(
            &root,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&root, &["clone", "--quiet", "origin.git", "repo"]);
        let w = Self {
            _dir: dir,
            root,
            tree,
        };
        w.commit(&w.repo(), SETTINGS, "{}\n");
        git(&w.repo(), &["push", "--quiet", "origin", "main"]);
        git(&w.repo(), &["fetch", "--quiet", "origin"]);
        let wt = w.wt();
        clone::make(&w.tree, &w.repo(), &wt, "kelpie/7", "origin/main").unwrap();
        w
    }

    fn repo(&self) -> PathBuf {
        self.root.join("repo")
    }

    fn wt(&self) -> PathBuf {
        self.root.join("wt")
    }

    fn commit(&self, cwd: &Path, file: &str, text: &str) -> String {
        let path = cwd.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        git(cwd, &["add", file]);
        git(cwd, &["commit", "--quiet", "-m", file]);
        git(cwd, &["rev-parse", "HEAD"])
    }

    fn write(&self, file: &str, text: &str) {
        let path = self.wt().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn differ(&self, accepted: Option<&str>) -> Vec<String> {
        differ(&self.tree, &self.repo(), &self.wt(), accepted, "main").unwrap()
    }

    fn changed(&self, accepted: Option<&str>) -> Vec<String> {
        changed(
            &self.tree,
            &self.repo(),
            "kelpie/7",
            &Remote::Origin,
            accepted,
            "main",
        )
        .unwrap()
        .1
    }

    // The head of the worktree's branch, pushed and fetched into the repo as
    // `changed` fetches it. A clone's own commits are in the clone alone until
    // then, and a head the maintainer accepted is one `changed` fetched.
    fn publish(&self) -> String {
        git(&self.wt(), &["push", "--quiet", "origin", "HEAD"]);
        git(&self.repo(), &["fetch", "--quiet", "origin"]);
        git(&self.wt(), &["rev-parse", "HEAD"])
    }

    fn push_branch(&self, file: &str, text: &str) -> String {
        let head = self.commit(&self.wt(), file, text);
        git(&self.wt(), &["push", "--quiet", "origin", "HEAD"]);
        head
    }
}

#[test]
fn claude_codes_own_files_are_named_in_any_case_and_anywhere_for_dot_claude() {
    for path in [
        ".claude",
        ".claude/settings.local.json",
        "./.claude/agents/a.md",
        ".CLAUDE/settings.json",
        "src/.Claude/skills/x/SKILL.md",
        ".mcp.json",
        ".MCP.JSON",
        // U+017F, which APFS reads as `s`
        ".mcp.j\u{17f}on",
        // A combining accent, which is refused rather than read
        ".cla\u{301}ude/settings.json",
    ] {
        assert!(fenced(Path::new(path)), "{path}");
    }
    for path in [
        // A Cyrillic `а`, which no file system reads as `a`
        ".cl\u{430}ude/settings.json",
        "claude/notes.md",
        "src/.mcp.json",
        ".mcp.json.bak",
        "README.md",
        "",
    ] {
        assert!(!fenced(Path::new(path)), "{path}");
    }
}

#[test]
fn codexs_own_files_are_named_in_any_case_and_anywhere() {
    for path in [
        ".codex",
        ".codex/config.toml",
        ".CODEX/config.toml",
        "src/.Codex/config.toml",
        // A combining accent, which is refused rather than read
        ".co\u{301}dex/config.toml",
    ] {
        assert_eq!(owner(Path::new(path)), Some("Codex"), "{path}");
    }
    for path in ["codex/notes.md", ".codex.toml", "src/codex.rs"] {
        assert!(!fenced(Path::new(path)), "{path}");
    }
    assert_eq!(owner(Path::new(".mcp.json")), Some("Claude Code"));
}

#[test]
fn a_call_reads_no_other_harnesss_login() {
    assert_eq!(
        others_credentials("Claude Code"),
        [
            "~/.config/goose/**",
            "~/.local/share/goose/**",
            "~/.local/state/goose/**",
            "~/Library/Application Support/Block/goose/**",
            "~/.codex/**",
            "~/.pi/**",
        ]
    );
    assert_eq!(
        others_credentials("Codex"),
        [
            "~/.claude/.credentials.json",
            "~/.claude.json",
            "~/.config/goose/**",
            "~/.local/share/goose/**",
            "~/.local/state/goose/**",
            "~/Library/Application Support/Block/goose/**",
            "~/.pi/**",
        ]
    );
}

// The sandbox's policy keeps its own list of the logins it closes to reads
// (`sandbox::denied`), and a login the fence names that it left out would be
// readable.
#[test]
fn every_login_the_fence_names_is_denied_for_read_by_the_sandbox() {
    let denied = sandbox::denied();
    for path in credentials() {
        assert!(denied.contains(&path.as_str()), "{path}");
    }
}

#[test]
fn a_goose_folder_is_fenced_and_its_credentials_are_unreadable_to_every_other_harness() {
    for path in [
        ".goose",
        ".goose/config.yaml",
        "src/.GOOSE/x",
        "a/b/.Goose/recipes",
    ] {
        assert_eq!(owner(Path::new(path)), Some("goose"), "{path}");
    }
    for path in ["goose/notes.md", ".goosehints", "src/goose.rs"] {
        assert!(!fenced(Path::new(path)), "{path}");
    }
    assert_eq!(
        owner(Path::new(".agents/skills")),
        Some("Codex"),
        ".agents stays Codex's"
    );

    let goose = [
        "~/.config/goose/**",
        "~/.local/share/goose/**",
        "~/.local/state/goose/**",
        "~/Library/Application Support/Block/goose/**",
    ];
    for harness in ["Claude Code", "Codex", PI] {
        let unread = others_credentials(harness);
        for path in goose {
            assert!(unread.iter().any(|p| p == path), "{harness}: {path}");
        }
    }
    let own = others_credentials("goose");
    assert!(goose.iter().all(|path| !own.iter().any(|p| p == path)));
    assert!(own.iter().any(|p| p == "~/.pi/**"));
}

// goose reads `settings.local.json` and `settings.json` under `.config/goose`
// in its working folder, and a `disabledPlugins` entry there switches off the
// plugin that fences a worker.
#[test]
fn a_goose_settings_file_in_the_worktree_at_any_depth_is_fenced() {
    for path in [
        ".config/goose",
        ".config/goose/settings.local.json",
        ".config/goose/settings.json",
        "packages/app/.config/goose/settings.local.json",
        ".CONFIG/Goose/settings.json",
    ] {
        assert_eq!(owner(Path::new(path)), Some("goose"), "{path}");
    }
    // Other tools' files under `.config`, and a goose folder elsewhere, are the project's.
    for path in [
        ".config",
        ".config/other/settings.json",
        "config/goose/settings.json",
        "goose/.config/x",
        "src/.config.rs",
    ] {
        assert!(!fenced(Path::new(path)), "{path}");
    }
    let wt = Path::new("/k/wt/7");
    let deny = deny_write_on(wt, "macos");
    for path in ["/k/wt/7/.config/goose", "/k/wt/7/**/.config/goose"] {
        assert!(deny.contains(&PathBuf::from(path)), "{path}");
    }
}

#[test]
fn the_sandbox_denies_every_harnesss_files_in_the_worktree() {
    let wt = Path::new("/k/wt/7");
    let deny: Vec<String> = deny_write_on(wt, "macos")
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert_eq!(
        deny,
        [
            "/k/wt/7/.claude",
            "/k/wt/7/**/.claude",
            "/k/wt/7/.mcp.json",
            "/k/wt/7/.goose",
            "/k/wt/7/**/.goose",
            "/k/wt/7/.config/goose",
            "/k/wt/7/**/.config/goose",
            "/k/wt/7/.codex",
            "/k/wt/7/**/.codex",
            "/k/wt/7/.agents",
            "/k/wt/7/**/.agents",
            "/k/wt/7/.pi",
            "/k/wt/7/**/.pi",
        ]
    );
}

#[test]
fn the_install_fence_keeps_the_root_harness_files_and_drops_the_nested_globs() {
    let wt = Path::new("/k/wt/7");
    let deny: Vec<String> = install_deny_write(wt)
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    assert_eq!(
        deny,
        [
            "/k/wt/7/.claude",
            "/k/wt/7/.mcp.json",
            "/k/wt/7/.goose",
            "/k/wt/7/.config/goose",
            "/k/wt/7/.codex",
            "/k/wt/7/.agents",
            "/k/wt/7/.pi",
        ]
    );
}

#[test]
fn a_workers_fence_still_denies_a_nested_claude_folder() {
    let wt = Path::new("/k/wt/7");
    assert!(deny_write_on(wt, "macos").contains(&wt.join("**").join(".claude")));
}

#[test]
fn a_pi_extension_added_to_a_branch_is_named() {
    let w = World::new();
    w.push_branch(".pi/extensions/run.ts", "export default () => {};\n");
    assert_eq!(w.changed(None).len(), 1);
    assert_eq!(owner(Path::new("src/.pi/x.ts")), Some("pi"));
}

#[test]
fn a_codex_file_added_to_the_worktree_or_a_branch_is_named() {
    let w = World::new();
    w.write(".codex/config.toml", "[mcp_servers]\n");
    assert_eq!(w.differ(None), [".codex/config.toml"]);

    let w = World::new();
    w.push_branch(".Codex/config.toml", "[mcp_servers]\n");
    assert_eq!(w.changed(None).len(), 1);
}

// The Windows fence lists only what exists when the call starts, so a
// harness file or folder made during the call is caught here alone.
#[test]
fn a_harness_file_or_folder_made_during_a_call_differs() {
    for made in [
        ".agents/skills/x/SKILL.md",
        ".codex/config.toml",
        ".goose/config.yaml",
        ".pi/extensions/run.ts",
        ".config/goose/config.yaml",
        ".mcp.json",
    ] {
        let w = World::new();
        w.write(made, "made during the call\n");
        assert_eq!(w.differ(None), [made]);
    }
}

#[test]
fn goose_files_in_the_config_folder_are_compared_with_an_accepted_commit() {
    let w = World::new();
    w.commit(&w.wt(), ".config/goose/config.yaml", "extensions: {}\n");
    let accepted = w.publish();
    let file = ".config/goose/config.yaml";
    assert_eq!(w.differ(None), [file]);
    assert_eq!(w.differ(Some(&accepted)), Vec::<String>::new());
    w.write(file, "changed\n");
    assert_eq!(w.differ(Some(&accepted)), [file]);
}

#[test]
fn a_file_named_config_in_the_worktree_root_is_no_harness_file() {
    let w = World::new();
    w.write(".config", "not a folder\n");
    assert_eq!(w.differ(None), Vec::<String>::new());
    assert!(matches_main(&w.tree, &w.repo(), &w.wt(), "main"));
}

#[test]
fn a_worktree_as_main_left_it_passes() {
    let w = World::new();
    w.write("src/lib.rs", "work\n");
    assert_eq!(w.differ(None), Vec::<String>::new());
}

#[test]
fn a_file_added_changed_or_removed_differs() {
    let w = World::new();
    w.write(".claude/settings.local.json", "{}\n");
    assert_eq!(w.differ(None), [".claude/settings.local.json"]);

    let w = World::new();
    w.write(SETTINGS, "{\"hooks\":{}}\n");
    w.write(".mcp.json", "{}\n");
    assert_eq!(w.differ(None), [".claude/settings.json", ".mcp.json"]);

    let w = World::new();
    fs::remove_file(w.wt().join(SETTINGS)).unwrap();
    assert_eq!(w.differ(None), [SETTINGS]);
}

#[cfg(unix)]
#[test]
fn a_link_or_a_new_exec_bit_differs() {
    let w = World::new();
    symlink("/etc/hosts", w.wt().join(".mcp.json")).unwrap();
    assert_eq!(w.differ(None), [".mcp.json"]);

    let w = World::new();
    let path = w.wt().join(SETTINGS);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(w.differ(None), [SETTINGS]);
}

#[test]
fn a_path_on_disk_is_spelled_with_slashes_as_git_spells_it() {
    let nested = Path::new(".claude").join("hooks").join("a.sh");
    assert_eq!(git_path(&nested), ".claude/hooks/a.sh");
    assert_eq!(git_path(Path::new(".mcp.json")), ".mcp.json");
}

// Windows keeps no mode on a file, so the disk names none: see `executable_bit`.
#[test]
fn a_file_with_no_mode_on_disk_matches_either_regular_mode_and_nothing_else() {
    let on_disk = |text: &str| {
        let path = PathBuf::from("/wt").join(text);
        let disk = BTreeMap::from([(
            text.to_owned(),
            Disk::File {
                path: path.clone(),
                executable: None,
            },
        )]);
        (disk, BTreeMap::from([(path, "blob".to_owned())]))
    };
    let differ = |mode: &str, blob: &str| {
        let (disk, hashes) = on_disk("f");
        let trusted = BTreeMap::from([("f".to_owned(), (mode.to_owned(), blob.to_owned()))]);
        let dir = Dir::new("fence-compare");
        let tree = process_tree(dir.path());
        compare(&tree, Path::new("."), &disk, &hashes, &trusted).unwrap()
    };
    assert_eq!(differ("100644", "blob"), Vec::<String>::new());
    assert_eq!(differ("100755", "blob"), Vec::<String>::new());
    assert_eq!(differ("120000", "blob"), ["f"], "main has a link there");
    assert_eq!(differ("100644", "other"), ["f"], "other contents");
}

#[test]
fn a_worktree_cut_before_main_changed_its_settings_still_passes() {
    let w = World::new();
    w.commit(&w.repo(), SETTINGS, "{\"model\":\"x\"}\n");
    git(&w.repo(), &["push", "--quiet", "origin", "main"]);
    git(&w.repo(), &["fetch", "--quiet", "origin"]);
    assert_eq!(w.differ(None), Vec::<String>::new());
}

#[test]
fn a_worktree_matches_main_only_when_its_files_are_origin_mains_now() {
    let w = World::new();
    assert!(matches_main(&w.tree, &w.repo(), &w.wt(), "main"));

    // Cut before main changed its settings: it passes the check, but is not main's.
    w.commit(&w.repo(), SETTINGS, "{\"model\":\"x\"}\n");
    git(&w.repo(), &["push", "--quiet", "origin", "main"]);
    git(&w.repo(), &["fetch", "--quiet", "origin"]);
    assert!(!matches_main(&w.tree, &w.repo(), &w.wt(), "main"));

    let w = World::new();
    w.write(".claude/agents/a.md", "hi\n");
    assert!(!matches_main(&w.tree, &w.repo(), &w.wt(), "main"));

    // A folder with none of the files differs from a main that holds some.
    let bare = w.root.join("bare");
    fs::create_dir(&bare).unwrap();
    assert!(!matches_main(&w.tree, &w.repo(), &bare, "main"));
}

#[test]
fn a_second_pass_over_the_same_commits_asks_git_for_no_tree_or_ancestor() {
    let w = World::new();
    let asked = || commit::ASKED.with(std::cell::Cell::get);
    w.write(SETTINGS, "{\"other\":true}\n");
    let first = w.differ(None);
    let after_first = asked();
    assert!(after_first > 0, "the first pass reads the commits");

    assert_eq!(w.differ(None), first);
    assert!(matches_main(&w.tree, &w.repo(), &w.wt(), "main") == first.is_empty());

    assert_eq!(asked(), after_first);
}

#[test]
fn a_moved_origin_main_is_read_again() {
    let w = World::new();
    w.write(SETTINGS, "{\"moved\":true}\n");
    assert_eq!(w.differ(None), [SETTINGS]);
    assert!(!matches_main(&w.tree, &w.repo(), &w.wt(), "main"));

    w.commit(&w.repo(), SETTINGS, "{\"moved\":true}\n");
    git(&w.repo(), &["push", "--quiet", "origin", "main"]);

    assert_eq!(w.differ(None), Vec::<String>::new());
    assert!(matches_main(&w.tree, &w.repo(), &w.wt(), "main"));
}

#[test]
fn a_worktree_at_an_accepted_head_passes_and_no_other() {
    let w = World::new();
    w.commit(&w.wt(), SETTINGS, "{\"hooks\":{}}\n");
    let accepted = w.publish();
    assert_eq!(w.differ(None), [SETTINGS]);
    assert_eq!(w.differ(Some(&accepted)), Vec::<String>::new());
    w.write(".claude/agents/a.md", "hi\n");
    assert_eq!(w.differ(Some(&accepted)), [".claude/agents/a.md", SETTINGS]);
}

#[test]
fn an_accepted_head_that_spells_the_folder_in_capitals_still_passes() {
    let w = World::new();
    w.write(".claude/agents/a.md", "hi\n");
    let blob = git(&w.wt(), &["hash-object", "-w", ".claude/agents/a.md"]);
    let entry = format!("100644,{blob},.CLAUDE/agents/a.md");
    git(&w.wt(), &["update-index", "--add", "--cacheinfo", &entry]);
    git(&w.wt(), &["commit", "--quiet", "-m", "capitals"]);
    let accepted = w.publish();
    assert_eq!(w.differ(None), [".claude/agents/a.md"]);
    assert_eq!(w.differ(Some(&accepted)), Vec::<String>::new());
}

// Kelpie runs on macOS, whose APFS opens `.mcp.jſon` as `.mcp.json`.
#[cfg(target_os = "macos")]
#[test]
fn a_file_the_file_system_reads_as_mcp_json_differs() {
    let w = World::new();
    w.write(".mcp.j\u{17f}on", "{}\n");
    assert_eq!(w.differ(None), [".mcp.json"]);
}

#[test]
fn a_branch_changing_them_in_any_case_is_named_until_accepted() {
    let w = World::new();
    w.push_branch("src/lib.rs", "work\n");
    assert_eq!(w.changed(None), Vec::<String>::new());

    let accepted = w.push_branch(".CLAUDE/agents/a.md", "hi\n");
    let changed = w.changed(None);
    assert_eq!(changed.len(), 1);
    assert!(
        changed[0].eq_ignore_ascii_case(".claude/agents/a.md"),
        "{changed:?}"
    );
    assert_eq!(w.changed(Some(&accepted)), Vec::<String>::new());

    w.push_branch("src/more.rs", "work\n");
    assert_eq!(w.changed(Some(&accepted)), Vec::<String>::new());
    w.push_branch(".mcp.json", "{}\n");
    assert_eq!(w.changed(Some(&accepted)).len(), 2);
}
