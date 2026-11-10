//! Sessions run by hand: a real Claude Code and a real goose, each answered,
//! stopped and resumed, with the events a turn gave printed as they arrive
//!
//! Every test here is `#[ignore]`d: each needs the person's own harness, a
//! login or a model, and a bridge download. The command to run one is in its
//! doc comment.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use super::claude::{LOGIN_ENV, pin};
use crate::session::{Choice, Session, SessionEvent};

use super::*;

/// Runs a real Claude Code session across two bridge versions by hand.
///
/// Needs the person's own `claude` on `PATH`, `CLAUDE_CODE_OAUTH_TOKEN` in the
/// environment, and two bridge downloads: kelpie's pin, then the registry's
/// latest. It sends one message on Haiku on the pinned bridge, then resumes the
/// same session id on the newer one, and checks the reply shows the session
/// remembered what was said. Run it with:
/// `cargo test -p kelpie-engine --lib a_real_claude_resumes_across_bridge_versions -- --ignored`
#[ignore = "needs a Claude Code login, a model, and two bridge downloads"]
#[test]
fn a_real_claude_resumes_across_bridge_versions() {
    use crate::bridge::{self, CheckOutcome};
    use crate::settings::BridgeUpdates;
    use crate::tree::ProcessTree;

    let data = temp("harness-bridge-swap");
    let project = data.join("project");
    fs::create_dir_all(&project).unwrap();
    let logins = Store::memory();
    logins
        .set(
            LOGIN,
            &env::var(LOGIN_ENV).expect("CLAUDE_CODE_OAUTH_TOKEN is not set"),
        )
        .unwrap();
    let tree = Arc::new(ProcessTree::new(RunId::new().unwrap(), data.join("run.json")).unwrap());
    let choice = Choice {
        model: Some("haiku".to_owned()),
        effort: None,
    };

    let pinned = bridge::ensure(&data, pin()).unwrap();
    let first = Session::new(
        Arc::clone(&tree),
        claude::harness(pinned, &data.join("pm"), &logins, false).unwrap(),
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

    // The registry's latest, installed and probed the way a bridge check does
    let latest = match bridge::check(
        &data,
        &tree,
        pin(),
        BridgeUpdates::RegistryAutomatic,
        claude::REQUIRES,
    ) {
        CheckOutcome::Ready { version } => version,
        CheckOutcome::Current => {
            panic!(
                "the registry holds nothing newer than kelpie's pin, so there is no swap to make"
            )
        }
        outcome => panic!("could not install the registry's latest: {outcome:?}"),
    };
    let newer = data
        .join("bridges")
        .join(pin().id)
        .join(latest.to_string())
        .join(claude::CMD);
    assert!(newer.is_file(), "{newer:?} was not installed");

    let second = Session::new(
        Arc::clone(&tree),
        claude::harness(newer, &data.join("pm"), &logins, false).unwrap(),
        project,
        Some(id),
        choice,
    );
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
        "the session did not survive the swap to {}: {:?}",
        latest,
        said(&recall)
    );
    fs::remove_dir_all(data).unwrap();
}

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
    let logins = Store::memory();
    logins
        .set(
            LOGIN,
            &env::var(LOGIN_ENV).expect("CLAUDE_CODE_OAUTH_TOKEN is not set"),
        )
        .unwrap();
    let harness = claude::harness(bridge, &data.join("pm"), &logins, false).unwrap();
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
    let harness = goose::harness(&data.join("pm"), provider("ollama", None), false).unwrap();
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
