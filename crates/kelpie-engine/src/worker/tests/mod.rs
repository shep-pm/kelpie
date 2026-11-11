//! Starting a worker, on a repo and a harness of each test's own
//!
//! Each test gets a folder with a bare origin holding `main`, a checkout of it,
//! and a [`FakeAgent`] as the harness, so a start prepares a real worktree
//! through git and runs a real session over ACP with no model behind it.

use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
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
use crate::lease::Leases;
use crate::sandbox::{self, Reads};
use crate::session::fake::{FakeAgent, Turn};
use crate::session::{Harness, Session};
use crate::tree::ProcessTree;
use crate::worktree::tests::{Dir, git, process_tree, write_in};

/// How long a test waits for an event its own turn must send
const WAIT: Duration = Duration::from_secs(20);

/// A sink, and the events it was given
pub(super) fn events() -> (
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
pub(super) fn until_turn_ends(heard: &Receiver<SessionEvent>) -> Vec<SessionEvent> {
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
pub(super) fn kept(rig: &Rig) -> Vec<Record> {
    Records::load(rig.paths.records()).unwrap().list().to_vec()
}

/// What a worker runs on when no agent file or project setting says otherwise
pub(super) fn binding() -> Binding {
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
pub(super) fn ticket(id: &str, text: &str) -> Ticket {
    Ticket {
        id: id.to_owned(),
        text: text.to_owned(),
    }
}

/// A project with an origin holding `main`, a checkout of it, and its records file
pub(super) struct Rig {
    _dir: Dir,
    root: PathBuf,
    tree: Arc<ProcessTree>,
    repo: PathBuf,
    paths: Paths,
    records: Arc<Mutex<Records>>,
    // The Apps kelpie mints this project's workers' tokens from, `None` where it
    // keeps none at all
    tokens: Option<AppTokens>,
    // The `gh` a worker's git answers github.com with. Every rig has a stub of one
    // in a `bin` folder of its own, so a start here always sees a machine that can
    // push; the test about a machine that cannot takes it away.
    gh: Option<PathBuf>,
}

impl Rig {
    pub(super) fn new(name: &str) -> Self {
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
        let gh = root.join("bin/gh");
        std::fs::create_dir_all(gh.parent().unwrap()).unwrap();
        std::fs::write(&gh, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _dir: dir,
            root,
            tree,
            repo,
            paths,
            records,
            tokens: None,
            gh: Some(gh),
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
        let origin = self.root.join("origin.git");
        git(&self.repo, &["config", "remote.origin.url", &url]);
        std::fs::OpenOptions::new()
            .append(true)
            .open(self.repo.join(".git/config"))
            .unwrap()
            .write_all(format!("[url \"{}\"]\n\tinsteadOf = {url}\n", origin.display()).as_bytes())
            .unwrap();
    }

    /// The project a test starts in, with `open` as its opener and `slots` as its cap
    pub(super) fn project<'a>(
        &'a self,
        binding: &'a Binding,
        open: &'a dyn Fn(&Opening<'_>) -> Result<Session, String>,
        slots: usize,
    ) -> Project<'a> {
        self.project_with(binding, open, slots, None)
    }

    /// The same, with `leases` holding each worker's lease socket
    fn project_with<'a>(
        &'a self,
        binding: &'a Binding,
        open: &'a dyn Fn(&Opening<'_>) -> Result<Session, String>,
        slots: usize,
        leases: Option<&'a Leases>,
    ) -> Project<'a> {
        Project {
            paths: &self.paths,
            folder: &self.repo,
            base: "main",
            tree: &self.tree,
            slots,
            turn_timeout: Duration::from_secs(60 * 60),
            records: &self.records,
            binding,
            network_allowlist: &[],
            reads: Reads::Project,
            tools: None,
            tokens: self.tokens.as_ref(),
            gh: self.gh.as_deref(),
            leases,
            usage: None,
            open,
        }
    }
}

/// The opener a test uses: `agent` is the harness, and no sandbox is started
pub(super) fn on(agent: &FakeAgent, opening: &Opening<'_>) -> Session {
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
        lease: None,
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

/// A worker mid-turn in `rig`, whose turn the test cancels after writing in its
/// worktree, so the turn's end settles it
struct Mid {
    worktree: PathBuf,
    session: Arc<Session>,
    heard: Receiver<SessionEvent>,
}

/// Start `agent` on ticket `74` in `rig` and wait for its turn to be in flight
fn mid_turn(rig: &Rig, agent: &FakeAgent) -> Mid {
    let binding = binding();
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(agent, opening)) };
    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket("74", "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running {
        session, worktree, ..
    } = started
    else {
        panic!("the ticket was not started: {started:?}");
    };
    assert!(
        matches!(until_started(&heard), SessionEvent::Started { .. }),
        "the turn is in flight"
    );
    Mid {
        worktree,
        session,
        heard,
    }
}

/// Commit `file` on the worker's branch, as its turn leaves it
fn commit(worktree: &Path, file: &str, text: &str) {
    write_in(worktree, file, text);
    git(worktree, &["add", file]);
    git(worktree, &["commit", "--quiet", "-m", file]);
}

/// What the fence made of ticket `74`'s branch, and what its record says
fn settled(rig: &Rig, worktree: &Path) -> (Ending, Vec<(Status, Seat)>) {
    let ending = turn_end::check(&rig.tree, &rig.repo, worktree, "main");
    let kept = kept(rig)
        .iter()
        .map(|record| (record.status, record.seat))
        .collect::<Vec<_>>();
    (ending, kept)
}

#[test]
fn a_branch_that_changes_a_harnesss_own_file_is_held_naming_it() {
    let rig = Rig::new("fence held");
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    let turn = mid_turn(&rig, &agent);

    // What the sandbox's globs miss: a harness's own file, committed on the branch
    commit(&turn.worktree, ".claude/settings.json", "{\"hooks\":{}}\n");
    turn.session.cancel();
    until_turn_ends(&turn.heard);

    let (ending, kept) = settled(&rig, &turn.worktree);
    assert_eq!(
        ending,
        Ending::Held(Held::OwnFiles {
            files: vec![".claude/settings.json".to_owned()],
        }),
        "the branch is held, naming each file"
    );
    let Ending::Held(held) = &ending else {
        panic!("the branch was not held: {ending:?}");
    };
    assert_eq!(
        held.line(),
        "the branch changes a harness's own files: .claude/settings.json (Claude Code)",
        "the held branch names each file and the harness that owns it"
    );
    assert_eq!(
        ending.status(),
        Status::WaitingOnMaintainer,
        "the ticket waits on the maintainer"
    );
    assert_eq!(
        kept,
        [(Status::WaitingOnMaintainer, Seat::Without)],
        "the held ticket gives its slot back"
    );
}

#[test]
fn a_clean_branch_marks_the_worker_done() {
    let rig = Rig::new("fence clean");
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    let turn = mid_turn(&rig, &agent);

    commit(&turn.worktree, "src/lib.rs", "work\n");
    turn.session.cancel();
    until_turn_ends(&turn.heard);

    let (ending, kept) = settled(&rig, &turn.worktree);
    assert_eq!(ending, Ending::Done, "no harness file changed");
    assert_eq!(
        kept,
        [(Status::Ended, Seat::Without)],
        "the worker is done and holds no slot"
    );
}

#[test]
fn a_planted_git_file_does_not_hide_a_fenced_change() {
    let rig = Rig::new("fence planted");
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    let turn = mid_turn(&rig, &agent);
    commit(&turn.worktree, ".claude/settings.json", "{\"hooks\":{}}\n");

    // A repo of the worker's own, whose `main` holds the same file, planted where
    // the worktree's `.git` points: git run in the worktree answers from it
    let planted = rig.root.join("planted");
    git(&rig.root, &["init", "--quiet", "-b", "main", "planted"]);
    commit(&planted, ".claude/settings.json", "{\"hooks\":{}}\n");
    std::fs::write(
        turn.worktree.join(".git"),
        format!("gitdir: {}\n", planted.join(".git").display()),
    )
    .unwrap();

    turn.session.cancel();
    until_turn_ends(&turn.heard);

    assert_eq!(
        git(&turn.worktree, &["rev-parse", "HEAD"]),
        git(&planted, &["rev-parse", "HEAD"]),
        "the planted repo is what a git that finds the worktree's own dirs reads"
    );
    let (ending, kept) = settled(&rig, &turn.worktree);
    assert_eq!(
        ending,
        Ending::Held(Held::OwnFiles {
            files: vec![".claude/settings.json".to_owned()],
        }),
        "kelpie reads the branch through the clone's git dirs, never the planted ones"
    );
    assert_eq!(kept, [(Status::WaitingOnMaintainer, Seat::Without)]);
}

#[test]
fn a_branch_that_lags_a_change_main_made_to_its_own_files_is_held() {
    let rig = Rig::new("fence behind");
    commit(&rig.repo, ".claude/settings.json", "{\"model\":\"a\"}\n");
    git(&rig.repo, &["push", "--quiet", "origin", "main"]);

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
    assert!(matches!(started, Started::Running { .. }), "{started:?}");
    until_turn_ends(&heard);
    let resumed = kept(&rig)[0].session.clone().expect("no session id kept");

    // `main` moves, and the next start brings it to the worktree without merging it
    commit(&rig.repo, ".claude/settings.json", "{\"model\":\"b\"}\n");
    git(&rig.repo, &["push", "--quiet", "origin", "main"]);

    let second = FakeAgent::new()
        .can_resume()
        .knows(&resumed.to_string())
        .turn(Turn::new().text("second turn").until_cancelled());
    let turn = mid_turn(&rig, &second);
    commit(&turn.worktree, "src/lib.rs", "work\n");
    turn.session.cancel();
    until_turn_ends(&turn.heard);

    let (ending, kept) = settled(&rig, &turn.worktree);
    assert_eq!(
        ending,
        Ending::Held(Held::BehindMain),
        "the branch carries main's settings as it cut them, not as main holds them now"
    );
    assert_eq!(kept, [(Status::WaitingOnMaintainer, Seat::Without)]);
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

mod credential;
mod hand_over;
mod lease;
mod needs_you;
mod refused_sandbox;
mod tell;
mod usage;
