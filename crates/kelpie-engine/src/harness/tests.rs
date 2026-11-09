use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::claude::{self, LOGIN_ENV, pin};
use super::goose::{self, Provider};
use super::{HarnessError, Login};
use crate::run::RunId;
use crate::session::{Choice, Extension, Harness, Session, SessionEvent};

/// The installed bridge command every test hands to `harness`
const BRIDGE: &str = "/kelpie-data/bridges/claude-acp/0.101.1/claude-agent-acp";

/// The session's state dir every test hands to `harness`
const STATE: &str = "/kelpie-data/projects/1/pm";

/// The login every test that needs one hands over
const TOKEN: &str = "test-token";

/// How long the hand-run test waits for one turn of a real session
const TURN: Duration = Duration::from_secs(180);

/// A folder of its own, named after this test's run
fn temp(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kelpie-{prefix}-{}", RunId::new().unwrap()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A folder holding a runnable `cmd`, as the person's install would
fn installed_with(cmd: &str) -> PathBuf {
    let dir = temp("harness-installed");
    let program = dir.join(cmd);
    fs::write(&program, "#!/bin/sh\necho kelpie harness fixture\n").unwrap();
    #[cfg(unix)]
    {
        use std::fs::Permissions;
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&program, Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// A folder holding a runnable `claude`, as the person's install would
fn installed() -> PathBuf {
    installed_with("claude")
}

/// A folder holding a runnable `goose`, as a person's install would
fn goose_installed() -> PathBuf {
    installed_with("goose")
}

/// A folder holding a `cmd` that cannot be run
fn not_runnable(cmd: &str) -> PathBuf {
    let dir = temp("harness-not-runnable");
    fs::write(dir.join(cmd), "not a program\n").unwrap();
    dir
}

/// `dirs` as the whole `PATH`
fn path_of(dirs: impl IntoIterator<Item: AsRef<OsStr>>) -> OsString {
    env::join_paths(dirs).unwrap()
}

/// Build a harness with `path` as the `PATH` it searches
fn build(path: &OsStr, login: Login) -> Result<Harness, HarnessError> {
    claude::harness_on_path(BRIDGE.into(), Path::new(STATE), &login, path)
}

/// The provider a test names, with its host when it has one
fn provider(name: &str, host: Option<&str>) -> Provider {
    Provider {
        name: name.to_owned(),
        host: host.map(str::to_owned),
    }
}

/// Build the harness of `cmd` with `path` as the `PATH` it searches
fn build_on(path: &OsStr, provider: &Provider) -> Result<Harness, HarnessError> {
    goose::harness_on_path(Path::new(STATE), provider, path)
}

/// The value `harness` sets `name` to
fn env_of(harness: &Harness, name: &str) -> Option<OsString> {
    harness
        .env
        .iter()
        .find(|(known, _)| known == name)
        .map(|(_, value)| value.clone())
}

#[test]
fn claude_points_the_bridge_at_the_persons_claude() {
    let dir = installed();

    let harness = build(&path_of([dir.clone()]), Login::from_token(TOKEN)).unwrap();

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

    let harness = build(&path_of([dir.clone()]), Login::from_token(TOKEN)).unwrap();

    assert_eq!(
        env_of(&harness, "CLAUDE_CONFIG_DIR").unwrap(),
        Path::new(STATE).join("claude")
    );
    assert_eq!(env_of(&harness, LOGIN_ENV).unwrap(), OsString::from(TOKEN));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn no_claude_on_path_says_to_install_it() {
    let empty = temp("harness-empty");
    let broken = not_runnable("claude");

    for path in [path_of([empty.clone()]), path_of([broken.clone()])] {
        let error = build(&path, Login::from_token(TOKEN)).unwrap_err();
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
fn no_login_says_how_to_make_one() {
    let dir = installed();

    for login in [Login::none(), Login::from_token("   ")] {
        let error = build(&path_of([dir.clone()]), login).unwrap_err();
        assert!(matches!(error, HarnessError::NoLogin { .. }), "{error}");
        let message = error.to_string();
        assert!(message.contains(LOGIN_ENV), "{message}");
        assert!(message.contains("claude setup-token"), "{message}");
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_new_session_meta_asks_for_the_raw_messages() {
    let dir = installed();

    let harness = build(&path_of([dir.clone()]), Login::from_token(TOKEN)).unwrap();

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

#[test]
fn a_login_keeps_its_token_out_of_debug() {
    assert_eq!(
        format!("{:?}", Login::from_token(TOKEN)),
        r#"Login { token: true }"#
    );
    assert_eq!(format!("{:?}", Login::none()), r#"Login { token: false }"#);
}

#[test]
fn the_pin_holds_one_runnable_archive_per_platform() {
    let pin = pin();

    assert_eq!(pin.id, "claude-acp");
    assert_eq!(pin.version, "0.101.1");
    assert_eq!(pin.platforms.len(), 4);
    for (platform, download) in pin.platforms {
        let url = download.url.as_str();
        let archive = download.url.rsplit('/').next().unwrap();
        let expected = match platform {
            Platform::MacOsArm64 => "claude-agent-acp-darwin-arm64.zip",
            Platform::MacOsX64 => "claude-agent-acp-darwin-x64.zip",
            Platform::LinuxX64 => "claude-agent-acp-linux-x64.tar.gz",
            Platform::LinuxArm64 => "claude-agent-acp-linux-arm64.tar.gz",
            Platform::WindowsX64 => "no windows archive is pinned yet",
        };
        assert_eq!(archive, expected, "{url}");
        assert_eq!(download.cmd, "claude-agent-acp");
        assert!(
            download
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
                && download.sha256.len() == 64,
            "{:?}",
            download.sha256
        );
    }
}

use crate::bridge::Platform;

/// Runs a real Claude Code session by hand.
///
/// Needs the person's own `claude` on `PATH`, `CLAUDE_CODE_OAUTH_TOKEN` in the
/// environment, and a download of the pinned bridge. It sends one message on
/// Haiku, stops the session, resumes it, and checks the reply shows the session
/// remembered what was said.
#[ignore = "needs a Claude Code login, the person's own claude, and a bridge download"]
#[test]
fn a_real_claude_answers_and_resumes() {
    use crate::bridge;
    use crate::tree::ProcessTree;

    let data = temp("harness-real");
    let project = data.join("project");
    fs::create_dir_all(&project).unwrap();
    let bridge = bridge::ensure(&data, pin()).unwrap();
    let harness = claude::harness(bridge, &data.join("pm"), Login::from_env(LOGIN_ENV)).unwrap();
    let tree = Arc::new(ProcessTree::new(RunId::new().unwrap(), data.join("run.json")).unwrap());
    let choice = Choice {
        model: Some("haiku".to_owned()),
        effort: None,
    };

    let first = Session::new(
        Arc::clone(&tree),
        harness.clone(),
        project.clone(),
        None,
        choice.clone(),
    );
    let answer = ask(
        &first,
        "Remember the word kelpie. Reply with that one word.",
    );
    let id = first.session_id().expect("no session id from Claude Code");
    drop(first);

    let second = Session::new(Arc::clone(&tree), harness, project, Some(id), choice);
    let recall = ask(
        &second,
        "Which word did I ask you to remember? Reply with that one word.",
    );
    drop(second);

    tree.stop_all(Duration::from_secs(10)).unwrap();
    assert!(
        said(&answer).to_lowercase().contains("kelpie"),
        "the first answer: {:?}",
        said(&answer)
    );
    assert!(
        said(&recall).to_lowercase().contains("kelpie"),
        "the resumed session did not remember: {:?}",
        said(&recall)
    );
    fs::remove_dir_all(data).unwrap();
}

/// Runs a real goose session by hand.
///
/// Needs the person's own goose on `PATH`, and a model it can reach: kelpie
/// names the provider `ollama` and asks for no model, so goose keeps its own
/// config. `GOOSE_MODEL` and `OLLAMA_HOST` in the environment that starts the
/// test reach the session, which inherits it. Run it with Ollama up:
/// `cargo test -p kelpie-engine --lib a_real_goose_answers_and_resumes -- --ignored`
#[ignore = "needs the person's own goose on PATH, and a local model it can reach"]
#[test]
fn a_real_goose_answers_and_resumes() {
    use crate::tree::ProcessTree;

    let data = temp("harness-real");
    let project = data.join("project");
    fs::create_dir_all(&project).unwrap();
    let harness = goose::harness(&data.join("pm"), provider("ollama", None)).unwrap();
    let tree = Arc::new(ProcessTree::new(RunId::new().unwrap(), data.join("run.json")).unwrap());
    let choice = Choice::default();

    let first = Session::new(
        Arc::clone(&tree),
        harness.clone(),
        project.clone(),
        None,
        choice.clone(),
    );
    let answer = ask(
        &first,
        "Remember the word kelpie. Reply with that one word.",
    );
    let id = first.session_id().expect("no session id from goose");
    drop(first);

    let second = Session::new(Arc::clone(&tree), harness, project, Some(id), choice);
    let recall = ask(
        &second,
        "Which word did I ask you to remember? Reply with that one word.",
    );
    drop(second);

    tree.stop_all(Duration::from_secs(10)).unwrap();
    assert!(
        said(&answer).to_lowercase().contains("kelpie"),
        "the first answer: {:?}",
        said(&answer)
    );
    assert!(
        said(&recall).to_lowercase().contains("kelpie"),
        "the resumed session did not remember: {:?}",
        said(&recall)
    );
    fs::remove_dir_all(data).unwrap();
}

/// One message on a live session: print every event, and return them all once the turn ends
fn ask(session: &Session, message: &str) -> Vec<SessionEvent> {
    let (sender, heard) = mpsc::channel();
    let label = message.to_owned();
    session.send(message, move |event| {
        println!("{label} -> {event:?}");
        let _ = sender.send(event);
    });
    let deadline = Instant::now() + TURN;
    let mut events = Vec::new();
    loop {
        let Ok(event) = heard.recv_timeout(
            deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
        ) else {
            panic!("no turn from {message} within {} s", TURN.as_secs());
        };
        let ended = matches!(event, SessionEvent::TurnEnded { .. });
        events.push(event);
        if ended {
            return events;
        }
    }
}

/// What a session's events said, joined into one text
fn said(events: &[SessionEvent]) -> String {
    events
        .iter()
        .map(|event| match event {
            SessionEvent::Text { delta } | SessionEvent::Failed { message: delta } => delta.clone(),
            _ => String::new(),
        })
        .collect()
}
