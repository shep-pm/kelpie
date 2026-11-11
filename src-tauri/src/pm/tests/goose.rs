//! The PM on goose: what the settings hand `goose::harness`, and what the chat
//! hears when goose cannot run
//!
//! `goose::harness` searches `PATH` for the person's own goose, and the goose
//! arm's session step starts crabpot around the harness it builds. A unit test
//! has neither, so each test hands the arm stand-ins for those two steps and
//! reads what it asked for.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use kelpie_engine::harness::goose;

use super::*;

/// What the goose arm handed `goose::harness`
#[derive(Debug, Clone)]
struct Ask {
    state_dir: PathBuf,
    provider: goose::Provider,
    harness_global_folder: bool,
}

/// What each test's stand-in for `goose::harness` recorded
type Asks = Arc<Mutex<Vec<Ask>>>;

/// The goose arm's `build` step, stood in for: record the provider kelpie asked
/// for, and hand back `agent`
fn found(
    agent: &FakeAgent,
    asks: &Asks,
) -> impl Fn(&Path, goose::Provider, bool) -> Result<SessionHarness, HarnessError> + Send + Sync + 'static
{
    let (agent, asks) = (agent.clone(), Arc::clone(asks));
    move |state_dir, provider, harness_global_folder| {
        asks.lock().unwrap().push(Ask {
            state_dir: state_dir.to_path_buf(),
            provider,
            harness_global_folder,
        });
        Ok(agent.harness())
    }
}

/// The same, answering the way `goose::harness` does when `PATH` holds no goose
fn not_on_path(
    asks: &Asks,
) -> impl Fn(&Path, goose::Provider, bool) -> Result<SessionHarness, HarnessError> + Send + Sync + 'static
{
    let asks = Arc::clone(asks);
    move |state_dir, provider, harness_global_folder| {
        asks.lock().unwrap().push(Ask {
            state_dir: state_dir.to_path_buf(),
            provider,
            harness_global_folder,
        });
        Err(HarnessError::NotInstalled {
            harness: "goose".to_owned(),
            cmd: "goose".to_owned(),
        })
    }
}

/// A chat whose PM runs on goose, with `agent` as `goose acp` and `build` stood in for
///
/// The session step is stood in for too, since a unit test starts no sandbox.
/// What it hands back is a session on `agent` in the project's folder, which is
/// what the arm's own step opens.
fn on_goose(
    agent: &FakeAgent,
    build: impl Fn(&Path, goose::Provider, bool) -> Result<SessionHarness, HarnessError>
    + Send
    + Sync
    + 'static,
) -> Chat {
    let agent = agent.clone();
    let opener: Opener = Box::new(move |opening| {
        open_with(opening, &build, |opening, _harness, _tools| {
            Ok(agent.session(
                opening.project.path.clone(),
                opening.project.pm_session.clone(),
                opening.pm.choice.clone(),
            ))
        })
    });
    let chat = Chat::with(opener, Box::new(|| Ok(None)), None);
    chat.state().set_project(chat.dir.clone()).unwrap();
    chat
}

/// Set each of `fields` in the project's own settings file
fn set(chat: &Chat, project: &str, fields: &[(&str, &str)]) {
    for (key, value) in fields {
        settings_set(
            chat.handle(),
            chat.state(),
            Target::Project {
                id: project.to_owned(),
            },
            (*key).to_owned(),
            (*value).into(),
        )
        .unwrap();
    }
}

/// What the chat heard when the goose arm could not open a session
fn failure(chat: &Chat) -> String {
    let events = chat.send("Hi").until_turn_ends();
    let Some(SessionEvent::Failed { message }) = events.last() else {
        panic!("the goose arm started a session: {events:?}");
    };
    message.clone()
}

#[test]
fn goose_in_the_settings_opens_the_pm_on_goose() {
    let agent = FakeAgent::new().turn(Turn::new().text("On goose."));
    let asks: Asks = Arc::new(Mutex::new(Vec::new()));
    let chat = on_goose(&agent, found(&agent, &asks));
    let project = chat.state().project_id(None).unwrap();
    set(
        &chat,
        &project,
        &[
            ("pm-harness", "goose"),
            ("provider", "ollama"),
            ("host", "http://127.0.0.1:11434"),
        ],
    );

    let events = chat.send("Hi").until_turn_ends();

    assert!(
        events.contains(&SessionEvent::Text {
            delta: "On goose.".to_owned()
        }),
        "{events:?}"
    );
    let asked = asks.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "the settings named goose once");
    assert_eq!(
        asked[0].provider,
        goose::Provider {
            name: "ollama".to_owned(),
            host: Some("http://127.0.0.1:11434".to_owned()),
        },
        "goose is asked to run on the provider the settings name"
    );
    assert_eq!(
        asked[0].state_dir,
        chat.dir.join("projects").join(&project).join("pm"),
        "goose keeps its config in the PM's own state folder"
    );
    assert!(
        !asked[0].harness_global_folder,
        "the person's own goose config stays out until they turn it on"
    );
    assert_eq!(agent.launches(), 1, "the PM ran on goose once");
}

#[test]
fn goose_with_no_provider_is_refused_naming_the_setting() {
    let agent = FakeAgent::new().turn(Turn::new().text("never runs"));
    let asks: Asks = Arc::new(Mutex::new(Vec::new()));
    let chat = on_goose(&agent, found(&agent, &asks));
    let project = chat.state().project_id(None).unwrap();
    set(&chat, &project, &[("pm-harness", "goose")]);

    let message = failure(&chat);

    assert!(message.contains("`provider`"), "{message}");
    assert!(message.contains("goose"), "{message}");
    assert!(
        asks.lock().unwrap().is_empty(),
        "kelpie never asked for goose"
    );
    assert_eq!(agent.launches(), 0, "nothing started");
}

#[test]
fn goose_not_on_path_says_kelpie_runs_the_persons_own_goose() {
    let agent = FakeAgent::new().turn(Turn::new().text("never runs"));
    let asks: Asks = Arc::new(Mutex::new(Vec::new()));
    let chat = on_goose(&agent, not_on_path(&asks));
    let project = chat.state().project_id(None).unwrap();
    set(
        &chat,
        &project,
        &[("pm-harness", "goose"), ("provider", "ollama")],
    );

    let message = failure(&chat);

    assert!(
        message.contains("goose") && message.contains("your own"),
        "{message}"
    );
    assert_eq!(message.lines().count(), 1, "one line: {message}");
    assert_eq!(agent.launches(), 0, "nothing started");
}
