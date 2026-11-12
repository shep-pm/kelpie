//! Several tickets at once, through stand-ins for GitHub and the harness
//!
//! Lifted in shape from SK's `runner/several.rs` (`docs/lifted.md`): a rig of a
//! project with a bare git origin and room for some workers, one [`FakeAgent`] for
//! each worker, and the app's own [`Workers`] and [`Poller`] on top. The cases in
//! [`cases`] assert only what the outside world sees: the board's columns, the pull
//! requests opened on [`forge::Github`], and the prompts each worker's agent got.
//!
//! The rig is built after the one in `workers::tests` and its board stand-in in
//! `workers/tests/hand_over.rs`, which are private to those tests. Its add, dispatch
//! and drop cases are not here: they come with the Phase 3 slices that build what
//! they are about.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use kelpie_engine::ci::fake::FakeCi;
use kelpie_engine::ci::{Ci, FailedCheck, FailedLog, HeadStatus};
use kelpie_engine::github::{Clock, Timestamp};
use kelpie_engine::login::Store;
use kelpie_engine::review::Reviewer;
use kelpie_engine::review::fake::FakeReviewer;
use kelpie_engine::run::RunId;
use kelpie_engine::session::Session;
use kelpie_engine::session::fake::FakeAgent;
use kelpie_engine::settings::{ConfigDir, Field, LayerFile, PickUp, Settings};
use kelpie_engine::ticket::{
    FakeQueue, GithubTickets, MergeQueue, OpenPull, PullState, Pulls, TicketId, Tickets,
};
use kelpie_engine::worker::{Opening, Paths, Record, Status, Ticket};
use kelpie_protocol::SessionEvent;
use serde_json::json;

use crate::board::poll::{Choices, Poller, Retry};
use crate::board::{BoardEvent, Column};
use crate::gate::Gate;
use crate::review::Make;
use crate::state::AppState;
use crate::workers::{
    Forge, Hands, NoteOutcome, Open, WorkerStarted, note_through, plan, stop_and_tell_on,
};

mod cases;
mod forge;

use forge::{Github, VIEWER};

/// How long a case waits for something its own turn must cause
const WAIT: Duration = Duration::from_secs(20);

/// The label of a Ready ticket
const READY: &str = "ready-for-agent";

/// The feed watch id the rig attaches under
const WATCH: u32 = 1;

/// Every event a worker's feed carried
type Heard = Arc<Mutex<Vec<SessionEvent>>>;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A clock standing still, which the poller's read of the closed list wants
struct Fixed;

impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::from_secs(1_700_000_000)
    }
}

/// A clock that stands where a case puts it, which the gate's passes read
///
/// A head's checks have time to register once [`SETTLE`] seconds have passed since a
/// pass first saw it, so a case lets that time pass between two reads.
#[derive(Debug, Clone, Default)]
struct Ticks(Arc<AtomicU64>);

impl Ticks {
    fn advance(&self, seconds: u64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for Ticks {
    fn now(&self) -> Timestamp {
        Timestamp::from_secs(1_700_000_000 + self.0.load(Ordering::SeqCst))
    }
}

/// How long the gate waits for a head's checks to register
const SETTLE: u64 = 120;

/// Run `git` in `folder`, as the checkout of a project would be made
fn git(folder: &Path, args: &[&str]) -> String {
    let done = Command::new("git")
        .current_dir(folder)
        .args([
            "-c",
            "user.name=kelpie",
            "-c",
            "user.email=kelpie@example.com",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        done.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&done.stderr)
    );
    String::from_utf8_lossy(&done.stdout).trim().to_owned()
}

/// A project on a bare git origin, with room for some workers, and a GitHub stood in
struct Rig {
    dir: PathBuf,
    state: AppState,
    project: String,

    /// The project's GitHub, which every case shares
    github: Arc<Github>,

    /// The project's CI, which a case scripts the failed logs of
    ci: Arc<FakeCi>,

    /// Where the gate's passes read the present moment from
    clock: Ticks,

    /// The agent each ticket's worker runs on
    agents: Arc<Mutex<BTreeMap<u64, FakeAgent>>>,

    /// The reviewer each ticket's pass reads with, scripted per ticket
    reviewers: Arc<Mutex<BTreeMap<u64, Arc<FakeReviewer>>>>,

    /// What each ticket's worker's feed carried
    heard: Mutex<BTreeMap<u64, Heard>>,
}

impl Rig {
    /// A project whose checkout has an origin holding `main`, with `slots` workers at a time
    fn new(slots: u64) -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-several-{}", RunId::new().unwrap()));
        let checkout = dir.join("checkout");
        fs::create_dir_all(&checkout).unwrap();
        git(
            &dir,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&checkout, &["init", "--quiet", "-b", "main"]);
        // An absolute URL, so a worker's clone can push to it too
        let origin = dir.join("origin.git");
        git(
            &checkout,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        git(
            &checkout,
            &["commit", "--quiet", "--allow-empty", "-m", "init"],
        );
        git(&checkout, &["push", "--quiet", "origin", "main"]);
        let state = AppState::open(
            dir.clone(),
            ConfigDir::at(dir.join("config")),
            Store::memory(),
            Box::new(|_| Err("these cases run no PM".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        state.set_project(checkout).unwrap();
        let project = state.project_id(None).unwrap();
        Settings::set(
            state.config(),
            LayerFile::Project(&project),
            Field::WorkerSlots,
            &json!(slots),
        )
        .unwrap();
        let github = Arc::new(Github::new());
        let ci = Arc::new(FakeCi::new());
        let queue = Arc::new(FakeQueue::new());
        state.workers().stand_in_forge(
            &project,
            Arc::new(Forge::new(
                Arc::clone(&github) as Arc<dyn GithubTickets>,
                Arc::clone(&github) as Arc<dyn Pulls>,
                Arc::clone(&ci) as Arc<dyn Ci>,
                Arc::clone(&queue) as Arc<dyn MergeQueue>,
            )),
        );
        Self {
            dir,
            state,
            project,
            github,
            ci,
            clock: Ticks::default(),
            agents: Arc::default(),
            reviewers: Arc::default(),
            heard: Mutex::default(),
        }
    }

    /// The reviewer ticket `number`'s pass reads with, which a case scripts before a read
    fn reviewer(&self, number: u64) -> Arc<FakeReviewer> {
        let mut reviewers = lock(&self.reviewers);
        Arc::clone(reviewers.entry(number).or_default())
    }

    /// The prompt of each read ticket `number`'s pass ran, in the order they read
    fn read_prompts(&self, number: u64) -> Vec<String> {
        self.reviewer(number).prompts()
    }

    /// Put issue `number` on the board as Ready, with `agent` as its worker's harness
    ///
    /// The worker's hand-over file is left where its sandbox would let it write one,
    /// since a fake agent writes no file: its title is the issue's with `feat: ` before it.
    fn ticket(&self, number: u64, title: &str, body: &str, agent: &FakeAgent) {
        self.github
            .open_issue(number, title, body, &[READY], VIEWER);
        lock(&self.agents).insert(number, agent.clone());
        let file = self.paths().pull_request_file(&number.to_string());
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, format!("feat: {title}\n\nIt is done.\n")).unwrap();
    }

    fn paths(&self) -> Paths {
        Paths::new(self.state.data_dir(), self.project.clone())
    }

    fn feed(&self, number: u64) -> Heard {
        Arc::clone(lock(&self.heard).entry(number).or_default())
    }

    /// Start ticket `number`'s worker, as the window's Start does
    fn start(&self, number: u64) -> WorkerStarted {
        let issue = self
            .github
            .issue(&TicketId::from_issue(number).unwrap())
            .unwrap();
        let ticket = Ticket {
            id: number.to_string(),
            text: format!("{}\n\n{}", issue.title, issue.body),
        };
        let (state, project) = (&self.state, &self.project);
        let folder = state.project_path_of(project).unwrap();
        state
            .workers()
            .start(
                project,
                &folder,
                state.tools(),
                &ticket,
                &|_, opening: &Opening<'_>| Ok(self.session_for(number, opening)),
            )
            .expect("the worker starts")
    }

    /// The session ticket `number`'s worker runs, built the way every start of it builds one
    ///
    /// The feed is watched from here, so no event of the turn is missed. What a turn that
    /// pushed leaves on the forge is the branch at its worktree's commit.
    fn session_for(&self, number: u64, opening: &Opening<'_>) -> Session {
        let agent = lock(&self.agents)
            .get(&number)
            .expect("the ticket was put on the board")
            .clone();
        let heard = self.feed(number);
        self.github.push(
            &format!("kelpie/{number}"),
            &git(opening.worktree, &["rev-parse", "HEAD"]),
        );
        self.state
            .workers()
            .watch(&self.project, &number.to_string(), WATCH, move |event| {
                lock(&heard).push(event);
                true
            })
            .expect("the worker has a record to watch");
        agent.session(
            opening.worktree.to_path_buf(),
            opening.resume.clone(),
            opening.choice.clone(),
        )
    }

    /// Tell ticket `number`'s worker `text` as a Note
    fn note(&self, number: u64, text: &str) -> NoteOutcome {
        let source: &dyn Tickets = self.github.as_ref();
        note_through(
            &self.state,
            &number.to_string(),
            text,
            Ok(source),
            // A worker told while it waits on a ruling is sent back, which starts its session again
            &|_, opening: &Opening<'_>| Ok(self.session_for(number, opening)),
        )
        .unwrap()
    }

    /// Park ticket `number`'s worker the way a ruling does: waiting on the maintainer, with `reason`
    fn rule(&self, number: u64, reason: &str) {
        let records = self.state.workers().records(&self.project).unwrap();
        let mut records = lock(&records);
        let ticket = number.to_string();
        records
            .set_status(&ticket, Status::WaitingOnMaintainer)
            .unwrap();
        records
            .set_reason(&ticket, Some(reason.to_owned()))
            .unwrap();
    }

    /// Ticket `number`'s worker commits `file` and pushes its branch, as a turn that did the work would
    ///
    /// The forge's head for the branch is the pushed commit, and the commit is returned.
    fn push_work(&self, number: u64, file: &str) -> String {
        let worktree = self.paths().worktree(&number.to_string());
        fs::write(worktree.join(file), "mine\n").unwrap();
        git(&worktree, &["add", file]);
        git(
            &worktree,
            &["commit", "--quiet", "-m", &format!("work on {file}")],
        );
        let branch = format!("kelpie/{number}");
        git(
            &worktree,
            &[
                "push",
                "--quiet",
                "origin",
                &format!("HEAD:refs/heads/{branch}"),
            ],
        );
        let head = git(&worktree, &["rev-parse", "HEAD"]);
        self.github.push(&branch, &head);
        head
    }

    /// `main` takes a commit that adds `file`, pushed from a clone of its own
    fn main_moves(&self, file: &str) {
        let other = self.dir.join("other");
        if !other.exists() {
            git(&self.dir, &["clone", "--quiet", "origin.git", "other"]);
        }
        git(&other, &["pull", "--quiet", "origin", "main"]);
        fs::write(other.join(file), "main\n").unwrap();
        git(&other, &["add", file]);
        git(
            &other,
            &["commit", "--quiet", "-m", &format!("main adds {file}")],
        );
        git(&other, &["push", "--quiet", "origin", "main"]);
    }

    /// Whether the pull request from `branch` has been merged on GitHub
    fn merged(&self, branch: &str) -> bool {
        self.github
            .pulls()
            .iter()
            .any(|pull| pull.branch == branch && pull.state == PullState::Merged)
    }

    /// What the origin holds at `branch`
    fn origin(&self, branch: &str) -> String {
        git(&self.dir.join("origin.git"), &["rev-parse", branch])
    }

    /// The parents of the commit `at` names in the origin
    fn origin_parents(&self, at: &str) -> Vec<String> {
        let line = git(
            &self.dir.join("origin.git"),
            &["rev-list", "--parents", "-n", "1", at],
        );
        line.split(' ').skip(1).map(str::to_owned).collect()
    }

    /// Ticket `number`'s record
    fn record(&self, number: u64) -> Record {
        let records = self.state.workers().records(&self.project).unwrap();
        let records = lock(&records);
        records
            .get(&number.to_string())
            .expect("the ticket has a record")
            .clone()
    }

    /// Cut ticket `number`'s turn short and send its worker `text` as the turn that follows
    fn stop_and_tell(&self, number: u64, text: &str) -> NoteOutcome {
        stop_and_tell_on(&self.state, &number.to_string(), text).unwrap()
    }

    /// Wait for `ask` to answer true
    fn until(&self, mut ask: impl FnMut() -> bool) {
        let since = Instant::now();
        while !ask() {
            assert!(since.elapsed() < WAIT, "it never happened");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait for `turns` of ticket `number`'s worker to have ended
    fn until_turns(&self, number: u64, turns: usize) {
        let heard = self.feed(number);
        self.until(|| {
            lock(&heard)
                .iter()
                .filter(|event| matches!(event, SessionEvent::TurnEnded { .. }))
                .count()
                >= turns
        });
    }

    /// Wait for ticket `number`'s worker to have said something, which it does once its
    /// harness has the prompt
    fn until_working(&self, number: u64) {
        let heard = self.feed(number);
        self.until(|| {
            lock(&heard)
                .iter()
                .any(|event| matches!(event, SessionEvent::Text { .. }))
        });
    }

    /// Wait for `count` drafts to be open on GitHub
    fn until_drafts(&self, count: usize) {
        self.until(|| self.github.pulls().len() >= count);
    }

    /// The text of each prompt `agent` was sent, oldest first
    fn prompts(&self, agent: &FakeAgent) -> Vec<String> {
        agent
            .params("session/prompt")
            .iter()
            .map(|call| call["prompt"][0]["text"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The column of each ticket on the board a read of GitHub shows, by ticket
    fn columns(&self) -> Vec<(String, Column)> {
        let Some(BoardEvent::Board(view)) = self.poller().read() else {
            panic!("the board shows nothing");
        };
        let mut columns: Vec<_> = view
            .tickets
            .into_iter()
            .map(|ticket| (ticket.id, ticket.column))
            .collect();
        columns.sort_by(|a, b| a.0.cmp(&b.0));
        columns
    }

    /// A poller of the project's board, whose read hands over for each worker that still
    /// owes the forge a draft
    fn poller(&self) -> Poller<Github> {
        let choose = Box::new(|| {
            Ok(Choices {
                pick_up: PickUp::Me,
                ready_label: READY.to_owned(),
                done_days: 14,
            })
        });
        let hands = self.hands();
        // Each read then runs the gate on the Review tickets, as the app's does
        let gate = self.gate();
        let retry: Retry = Box::new(move |records: &[Record], pulls: &[OpenPull]| {
            hands.retry(records);
            gate.after_read(records, pulls);
        });
        Poller::new(
            Arc::clone(&self.github),
            choose,
            self.state.workers().records(&self.project).unwrap(),
            self.state.workers().requests_of(&self.project),
            retry,
            Arc::new(Fixed),
        )
    }

    /// The project's gate, which a read of the board runs on its Review tickets
    ///
    /// A worker it sends back gets a session on the agent its ticket was put on the
    /// board with, which the worktree's folder names.
    fn gate(&self) -> Arc<Gate> {
        let folder = self.state.project_path_of(&self.project).unwrap();
        let agents = Arc::clone(&self.agents);
        let open: Arc<Open> = Arc::new(move |_, opening: &Opening<'_>| {
            let number: u64 = opening
                .worktree
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.parse().ok())
                .ok_or("the worktree names no ticket")?;
            let agent = lock(&agents)
                .get(&number)
                .cloned()
                .ok_or("the ticket was put on the board")?;
            Ok(agent.session(
                opening.worktree.to_path_buf(),
                opening.resume.clone(),
                opening.choice.clone(),
            ))
        });
        // Each ticket's pass reads with a reviewer of its own, which a case scripts
        let reviewers = Arc::clone(&self.reviewers);
        let make: Arc<Make> = Arc::new(move |ticket, _binding, _plan| {
            let number: u64 = ticket.parse().map_err(|_| "the ticket names no issue")?;
            let reviewer = {
                let mut reviewers = lock(&reviewers);
                Arc::clone(reviewers.entry(number).or_default())
            };
            Ok(reviewer as Arc<dyn Reviewer>)
        });
        self.state
            .workers_arc()
            .gate(
                &self.project,
                &folder,
                None,
                Arc::new(self.clock.clone()),
                open,
                Some(make),
            )
            .expect("the project's gate")
    }

    /// CI fails the check `check` on ticket `number`'s `head`, whose failed log ends with `tail`
    fn fails(&self, number: u64, head: &str, check: &str, tail: &str) {
        let url = format!("https://github.com/shep-pm/kelpie/actions/runs/{number}");
        self.github.script_checks(
            &format!("kelpie/{number}"),
            HeadStatus::Failed(vec![FailedCheck {
                name: check.to_owned(),
                url: Some(url.clone()),
            }]),
        );
        let log = FailedLog {
            check: check.to_owned(),
            run: number,
            url,
            tail: tail.to_owned(),
        };
        self.ci.script_logs(head, false, Ok(vec![log]));
    }

    /// Wait until the gate's passes of the last read have ended
    fn until_passed(&self) {
        self.gate().idle();
    }

    /// What the project's finished turns hand over
    fn hands(&self) -> Hands {
        let plan = plan(self.state.config(), &self.project).unwrap();
        let folder = self.state.project_path_of(&self.project).unwrap();
        self.state
            .workers()
            .hands(&self.project, &folder, &plan)
            .expect("the project's records can be read")
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
