//! Starting a worker, on a repo and a harness of each test's own
//!
//! Each test gets a folder with a bare origin holding `main`, a checkout of it,
//! and a [`FakeAgent`] as the harness, so a start prepares a real worktree
//! through git and runs a real session over ACP with no model behind it.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crabpot::Config;
use kelpie_protocol::{SessionEvent, StopReason, ToolStatus};
use serde_json::json;

use super::*;
use crate::agents::{Binding, Origin, Sourced};
use crate::github::fake::{FakeClock, FakeGithub};
use crate::github::{AppTokens, TokenError};
use crate::sandbox::{self, Reads};
use crate::session::fake::{FakeAgent, Turn};
use crate::session::{Harness, Session};
use crate::tree::ProcessTree;
use crate::worktree::tests::{Dir, git, process_tree};

/// How long a test waits for an event its own turn must send
const WAIT: Duration = Duration::from_secs(20);

/// A sink, and the events it was given
fn events() -> (
    impl FnMut(SessionEvent) + Send + 'static,
    Receiver<SessionEvent>,
) {
    let (sender, receiver) = channel();
    (
        move |event| {
            let _ = sender.send(event);
        },
        receiver,
    )
}

/// Every event up to and including the one that ends the turn
fn until_turn_ends(heard: &Receiver<SessionEvent>) -> Vec<SessionEvent> {
    let mut seen = Vec::new();
    loop {
        let event = heard
            .recv_timeout(WAIT)
            .expect("the worker sent no more events");
        let ends = matches!(event, SessionEvent::TurnEnded { .. });
        seen.push(event);
        if ends {
            return seen;
        }
    }
}

/// The first event the turn sends, which is its `Started`
fn until_started(heard: &Receiver<SessionEvent>) -> SessionEvent {
    heard.recv_timeout(WAIT).expect("the worker sent no event")
}

/// The records on disk, which is what the next launch reads
fn kept(rig: &Rig) -> Vec<Record> {
    Records::load(rig.paths.records()).unwrap().list().to_vec()
}

/// What a worker runs on when no agent file or project setting says otherwise
fn binding() -> Binding {
    binding_on("Do the ticket.")
}

/// The same, with `body` as the agent file's instructions
fn binding_on(body: &str) -> Binding {
    Binding {
        harness: Sourced {
            value: None,
            from: Origin::BuiltIn,
        },
        model: Sourced {
            value: None,
            from: Origin::BuiltIn,
        },
        effort: Sourced {
            value: None,
            from: Origin::BuiltIn,
        },
        body: Sourced {
            value: Some(body.to_owned()),
            from: Origin::BuiltIn,
        },
    }
}

/// The ticket a test starts
fn ticket(id: &str, text: &str) -> Ticket {
    Ticket {
        id: id.to_owned(),
        text: text.to_owned(),
    }
}

/// A project with an origin holding `main`, a checkout of it, and its records file
struct Rig {
    _dir: Dir,
    tree: Arc<ProcessTree>,
    repo: PathBuf,
    paths: Paths,
    records: Arc<Mutex<Records>>,
    // The Apps kelpie mints this project's workers' tokens from, `None` where it
    // keeps none at all
    tokens: Option<AppTokens>,
}

impl Rig {
    fn new(name: &str) -> Self {
        let dir = Dir::new(name);
        let root = dir.path().canonicalize().unwrap();
        git(
            &root,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&root, &["clone", "--quiet", "origin.git", "repo"]);
        let repo = root.join("repo");
        git(&repo, &["commit", "--quiet", "--allow-empty", "-m", "init"]);
        git(&repo, &["push", "--quiet", "origin", "main"]);
        let paths = Paths::new(root.join("data"), "p1");
        let records = Arc::new(Mutex::new(Records::load(paths.records()).unwrap()));
        let tree = Arc::new(process_tree(&root));
        Self {
            _dir: dir,
            tree,
            repo,
            paths,
            records,
            tokens: None,
        }
    }

    /// Make the checkout's `origin` `slug` on GitHub
    ///
    /// A `[url "<the bare origin here>"] insteadOf = <slug's URL>` section is
    /// what git resolves that URL with, so the origin stays the local bare repo
    /// this rig was cloned from and every fetch the start makes is off the
    /// network. It is appended to the clone's config by hand: git's own
    /// `config` command keeps the quotes of a key like
    /// `url."<base>".insteadOf` in the subsection it writes.
    fn on_github(&mut self, slug: &str) {
        let url = format!("https://github.com/{slug}.git");
        let origin = self._dir.path().join("origin.git").canonicalize().unwrap();
        git(&self.repo, &["config", "remote.origin.url", &url]);
        std::fs::OpenOptions::new()
            .append(true)
            .open(self.repo.join(".git/config"))
            .unwrap()
            .write_all(format!("[url \"{}\"]\n\tinsteadOf = {url}\n", origin.display()).as_bytes())
            .unwrap();
    }

    /// The project a test starts in, with `open` as its opener and `slots` as its cap
    fn project<'a>(
        &'a self,
        binding: &'a Binding,
        open: &'a dyn Fn(&Opening<'_>) -> Result<Session, String>,
        slots: usize,
    ) -> Project<'a> {
        Project {
            paths: &self.paths,
            folder: &self.repo,
            base: "main",
            tree: &self.tree,
            slots,
            records: &self.records,
            binding,
            network_allowlist: &[],
            reads: Reads::Project,
            tools: None,
            tokens: self.tokens.as_ref(),
            open,
        }
    }
}

/// The opener a test uses: `agent` is the harness, and no sandbox is started
fn on(agent: &FakeAgent, opening: &Opening<'_>) -> Session {
    agent.session(
        opening.worktree.to_path_buf(),
        opening.resume.clone(),
        opening.choice.clone(),
    )
}

/// The worktree's branch, as git reads it there
fn branch_of(worktree: &Path) -> String {
    git(worktree, &["rev-parse", "--abbrev-ref", "HEAD"])
}

#[test]
fn start_prepares_the_worktree_on_its_branch_and_starts_a_judged_session() {
    let rig = Rig::new("start");
    let agent = FakeAgent::new().turn(
        Turn::new()
            .permission_to_write("t1", "Write src/lib.rs", "src/lib.rs")
            .permission_to_write("t2", "Write beside the worktree", "../outside"),
    );
    let binding = binding();
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();

    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();

    let Started::Running {
        record, worktree, ..
    } = started
    else {
        panic!("the ticket was not started: {started:?}");
    };
    assert_eq!(record.branch, "kelpie/74");
    assert_eq!(record.status, Status::Running);
    assert_eq!(record.seat, Seat::Held);
    assert_eq!(
        worktree,
        rig.paths.worktree("74"),
        "the worktree is under the project's state folder"
    );
    assert_eq!(branch_of(&worktree), "kelpie/74");
    assert_eq!(
        git(&worktree, &["rev-parse", "HEAD"]),
        git(&rig.repo, &["rev-parse", "origin/main"]),
        "the branch is cut from the latest origin/main"
    );
    assert!(
        rig.paths.git_dir("74").join("HEAD").is_file(),
        "the clone has a git dir of its own beside the worktree"
    );

    let heard = until_turn_ends(&heard);
    assert!(
        matches!(heard.first(), Some(SessionEvent::Started { .. })),
        "{heard:?}"
    );
    let step = |id: &str| {
        heard
            .iter()
            .find(|event| matches!(event, SessionEvent::ToolStep { id: step, .. } if *step == id))
    };
    assert!(
        matches!(
            step("t1"),
            Some(SessionEvent::ToolStep {
                status: ToolStatus::Done,
                ..
            })
        ),
        "{heard:?}"
    );
    assert!(
        matches!(
            step("t2"),
            Some(SessionEvent::ToolStep {
                status: ToolStatus::Refused,
                reason: Some(_),
                ..
            })
        ),
        "a write outside the worktree is refused with its reason"
    );
    let prompt = agent.params("session/prompt")[0]["prompt"][0]["text"]
        .as_str()
        .expect("the prompt holds no text")
        .to_owned();
    assert!(
        prompt.ends_with("Build the thing"),
        "the ticket's text is the worker's first message: {prompt:?}"
    );
    assert!(
        prompt.starts_with("Do the ticket."),
        "the message carries the binding's instructions: {prompt:?}"
    );
    assert_eq!(
        heard.last(),
        Some(&SessionEvent::TurnEnded {
            stop_reason: StopReason::EndTurn,
            usage: None,
        })
    );

    let kept = kept(&rig);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].ticket, "74");
    assert_eq!(kept[0].branch, "kelpie/74");
    assert!(kept[0].session.is_some(), "the session id the harness gave");
}

#[test]
fn starting_the_same_ticket_again_resumes_its_worktree_and_session() {
    let rig = Rig::new("resume");
    let binding = binding();
    let first = FakeAgent::new().turn(Turn::new().text("first turn"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&first, opening)) };
    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { session, .. } = started else {
        panic!("{started:?}");
    };
    until_turn_ends(&heard);
    let resumed = kept(&rig)[0].session.clone().expect("no session id kept");
    session.stop();

    // What the worker left in its worktree, uncommitted
    let worktree = rig.paths.worktree("74");
    std::fs::write(worktree.join("notes.md"), "kept").unwrap();

    let second = FakeAgent::new()
        .can_resume()
        .knows(&resumed.to_string())
        .turn(Turn::new().text("second turn"));
    let open2 = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&second, opening)) };
    let (sink, heard) = events();
    let again = start(
        &rig.project(&binding, &open2, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(again, Started::Running { .. }), "{again:?}");
    until_turn_ends(&heard);

    assert_eq!(
        std::fs::read_to_string(worktree.join("notes.md")).unwrap(),
        "kept",
        "preparing again leaves the worktree as the worker left it"
    );
    assert_eq!(branch_of(&worktree), "kelpie/74");
    assert_eq!(
        second.params("session/resume")[0]["sessionId"],
        json!(resumed.to_string()),
        "the session resumes under the id its harness gave"
    );
    assert!(
        second.params("session/new").is_empty(),
        "no second session is opened for the same ticket"
    );
    assert_eq!(
        second.params("session/prompt")[0]["prompt"][0]["text"],
        json!("Build the thing"),
        "the resumed turn is prompted with the ticket again"
    );

    let kept = kept(&rig);
    assert_eq!(kept.len(), 1, "the ticket is one worker, not two");
    assert_eq!(kept[0].session.as_ref(), Some(&resumed));
}

#[test]
fn a_start_with_no_free_slot_waits_for_one() {
    let rig = Rig::new("slots");
    let binding = binding();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("first"))
        .turn(Turn::new().text("second"));

    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();
    let first = start(
        &rig.project(&binding, &open, 1),
        &ticket("74", "First ticket"),
        sink,
    )
    .unwrap();
    assert!(matches!(first, Started::Running { .. }), "{first:?}");
    assert!(
        matches!(until_started(&heard), SessionEvent::Started { .. }),
        "the first worker's turn is in flight"
    );

    let (sink, idle) = events();
    let second = start(
        &rig.project(&binding, &open, 1),
        &ticket("75", "Second ticket"),
        sink,
    )
    .unwrap();
    let Started::Waiting { record } = second else {
        panic!("the second ticket took a slot it does not have: {second:?}");
    };
    assert_eq!(record.seat, Seat::Waiting);
    assert_eq!(record.status, Status::Running, "it needs a slot");
    assert_eq!(
        agent.launches(),
        1,
        "nothing runs for a ticket with no slot"
    );
    assert!(
        idle.try_recv().is_err(),
        "no turn ran for a ticket with no slot"
    );

    assert_eq!(
        kept(&rig)
            .iter()
            .map(|r| (r.ticket.as_str(), r.seat))
            .collect::<Vec<_>>(),
        [("74", Seat::Held), ("75", Seat::Waiting)],
        "the queued ticket is kept in line for a slot"
    );

    until_turn_ends(&heard);
    assert_eq!(
        kept(&rig)
            .iter()
            .map(|r| (r.status, r.seat))
            .collect::<Vec<_>>(),
        [
            (Status::Ended, Seat::Without),
            (Status::Running, Seat::Waiting)
        ],
        "the finished worker holds no slot"
    );

    let (sink, heard) = events();
    let again = start(
        &rig.project(&binding, &open, 1),
        &ticket("75", "Second ticket"),
        sink,
    )
    .unwrap();
    assert!(
        matches!(again, Started::Running { .. }),
        "the freed slot went to the ticket in line for it: {again:?}"
    );
    until_turn_ends(&heard);
    assert_eq!(agent.launches(), 2);
}

#[test]
fn a_worker_mid_turn_at_quit_is_paused_on_the_next_launch() {
    let rig = Rig::new("paused");
    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("mid").until_cancelled());
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();

    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(started, Started::Running { .. }), "{started:?}");
    assert!(
        matches!(until_started(&heard), SessionEvent::Started { .. }),
        "the turn is in flight"
    );
    let mid = kept(&rig).remove(0);
    assert_eq!(
        (mid.status, mid.seat),
        (Status::Running, Seat::Held),
        "the record says the worker holds a slot and is mid-turn"
    );

    // The run ends with the turn in flight, and the next launch reads the file
    let mut records = Records::load(rig.paths.records()).unwrap();
    assert_eq!(records.pause_mid_turn().unwrap(), ["74"]);
    assert_eq!(
        records.list(),
        [Record {
            status: Status::Paused,
            seat: Seat::Without,
            ..mid
        }],
        "Paused, and holding no slot"
    );
    assert_eq!(
        Records::load(rig.paths.records()).unwrap().list(),
        records.list(),
        "the pause is kept for the launch after that"
    );
}

#[test]
fn a_start_that_never_begins_gives_its_slot_back() {
    let rig = Rig::new("slot back");
    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("second turn"));
    // What a harness that is not installed does
    let broken = |_opening: &Opening<'_>| -> Result<Session, String> {
        Err("the harness is not installed".to_owned())
    };
    let project = rig.project(&binding, &broken, 1);

    let (sink, heard) = events();
    let error = start(&project, &ticket("74", "First ticket"), sink).expect_err("the opener fails");
    assert!(matches!(error, StartError::Session(_)), "{error:?}");
    assert!(heard.try_recv().is_err(), "nothing ran for it");
    assert!(kept(&rig).is_empty(), "the failed start holds no slot");

    // The slot the failed start held goes to the next ticket
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let project = rig.project(&binding, &open, 1);
    let (sink, heard) = events();
    let second = start(&project, &ticket("75", "Second ticket"), sink).unwrap();
    assert!(matches!(second, Started::Running { .. }), "{second:?}");
    until_turn_ends(&heard);
    assert_eq!(
        kept(&rig)
            .iter()
            .map(|record| (record.ticket.as_str(), record.seat))
            .collect::<Vec<_>>(),
        [("75", Seat::Without)],
        "one worker ran, and it holds no slot now its turn ended"
    );
}

#[test]
fn a_ticket_already_running_is_not_started_twice() {
    let rig = Rig::new("twice");
    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("first turn"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let project = rig.project(&binding, &open, 2);

    let (sink, heard) = events();
    let first = start(&project, &ticket("74", "Build the thing"), sink).unwrap();
    assert!(matches!(first, Started::Running { .. }), "{first:?}");
    assert!(
        matches!(until_started(&heard), SessionEvent::Started { .. }),
        "the turn is in flight"
    );

    // The Start button again, or the PM and the button at once
    let (sink, idle) = events();
    let again = start(&project, &ticket("74", "Build the thing"), sink)
        .expect_err("the ticket's worker is already running");
    assert!(
        matches!(again, StartError::Running { ticket: ref name } if name == "74"),
        "{again:?}"
    );
    assert!(idle.try_recv().is_err(), "no second turn ran");
    assert_eq!(agent.launches(), 1, "one session for one worktree");
    assert_eq!(
        kept(&rig)
            .iter()
            .map(|record| (record.ticket.as_str(), record.status, record.seat))
            .collect::<Vec<_>>(),
        [("74", Status::Running, Seat::Held)],
        "the record is as the running worker left it"
    );
}

#[test]
fn a_ticket_id_is_refused_where_it_collides_with_another_worktree() {
    let rig = Rig::new("names");
    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let project = rig.project(&binding, &open, 2);

    // `74.git` is ticket 74's clone git dir, and the rest are ref names git refuses
    for id in ["74.git", "74.lock", "74..75", "74.", ".74", ""] {
        let (sink, heard) = events();
        let error = start(&project, &ticket(id, "Build the thing"), sink)
            .expect_err("{id} names a folder or a branch kelpie cannot use");
        assert!(matches!(error, StartError::Name { .. }), "{id}: {error:?}");
        assert!(heard.try_recv().is_err(), "{id}: nothing ran");
    }
    assert!(kept(&rig).is_empty(), "a refused id holds nothing");
    assert_eq!(agent.launches(), 0, "nothing ran for a refused id");

    // An id with a dot inside is one kelpie can name a git dir for
    let (sink, heard) = events();
    let dotted = start(&project, &ticket("1.2", "Build the thing"), sink).unwrap();
    assert!(matches!(dotted, Started::Running { .. }), "{dotted:?}");
    until_turn_ends(&heard);
    let mut git_dir = rig.paths.worktree("1.2").as_os_str().to_owned();
    git_dir.push(".git");
    assert_eq!(
        rig.paths.git_dir("1.2"),
        PathBuf::from(git_dir),
        "the clone's git dir is the worktree plus `.git`, not its extension swapped"
    );
    assert!(
        rig.paths.git_dir("1.2").join("HEAD").is_file(),
        "the clone the worktree was prepared into is the one Paths names"
    );
}

#[test]
fn the_record_sits_outside_every_folder_the_worker_writes() {
    let rig = Rig::new("record");
    let records = rig.paths.records();
    let written = [
        rig.paths.worktree("74"),
        rig.paths.git_dir("74"),
        rig.paths.state_dir("74"),
    ];
    for folder in &written {
        assert!(
            !records.starts_with(folder),
            "{records:?} is inside {folder:?}, which the worker writes"
        );
    }

    let harness = Harness::new("fake-agent-acp");
    let reach = sandbox::Project {
        folder: &rig.repo,
        network_allowlist: &[],
        reads: Reads::Project,
        sessions_root: &rig.paths.sessions_root(),
    };
    let folders = sandbox::Worker {
        worktree: &rig.paths.worktree("74"),
        git_dir: &rig.paths.git_dir("74"),
        git_common_dir: &rig.repo.join(".git"),
        state_dir: &rig.paths.state_dir("74"),
        kelpie_home: &rig.paths.data_dir,
        door: None,
        lease_address: None,
    };
    let policy = sandbox::worker::policy(&harness, &reach, &folders).unwrap();

    let written: Vec<String> = written
        .iter()
        .map(|folder| folder.to_string_lossy().into_owned())
        .collect();
    let as_str: Vec<&str> = written.iter().map(String::as_str).collect();
    assert_eq!(
        policy.filesystem.allow_write,
        entries(&as_str).filesystem.allow_write,
        "the worker writes its worktree, its clone's git dir and its state folder, and nothing else"
    );

    // Kelpie's data folder is denied by name, or by the home folder that holds it
    let home = std::env::var_os("HOME").expect("no home folder").into();
    let closers: Vec<String> = [rig.paths.data_dir.clone(), rig.paths.sessions_root(), home]
        .iter()
        .map(|folder| format!("{}/**", folder.display()))
        .collect();
    let closers: Vec<&str> = closers.iter().map(String::as_str).collect();
    let closed = entries(&closers).filesystem.deny_read;
    assert!(
        closed
            .iter()
            .any(|entry| policy.filesystem.deny_read.contains(entry)),
        "kelpie's data folder, which holds the record, is closed to the worker: {:?}",
        policy.filesystem.deny_read
    );

    let record = rig.paths.project();
    let held = entries(&[&format!("{}/**", record.display())])
        .filesystem
        .allow_read
        .unwrap_or_default();
    let allowed = policy.filesystem.allow_read.unwrap_or_default();
    assert!(
        !allowed.iter().any(|entry| held.contains(entry)),
        "no folder the worker may read holds the record: {allowed:?}"
    );
}

#[test]
fn the_instructions_carry_the_base_branch_and_the_hand_over_file() {
    let rig = Rig::new("instructions");
    let binding = binding_on("Run `{kelpie}` on `origin/{base}` and write {pull_request}.");
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();

    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(started, Started::Running { .. }), "{started:?}");
    until_turn_ends(&heard);

    let prompt = agent.params("session/prompt")[0]["prompt"][0]["text"]
        .as_str()
        .expect("the prompt holds no text")
        .to_owned();
    assert!(prompt.contains("origin/main"), "{prompt:?}");
    assert!(
        prompt.contains(&rig.paths.pull_request_file("74").display().to_string()),
        "{prompt:?}"
    );
    assert!(
        !prompt.contains("{base}")
            && !prompt.contains("{kelpie}")
            && !prompt.contains("{pull_request}"),
        "no placeholder reaches the worker: {prompt:?}"
    );
}

/// The variables each start gave its opener, in order
fn envs(held: &Mutex<Vec<Vec<(String, String)>>>) -> Vec<Vec<(String, String)>> {
    held.lock().unwrap().clone()
}

/// `name` in `env`, `None` where kelpie sets no such variable
fn in_env(env: &[(String, String)], name: &str) -> Option<String> {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

/// The token file the worker's `gh` reads, as kelpie wrote it
fn hosts(state_dir: &Path) -> String {
    std::fs::read_to_string(login::config_dir(state_dir).join("hosts.yml")).unwrap()
}

/// The first turn's prompt, which carries the instructions and the ticket's text
fn prompt(agent: &FakeAgent) -> String {
    agent.params("session/prompt")[0]["prompt"][0]["text"]
        .as_str()
        .expect("the prompt holds no text")
        .to_owned()
}

// The worker's credential on GitHub: one repo, minted for this turn alone, in a
// folder of its own that git's credential helper answers from. No variable
// carries it, so a harness that strips token-shaped names, and every process
// that reads another's environment, still sees nothing.
#[test]
fn a_worker_pushes_with_a_token_for_its_one_repo_minted_fresh_for_each_turn() {
    let mut rig = Rig::new("worker token");
    rig.on_github("shep-pm/koji");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("first turn"))
        .turn(Turn::new().text("second turn"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let open = |opening: &Opening<'_>| -> Result<Session, String> {
        seen.lock().unwrap().push(opening.env.clone());
        Ok(on(&agent, opening))
    };

    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { state_dir, .. } = started else {
        panic!("{started:?}");
    };
    until_turn_ends(&heard);
    let first = hosts(&state_dir);
    assert!(first.contains("oauth_token: ghs_test1"), "{first}");
    assert!(first.contains("user: x-access-token"), "{first}");

    let (sink, heard) = events();
    let again = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(again, Started::Running { .. }), "{again:?}");
    until_turn_ends(&heard);

    let second = hosts(&state_dir);
    assert!(second.contains("oauth_token: ghs_test2"), "{second}");
    assert!(!second.contains("ghs_test1"), "{second}");
    assert_eq!(
        github.asked().len(),
        4,
        "each turn mints its own, and kelpie mints no token of its own here"
    );

    let envs = envs(&seen);
    assert_eq!(envs.len(), 2, "each turn gets its own environment");
    for env in &envs {
        let build = rig.paths.build_dir("74");
        let dir = login::config_dir(&state_dir);
        assert_eq!(
            in_env(env, "CARGO_TARGET_DIR").as_deref(),
            Some(build.to_string_lossy().as_ref()),
            "{env:?}"
        );
        assert_eq!(
            in_env(env, "GH_CONFIG_DIR").as_deref(),
            Some(dir.to_string_lossy().as_ref()),
            "the worker's `gh` reads its own folder, not the maintainer's: {env:?}"
        );
        assert_eq!(
            in_env(env, "GIT_TERMINAL_PROMPT").as_deref(),
            Some("0"),
            "{env:?}"
        );
        assert!(
            in_env(env, "GIT_CONFIG_PARAMETERS")
                .unwrap()
                .contains("!gh auth git-credential"),
            "git answers github.com from the worker's own login: {env:?}"
        );
        assert!(
            env.iter().all(|(_, value)| !value.contains("ghs_test")),
            "no variable carries the token: {env:?}"
        );
    }
}

// An App kelpie registered that the repo does not cover leaves the worker with
// nothing to push with. It runs, says so, and git is pointed at no helper at
// all, so nothing the maintainer logged into is ever asked.
#[test]
fn a_worker_with_no_app_on_its_repo_is_told_it_cannot_push() {
    let mut rig = Rig::new("no app");
    rig.on_github("shep-pm/koji");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    // Registered, and installed on another repo alone
    github.registered(home.path(), "other/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let open = |opening: &Opening<'_>| -> Result<Session, String> {
        seen.lock().unwrap().push(opening.env.clone());
        Ok(on(&agent, opening))
    };
    let (sink, heard) = events();

    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { state_dir, .. } = started else {
        panic!("{started:?}");
    };
    until_turn_ends(&heard);

    let prompt = prompt(&agent);
    assert!(
        prompt.contains("no credential for this repo") && prompt.contains("git push"),
        "{prompt:?}"
    );
    assert!(
        !login::config_dir(&state_dir).join("hosts.yml").exists(),
        "no login is written where there is no token"
    );
    let env = &envs(&seen)[0];
    assert_eq!(
        in_env(env, "GIT_CONFIG_PARAMETERS").as_deref(),
        Some(login::NO_HELPER),
        "the maintainer's helpers are emptied, and github.com is answered by nothing: {env:?}"
    );
}

// A repo kelpie cannot name on GitHub gets no token, whatever Apps it has: the
// token is for one repo, and this is not it.
#[test]
fn a_repo_that_is_not_github_gets_the_worker_no_token() {
    let mut rig = Rig::new("not github");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();

    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(started, Started::Running { .. }), "{started:?}");
    until_turn_ends(&heard);

    assert!(
        github.asked().is_empty(),
        "no token was minted for any repo"
    );
    assert!(!login::config_dir(&rig.paths.state_dir("74")).exists());
    assert!(prompt(&agent).contains("no credential for this repo"));
}

// A token GitHub made hold more than pushing is never handed to a worker, and
// the start says why instead of running the turn with it.
#[test]
fn a_token_holding_more_than_pushing_refuses_the_start() {
    let mut rig = Rig::new("too broad");
    rig.on_github("shep-pm/koji");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    github.grants_too("pull_requests", "write");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();

    let error = start(&rig.project(&binding, &open, 2), &ticket("74", "x"), sink)
        .expect_err("GitHub made the token hold pull_requests");
    assert!(
        matches!(error, StartError::Token(TokenError::TooBroad(_))),
        "{error:?}"
    );
    assert!(heard.try_recv().is_err(), "no turn ran with it");
    assert!(kept(&rig).is_empty(), "the refused start holds no slot");
    assert!(
        !login::config_dir(&rig.paths.state_dir("74")).exists(),
        "no login was written"
    );
    assert_eq!(agent.launches(), 0);
}

/// What crabpot makes of `paths` as a list of its own
fn entries(paths: &[&str]) -> Config {
    let settings = json!({
        "network": {"allowedDomains": [], "deniedDomains": []},
        "filesystem": {
            "denyRead": paths, "allowRead": paths, "allowWrite": paths, "denyWrite": paths,
        },
    });
    Config::from_json(&settings.to_string()).unwrap()
}
