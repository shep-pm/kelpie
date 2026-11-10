//! The person's own global harness folder, linked into a session's config
//! folder when they turn *Harness global folder* on, and taken back when they
//! turn it off
//!
//! `harness/global.rs`, through `claude::harness_on_path` and
//! `goose::harness_on_path`

use super::*;

/// The folder a test hands as `state_dir`, which the harness makes config folders in
fn session_state() -> PathBuf {
    temp("harness-state")
}

/// A person's Claude Code global folder: every entry but `output-styles`, and their login beside them
fn claudes_global() -> PathBuf {
    let global = temp("harness-global-claude");
    fs::write(
        global.join("CLAUDE.md"),
        "# the person's own instructions\n",
    )
    .unwrap();
    fs::write(global.join("settings.json"), "{\"model\":\"opus\"}\n").unwrap();
    fs::write(
        global.join(".credentials.json"),
        "{\"token\":\"never-read\"}\n",
    )
    .unwrap();
    fs::write(global.join(".claude.json"), "{\"mcpServers\":{}}\n").unwrap();
    for folder in ["skills", "agents", "commands"] {
        fs::create_dir_all(global.join(folder)).unwrap();
        fs::write(global.join(folder).join("one.md"), "one\n").unwrap();
    }
    global
}

/// A person's goose config folder: their config, the secrets beside it, and their prompts
///
/// goose keeps this folder at `~/.config/goose` when the person sets nothing, and
/// at `<GOOSE_PATH_ROOT>/config` when they set it, which is what
/// `goose::global_folder` is for. Its secrets and its prompts are not entries.
fn gooses_config() -> PathBuf {
    let global = temp("harness-global-goose");
    fs::write(global.join("config.yaml"), "GOOSE_MODEL: x\n").unwrap();
    fs::write(
        global.join("secrets.yaml"),
        "ANTHROPIC_API_KEY: never-read\n",
    )
    .unwrap();
    fs::create_dir_all(global.join("prompts")).unwrap();
    fs::write(
        global.join("prompts/compaction_summary.md"),
        "not kelpie's\n",
    )
    .unwrap();
    global
}

/// Claude Code on `state`, with the person's own global folder read when `on`
fn claudes_harness(
    state: &Path,
    global: &Path,
    install: &Path,
    on: bool,
) -> Result<Harness, HarnessError> {
    claude::harness_on_path(
        BRIDGE.into(),
        state,
        &logged_in(),
        &path_of([install]),
        Some(global),
        on,
    )
}

/// goose on `state`, with the person's own goose config folder read when `on`
fn gooses_harness(
    state: &Path,
    global: &Path,
    install: &Path,
    on: bool,
) -> Result<Harness, HarnessError> {
    goose::harness_on_path(
        state,
        &provider("ollama", None),
        &path_of([install]),
        Some(global),
        on,
    )
}

/// Every name under `folder`, as `folder` names them, links and folders alike
fn names(folder: &Path) -> Vec<String> {
    let Ok(read) = fs::read_dir(folder) else {
        return Vec::new();
    };
    read.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn claudes_global_folder_is_linked_when_on() {
    let (install, global, state) = (installed(), claudes_global(), session_state());

    let harness = claudes_harness(&state, &global, &install, true).unwrap();

    let config = state.join("claude");
    for name in ["CLAUDE.md", "settings.json", "skills", "agents", "commands"] {
        assert_eq!(
            fs::read_link(config.join(name)).unwrap(),
            global.join(name),
            "{name} is not linked into the session's config folder"
        );
    }
    assert!(
        !config.join("output-styles").exists(),
        "an entry the person does not have was made anyway"
    );
    assert_eq!(
        harness.global_reads,
        ["CLAUDE.md", "settings.json", "skills", "agents", "commands"]
            .iter()
            .map(|name| global.join(name))
            .collect::<Vec<PathBuf>>(),
        "the sandbox is not told what the harness reads"
    );
    assert_eq!(
        env_of(&harness, "CLAUDE_CONFIG_DIR").unwrap(),
        config,
        "the session still gets a config folder of kelpie's own"
    );
    for dir in [install, global, state] {
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn gooses_global_config_is_linked_where_goose_reads_it() {
    let (install, global, state) = (goose_installed(), gooses_config(), session_state());

    let harness = gooses_harness(&state, &global, &install, true).unwrap();

    // goose reads its config from `<root>/config/`, so the link has to sit there
    // and not in the root it is given.
    let config = state.join("goose").join("config");
    assert_eq!(
        fs::read_link(config.join("config.yaml")).unwrap(),
        global.join("config.yaml"),
        "goose's own config is not linked where goose looks for it"
    );
    assert_eq!(
        fs::read_to_string(config.join("config.yaml")).unwrap(),
        "GOOSE_MODEL: x\n",
        "the link does not carry the person's own config"
    );
    assert_eq!(
        names(&state.join("goose")),
        ["config"],
        "the session's goose root holds only the config folder goose reads"
    );
    assert_eq!(
        harness.global_reads,
        [global.join("config.yaml")],
        "the sandbox is not told what the harness reads"
    );
    assert_eq!(
        env_of(&harness, "GOOSE_PATH_ROOT").unwrap(),
        state.join("goose"),
        "the session still gets a goose root of kelpie's own"
    );
    for dir in [install, global, state] {
        fs::remove_dir_all(dir).unwrap();
    }
}

/// Where the person's own goose keeps its config, by what they set
#[test]
fn gooses_config_folder_is_one_under_a_root_the_person_names() {
    let (root, home) = (temp("harness-goose-root"), temp("harness-home"));
    let (unset, empty) = (
        Some(home.join(".config").join("goose")),
        Some(home.join(".config").join("goose")),
    );

    assert_eq!(
        goose::global_folder(Some(OsStr::new(&root)), Some(&home)),
        Some(root.join("config")),
        "a person who set GOOSE_PATH_ROOT keeps their config one folder under it"
    );
    assert_eq!(
        goose::global_folder(None, Some(&home)),
        unset,
        "a person who set nothing keeps it in the goose folder under the OS's config folder"
    );
    assert_eq!(
        goose::global_folder(Some(OsStr::new("")), Some(&home)),
        empty,
        "an empty root names no folder of its own"
    );
    assert_eq!(
        goose::global_folder(Some(OsStr::new(&root)), None),
        Some(root.join("config")),
        "a root needs no home folder to be read under"
    );
    assert_eq!(
        goose::global_folder(None, None),
        None,
        "nothing names a folder to read"
    );
    for dir in [root, home] {
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn nothing_is_linked_when_off() {
    let (install, global) = (installed(), claudes_global());
    let (goose_install, goose_global) = (goose_installed(), gooses_config());
    let (claude_state, goose_state) = (session_state(), session_state());

    let claude = claudes_harness(&claude_state, &global, &install, false).unwrap();
    let goose = gooses_harness(&goose_state, &goose_global, &goose_install, false).unwrap();

    assert!(claude.global_reads.is_empty(), "{:?}", claude.global_reads);
    assert!(goose.global_reads.is_empty(), "{:?}", goose.global_reads);
    assert_eq!(names(&claude_state.join("claude")), Vec::<String>::new());
    assert_eq!(names(&goose_state.join("goose")), Vec::<String>::new());
    assert!(
        !goose_state.join("goose").join("config").exists(),
        "the config folder was made with nothing to link into it"
    );
    for dir in [
        install,
        global,
        goose_install,
        goose_global,
        claude_state,
        goose_state,
    ] {
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn turning_it_off_removes_the_links() {
    let (install, global, state) = (installed(), claudes_global(), session_state());

    claudes_harness(&state, &global, &install, true).unwrap();
    let config = state.join("claude");
    assert!(config.join("CLAUDE.md").is_symlink());

    let off = claudes_harness(&state, &global, &install, false).unwrap();

    for name in ["CLAUDE.md", "settings.json", "skills", "agents", "commands"] {
        assert!(!config.join(name).exists(), "{name} is still linked");
    }
    assert!(off.global_reads.is_empty(), "{:?}", off.global_reads);
    assert_eq!(
        fs::read_to_string(global.join("CLAUDE.md")).unwrap(),
        "# the person's own instructions\n",
        "taking the links back changed the person's own file"
    );
    for dir in [install, global, state] {
        fs::remove_dir_all(dir).unwrap();
    }
}

/// goose's link sits one folder under the root kelpie gives it, where goose looks
/// for its config, so taking the setting back has to reach it down there
#[test]
fn turning_it_off_removes_a_link_one_folder_deep() {
    let (install, global, state) = (goose_installed(), gooses_config(), session_state());
    let config = state.join("goose").join("config");

    gooses_harness(&state, &global, &install, true).unwrap();
    assert!(
        config.join("config.yaml").is_symlink(),
        "the link was not made where goose looks"
    );

    let off = gooses_harness(&state, &global, &install, false).unwrap();

    assert!(
        !config.join("config.yaml").exists(),
        "the link one folder deep was left in the session's goose root"
    );
    assert!(off.global_reads.is_empty(), "{:?}", off.global_reads);
    assert_eq!(
        fs::read_to_string(global.join("config.yaml")).unwrap(),
        "GOOSE_MODEL: x\n",
        "taking the link back changed the person's own config"
    );
    for dir in [install, global, state] {
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn a_login_in_the_global_folder_is_never_linked() {
    let (install, global, state) = (installed(), claudes_global(), session_state());
    let (goose_install, goose_global, goose_state) =
        (goose_installed(), gooses_config(), session_state());

    let claude = claudes_harness(&state, &global, &install, true).unwrap();
    let goose = gooses_harness(&goose_state, &goose_global, &goose_install, true).unwrap();

    for name in [".credentials.json", ".claude.json"] {
        assert!(
            !state.join("claude").join(name).exists(),
            "{name} reached the session's config folder"
        );
    }
    let goose_config = goose_state.join("goose").join("config");
    for name in ["secrets.yaml", "prompts"] {
        assert!(
            !goose_config.join(name).exists(),
            "{name} reached the session's goose config folder"
        );
    }
    for read in claude.global_reads.iter().chain(goose.global_reads.iter()) {
        let last = read.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            last != "secrets.yaml" && last != ".credentials.json",
            "{read:?} is a login or a secret"
        );
    }
    for dir in [
        install,
        global,
        goose_install,
        goose_global,
        state,
        goose_state,
    ] {
        fs::remove_dir_all(dir).unwrap();
    }
}

/// kelpie's own settings for the session, and the person's own beside them
#[test]
fn a_file_of_kelpies_own_in_the_session_folder_stands_in_place_of_a_link() {
    let (install, global, state) = (installed(), claudes_global(), session_state());
    let config = state.join("claude");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        "{\"permissionMode\":\"bypassPermissions\"}\n",
    )
    .unwrap();

    let harness = claudes_harness(&state, &global, &install, true).unwrap();

    assert!(
        !config.join("settings.json").is_symlink(),
        "the person's settings.json took the place of kelpie's"
    );
    assert_eq!(
        fs::read_to_string(config.join("settings.json")).unwrap(),
        "{\"permissionMode\":\"bypassPermissions\"}\n"
    );
    assert!(config.join("CLAUDE.md").is_symlink());
    assert_eq!(
        harness.global_reads,
        ["CLAUDE.md", "skills", "agents", "commands"]
            .iter()
            .map(|name| global.join(name))
            .collect::<Vec<PathBuf>>(),
        "the sandbox was told to open the person's settings.json for a file the session holds as its own"
    );
    for dir in [install, global, state] {
        fs::remove_dir_all(dir).unwrap();
    }
}

/// A harness that saves its settings by renaming a new file over a link leaves a
/// regular file in the session's config folder, which kelpie cannot tell from one
/// it wrote there itself
#[test]
fn a_file_written_over_a_link_stands_and_is_not_read_as_the_persons() {
    let (install, global, state) = (installed(), claudes_global(), session_state());
    let config = state.join("claude");
    let (link, persons_own) = (config.join("settings.json"), global.join("settings.json"));

    claudes_harness(&state, &global, &install, true).unwrap();
    assert!(link.is_symlink());
    fs::remove_file(&link).unwrap();
    fs::write(&link, "{\"model\":\"sonnet\"}\n").unwrap();

    let on = claudes_harness(&state, &global, &install, true).unwrap();
    assert!(
        !link.is_symlink(),
        "the session's own settings were replaced by a link"
    );
    assert!(
        !on.global_reads.contains(&persons_own),
        "the person's settings.json was opened for a file the session holds as its own: {:?}",
        on.global_reads
    );

    let off = claudes_harness(&state, &global, &install, false).unwrap();
    assert!(off.global_reads.is_empty(), "{:?}", off.global_reads);
    assert_eq!(
        fs::read_to_string(&link).unwrap(),
        "{\"model\":\"sonnet\"}\n",
        "a file of the session's own was taken back with the links"
    );
    assert_eq!(
        fs::read_to_string(&persons_own).unwrap(),
        "{\"model\":\"opus\"}\n",
        "the person's own file was changed"
    );
    for dir in [install, global, state] {
        fs::remove_dir_all(dir).unwrap();
    }
}
