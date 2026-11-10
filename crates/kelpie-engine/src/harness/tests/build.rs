//! Building Claude Code's and goose's harness: what a session runs, and what
//! kelpie says when an install or a login is missing
//!
//! `harness/claude.rs` and `harness/goose.rs`

use serde_json::json;

use super::claude::LOGIN_ENV;
use crate::session::Extension;

use super::*;

#[test]
fn claude_points_the_bridge_at_the_persons_claude() {
    let dir = installed();

    let harness = build(&path_of([dir.clone()]), &logged_in()).unwrap();

    assert_eq!(harness.program, PathBuf::from(BRIDGE));
    assert!(harness.args.is_empty());
    let executable = env_of(&harness, "CLAUDE_CODE_EXECUTABLE").unwrap();
    assert_eq!(executable, dir.join("claude"));
    assert!(Path::new(&executable).is_absolute(), "{executable:?}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn claude_gets_a_config_folder_of_its_own() {
    let dir = installed();

    let harness = build(&path_of([dir.clone()]), &logged_in()).unwrap();

    assert_eq!(
        env_of(&harness, "CLAUDE_CONFIG_DIR").unwrap(),
        Path::new(STATE).join("claude")
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn claude_takes_its_login_from_the_store() {
    let dir = installed();
    let logins = Store::memory();
    logins.set(LOGIN, "  from-the-store\n").unwrap();
    logins.set("goose", "another-harness-login").unwrap();

    let harness = build(&path_of([dir.clone()]), &logins).unwrap();

    assert_eq!(
        env_of(&harness, LOGIN_ENV).unwrap(),
        OsString::from("from-the-store")
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn no_claude_on_path_says_to_install_it() {
    let empty = temp("harness-empty");
    let broken = not_runnable("claude");

    for path in [path_of([empty.clone()]), path_of([broken.clone()])] {
        let error = build(&path, &logged_in()).unwrap_err();
        assert!(
            matches!(error, HarnessError::NotInstalled { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("Claude Code is not installed"));
        assert!(error.to_string().contains("Install Claude Code"));
    }
    fs::remove_dir_all(empty).unwrap();
    fs::remove_dir_all(broken).unwrap();
}

#[test]
fn no_login_points_at_the_settings_page() {
    let dir = installed();
    let others = Store::memory();
    others.set("goose", "not-claudes").unwrap();

    for logins in [Store::memory(), others] {
        let error = build(&path_of([dir.clone()]), &logins).unwrap_err();
        assert!(matches!(error, HarnessError::NoLogin { .. }), "{error}");
        let message = error.to_string();
        assert!(message.contains("settings page"), "{message}");
        assert!(message.contains("claude setup-token"), "{message}");
        assert!(!message.contains(LOGIN_ENV), "{message}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_new_session_meta_asks_for_the_raw_messages() {
    let dir = installed();

    let harness = build(&path_of([dir.clone()]), &logged_in()).unwrap();

    assert_eq!(
        harness.new_session_meta.unwrap(),
        json!({ "claudeCode": { "emitRawSDKMessages": true } })
    );
    assert_eq!(harness.extensions, [Extension::ClaudeSdkMessage]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn goose_runs_acp_with_a_config_root_of_its_own() {
    let dir = goose_installed();

    let harness = build_on(&path_of([dir.clone()]), &provider("ollama", None)).unwrap();

    assert_eq!(harness.program, dir.join("goose"));
    assert_eq!(harness.args, [OsString::from("acp")]);
    assert_eq!(
        env_of(&harness, "GOOSE_PATH_ROOT").unwrap(),
        Path::new(STATE).join("goose")
    );
    assert!(
        harness.new_session_meta.is_none(),
        "goose needs no `_meta` of kelpie's"
    );
    assert!(
        harness.extensions.is_empty(),
        "kelpie reads no goose notification beyond ACP's own"
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn goose_gets_its_provider_and_host() {
    let dir = goose_installed();

    let named = build_on(
        &path_of([dir.clone()]),
        &provider("ollama", Some("http://127.0.0.1:11434")),
    )
    .unwrap();
    let its_own = build_on(&path_of([dir.clone()]), &provider("ollama", None)).unwrap();

    assert_eq!(
        env_of(&named, "GOOSE_PROVIDER").unwrap(),
        OsString::from("ollama")
    );
    assert_eq!(
        env_of(&named, "OLLAMA_HOST").unwrap(),
        OsString::from("http://127.0.0.1:11434")
    );
    assert_eq!(
        env_of(&its_own, "GOOSE_PROVIDER").unwrap(),
        OsString::from("ollama")
    );
    assert!(
        env_of(&its_own, "OLLAMA_HOST").is_none(),
        "no host: goose keeps its own config"
    );
    for harness in [&named, &its_own] {
        assert!(
            env_of(harness, "GOOSE_MODEL").is_none(),
            "the model is a config option, not an environment variable"
        );
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn no_goose_on_path_says_to_install_it() {
    let empty = temp("harness-empty");
    let broken = not_runnable("goose");

    for path in [path_of([empty.clone()]), path_of([broken.clone()])] {
        let error = build_on(&path, &provider("ollama", None)).unwrap_err();
        assert!(
            matches!(error, HarnessError::NotInstalled { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("goose is not installed"));
        assert!(error.to_string().contains("Install goose"));
    }
    fs::remove_dir_all(empty).unwrap();
    fs::remove_dir_all(broken).unwrap();
}
