//! The gate's test rig: a bare origin, a project checkout, ticket worktrees and stand-ins
//!
//! The origin is a bare repo at an absolute path, so a worker's clone can push to it. The
//! forge is stood in with [`FakePulls`], [`FakeTickets`] and [`FakeCi`], the workers run on
//! a [`FakeAgent`], and the clock stands where a test puts it, so no test needs `gh`, a
//! model, the network or a wait.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use kelpie_engine::ci::fake::FakeCi;
use kelpie_engine::ci::{Ci, FailedLog, HeadStatus};
use kelpie_engine::github::{Clock, Timestamp};
use kelpie_engine::lease::Leases;
use kelpie_engine::login::Store;
use kelpie_engine::review::fake::FakeReviewer;
use kelpie_engine::review::{Finding, Read, Reviewer, ReviewerError, TestRun};
use kelpie_engine::run::RunId;
use kelpie_engine::session::fake::FakeAgent;
use kelpie_engine::settings::{CatchUp, ConfigDir, Field, LayerFile, Settings};
use kelpie_engine::ticket::fake::{FakePulls, FakeTickets};
use kelpie_engine::ticket::{
    FakeQueue, ForgeError, GithubTickets, Issue, MergeQueue, OpenPull, Pulls, TicketId,
};
use kelpie_engine::worker::{Paths, Record, Status};
use kelpie_engine::worktree::{self, Remote, Start};
use serde_json::{Value, json};

use crate::gate::Gate;
use crate::review::{Exit, Kit, Launch, SandboxedTests, Testing};
use crate::state::AppState;
use crate::workers::{Forge, Open};

/// The second the rig's clock starts at
pub(crate) const START: u64 = 1_700_000_000;

/// A clock that stands where a test puts it
#[derive(Debug, Clone)]
pub(crate) struct Ticks(Arc<AtomicU64>);

impl Ticks {
    pub(crate) fn advance(&self, seconds: u64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }
}

impl Clock for Ticks {
    fn now(&self) -> Timestamp {
        Timestamp::from_secs(self.0.load(Ordering::SeqCst))
    }
}

/// [`FakeCi`] whose reruns a test can hold, which is how it sees a pass at work
#[derive(Debug, Default)]
pub(crate) struct Held {
    inner: FakeCi,
    holding: AtomicBool,
    asked: AtomicUsize,
}

impl Held {
    /// Hold each rerun until [`Held::release`]
    pub(crate) fn hold(&self) {
        self.holding.store(true, Ordering::SeqCst);
    }

    pub(crate) fn release(&self) {
        self.holding.store(false, Ordering::SeqCst);
    }

    /// How many reruns were asked for, held or not
    pub(crate) fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// The runs it was asked to rerun, in order
    pub(crate) fn reruns(&self) -> Vec<u64> {
        self.inner.reruns()
    }

    /// The failed logs `head` has, which a pass reads for a red run
    pub(crate) fn script_logs(&self, head: &str, logs: Vec<FailedLog>) {
        self.inner.script_logs(head, false, Ok(logs));
    }
}

impl Ci for Held {
    fn head_status(&self, head: &str) -> Result<HeadStatus, ForgeError> {
        self.inner.head_status(head)
    }

    fn rerun(&self, run: u64) -> Result<(), ForgeError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let since = Instant::now();
        while self.holding.load(Ordering::SeqCst) && since.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        self.inner.rerun(run)
    }

    fn logs(&self, head: &str, merge_group: bool) -> Result<Vec<FailedLog>, ForgeError> {
        self.inner.logs(head, merge_group)
    }
}

/// Run `git` in `folder`, as the checkout of a project would be made
pub(crate) fn git(folder: &Path, args: &[&str]) -> String {
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

/// A project on a repo with an origin, ticket workers on it, and a gate stood on stand-ins
pub(crate) struct Rig {
    pub(crate) root: PathBuf,
    pub(crate) state: AppState,
    pub(crate) project: String,
    pub(crate) pulls: Arc<FakePulls>,
    pub(crate) tickets: Arc<FakeTickets>,
    pub(crate) ci: Arc<Held>,
    pub(crate) queue: Arc<FakeQueue>,
    pub(crate) agent: FakeAgent,
    pub(crate) clock: Ticks,
    pub(crate) gate: Arc<Gate>,

    /// Each ticket's reviewer, which a test scripts the answers of
    reviewers: Arc<Mutex<BTreeMap<String, Arc<Reader>>>>,

    /// Where each test run was launched, and what it exited with
    pub(crate) launches: Arc<Launches>,

    /// The door of each lease a test run was given, in the order they were made
    pub(crate) doors: Arc<Mutex<Vec<String>>>,

    /// Why the next test run's settings cannot be made, when a test says they cannot
    unreachable: Arc<Mutex<Option<String>>>,

    /// Holds the leases the runs queue on
    _leases: Arc<Leases>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A ticket's reviewer: a [`FakeReviewer`] a test can hold in flight, so a case can see a
/// read at work rather than only after it
#[derive(Debug, Default)]
pub(crate) struct Reader {
    fake: FakeReviewer,
    holding: AtomicBool,
    asked: AtomicBool,

    /// The files the next read writes into its tests folder, as a reviewer that attaches a
    /// failing test to a finding does
    attach: Mutex<Vec<(String, String)>>,
}

impl Reader {
    /// Hold each read until [`Reader::release`]
    pub(crate) fn hold(&self) {
        self.holding.store(true, Ordering::SeqCst);
    }

    pub(crate) fn release(&self) {
        self.holding.store(false, Ordering::SeqCst);
    }

    /// Whether a read has begun
    pub(crate) fn asked(&self) -> bool {
        self.asked.load(Ordering::SeqCst)
    }

    /// What the next read not yet answered answers
    pub(crate) fn script_read(&self, answer: Result<Vec<Finding>, ReviewerError>) {
        self.fake.script_read(answer);
    }

    /// The prompt of each read, in the order they ran
    pub(crate) fn prompts(&self) -> Vec<String> {
        self.fake.prompts()
    }

    /// The next read writes `text` at `path` in its tests folder, if it is given one
    pub(crate) fn attach(&self, path: &str, text: &str) {
        lock(&self.attach).push((path.to_owned(), text.to_owned()));
    }
}

impl Reviewer for Reader {
    fn read(&self, read: &Read<'_>) -> Result<Vec<Finding>, ReviewerError> {
        self.asked.store(true, Ordering::SeqCst);
        let since = Instant::now();
        while self.holding.load(Ordering::SeqCst) && since.elapsed() < Duration::from_secs(10) {
            thread::sleep(Duration::from_millis(10));
        }
        let attached: Vec<_> = lock(&self.attach).drain(..).collect();
        if let Some(folder) = read.tests {
            for (path, text) in attached {
                let at = folder.join(path);
                fs::create_dir_all(at.parent().unwrap()).unwrap();
                fs::write(at, text).unwrap();
            }
        }
        self.fake.read(read)
    }
}

/// One test run the rig's sandbox stand-in was asked for
#[derive(Debug, Clone)]
pub(crate) struct Launched {
    /// The folder it ran in
    pub(crate) cwd: PathBuf,

    /// What the shell was given
    pub(crate) script: String,

    /// The door of the lease it queued on
    pub(crate) door: String,

    /// How long it could run once it held the lease
    pub(crate) ceiling: Duration,

    /// The watched paths the folder held as it ran
    pub(crate) held: Vec<PathBuf>,
}

/// What a test run is launched through in the rig: no sandbox and no `kelpie lease run`, which
/// a unit test cannot start, only the exit each run is scripted to have
#[derive(Debug, Default)]
pub(crate) struct Launches {
    exits: Mutex<VecDeque<Exit>>,
    watched: Mutex<Vec<PathBuf>>,
    ran: Mutex<Vec<Launched>>,
}

impl Launches {
    /// What the next runs exit with, one each. A run with none left exits 0.
    pub(crate) fn script(&self, exits: impl IntoIterator<Item = Exit>) {
        lock(&self.exits).extend(exits);
    }

    /// Notes, for every later run, whether its folder holds `path`
    pub(crate) fn watch(&self, path: &str) {
        lock(&self.watched).push(PathBuf::from(path));
    }

    /// Every run asked for, oldest first
    pub(crate) fn ran(&self) -> Vec<Launched> {
        lock(&self.ran).clone()
    }
}

/// The launch of a rig's sandbox stand-in, scripted through its [`Launches`]
pub(crate) struct Through(pub(crate) Arc<Launches>);

impl Launch for Through {
    fn launch(
        &self,
        run: &TestRun,
        door: &kelpie_engine::tools::Door,
        script: &str,
    ) -> Result<Exit, String> {
        let held = (lock(&self.0.watched).iter())
            .filter(|path| run.cwd.join(path).exists())
            .cloned()
            .collect();
        lock(&self.0.ran).push(Launched {
            cwd: run.cwd.clone(),
            script: script.to_owned(),
            door: door.to_string(),
            ceiling: run.ceiling,
            held,
        });
        Ok(lock(&self.0.exits).pop_front().unwrap_or(Exit {
            code: 0,
            output: Vec::new(),
        }))
    }
}

/// The reviewer builder the rig's gate reads with: each ticket's own [`Reader`]
fn scripted(reviewers: &Arc<Mutex<BTreeMap<String, Arc<Reader>>>>) -> Arc<crate::review::Make> {
    let reviewers = Arc::clone(reviewers);
    Arc::new(move |ticket, _binding, _plan| {
        let reader = {
            let mut reviewers = lock(&reviewers);
            Arc::clone(reviewers.entry(ticket.to_owned()).or_default())
        };
        Ok(reader as Arc<dyn Reviewer>)
    })
}

impl Rig {
    pub(crate) fn new(agent: FakeAgent) -> Self {
        let dir = std::env::temp_dir().join(format!("kelpie-gate-{}", RunId::new().unwrap()));
        fs::create_dir_all(&dir).unwrap();
        let root = dir.canonicalize().unwrap();
        let checkout = root.join("checkout");
        git(
            &root,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        );
        git(&root, &["clone", "--quiet", "origin.git", "checkout"]);
        fs::write(checkout.join("shared.txt"), "base\n").unwrap();
        git(&checkout, &["add", "."]);
        git(&checkout, &["commit", "--quiet", "-m", "init"]);
        git(&checkout, &["push", "--quiet", "origin", "main"]);
        let state = AppState::open(
            root.clone(),
            ConfigDir::at(root.join("config")),
            Store::memory(),
            Box::new(|_| Err("these tests run no PM".to_owned())),
            Box::new(|| Ok(None)),
            None,
        )
        .unwrap();
        state.set_project(checkout.clone()).unwrap();
        let project = state.project_id(None).unwrap();
        let pulls = Arc::new(FakePulls::default());
        let ci = Arc::new(Held::default());
        let queue = Arc::new(FakeQueue::new());
        let tickets = Arc::new(FakeTickets::new());
        for number in [7, 8] {
            tickets.script_issue(
                TicketId::from_issue(number).unwrap(),
                Ok(Issue {
                    title: format!("Ticket {number}"),
                    body: "Build it.".to_owned(),
                    labels: Vec::new(),
                    open: true,
                }),
            );
        }
        state.workers().stand_in_forge(
            &project,
            Arc::new(Forge::new(
                Arc::clone(&tickets) as Arc<dyn GithubTickets>,
                Arc::clone(&pulls) as Arc<dyn Pulls>,
                Arc::clone(&ci) as Arc<dyn Ci>,
                Arc::clone(&queue) as Arc<dyn MergeQueue>,
            )),
        );
        let clock = Ticks(Arc::new(AtomicU64::new(START)));
        let open: Arc<Open> = {
            let agent = agent.clone();
            Arc::new(move |_on, opening| {
                Ok(agent.session(
                    opening.worktree.to_path_buf(),
                    opening.resume.clone(),
                    opening.choice.clone(),
                ))
            })
        };
        let reviewers: Arc<Mutex<BTreeMap<String, Arc<Reader>>>> = Arc::default();
        let launches = Arc::new(Launches::default());
        let doors: Arc<Mutex<Vec<String>>> = Arc::default();
        let unreachable: Arc<Mutex<Option<String>>> = Arc::default();
        let leases = Arc::new(Leases::new(
            &Settings::load(state.config(), &project).unwrap(),
        ));
        state.workers().stand_in_testing(
            &project,
            Testing {
                runner: Arc::new(SandboxedTests::through(Box::new(Through(Arc::clone(
                    &launches,
                ))))),
                gear: Arc::new({
                    let (leases, doors, unreachable) = (
                        Arc::clone(&leases),
                        Arc::clone(&doors),
                        Arc::clone(&unreachable),
                    );
                    let sockets = root.join("leases");
                    move |ticket, _plan| {
                        if let Some(why) = lock(&unreachable).clone() {
                            return Err(why);
                        }
                        let folder = sockets.join(ticket);
                        fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
                        let lease = leases.session(&folder).map_err(|e| e.to_string())?;
                        lock(&doors).push(lease.door().to_string());
                        Ok(Kit {
                            reach: crabpot::Config::default(),
                            lease: Some(lease),
                        })
                    }
                }),
            },
        );
        let gate = state
            .workers_arc()
            .gate(
                &project,
                &checkout,
                None,
                Arc::new(clock.clone()),
                open,
                Some(scripted(&reviewers)),
            )
            .unwrap();
        Self {
            root,
            state,
            project,
            pulls,
            tickets,
            ci,
            queue,
            agent,
            clock,
            gate,
            reviewers,
            launches,
            doors,
            unreachable,
            _leases: leases,
        }
    }

    /// The project's Test command is `command`, so reviewers are asked for tests
    pub(crate) fn test_command(&self, command: &str) {
        self.set(Field::TestCommand, &json!(command));
    }

    /// The next test runs cannot have settings of their own, for `why`
    pub(crate) fn runs_unreachable(&self, why: &str) {
        *lock(&self.unreachable) = Some(why.to_owned());
    }

    /// The reviewer ticket `ticket`'s pass reads with, which a test scripts before a read
    pub(crate) fn reader(&self, ticket: &str) -> Arc<Reader> {
        let mut reviewers = lock(&self.reviewers);
        Arc::clone(reviewers.entry(ticket.to_owned()).or_default())
    }

    /// The line each Review ticket shows, by ticket
    pub(crate) fn lines(&self) -> BTreeMap<String, kelpie_engine::gate::Line> {
        self.gate.lines()
    }

    /// The project's review pass, whose `review.json` a test reads back
    pub(crate) fn review(&self) -> Arc<crate::review::Review> {
        self.gate.review()
    }

    /// The project lists `names` as its reviewers, in the order they read
    pub(crate) fn reviewers_listed(&self, names: &[&str]) {
        self.set(Field::Reviewers, &json!(names));
    }

    /// A reviewer agent named `name`, with `extra` as the keys its file adds
    ///
    /// A listed reviewer needs a file with `role: reviewer` behind it, or the lineup leaves
    /// it out and the pass has no reader.
    pub(crate) fn reviewer_agent(&self, name: &str, extra: &str) {
        let folder = self.state.config().path().join("agents");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            self.agent_file(name),
            format!("---\nrole: reviewer{extra}\n---\n\nRead the diff and say what is wrong.\n"),
        )
        .unwrap();
    }

    /// The agent file `name` is read from, which a test can write what it likes into
    pub(crate) fn agent_file(&self, name: &str) -> PathBuf {
        self.state
            .config()
            .path()
            .join("agents")
            .join(format!("{name}.md"))
    }

    pub(crate) fn checkout(&self) -> PathBuf {
        self.root.join("checkout")
    }

    pub(crate) fn paths(&self) -> Paths {
        Paths::new(self.state.data_dir(), self.project.clone())
    }

    pub(crate) fn worktree(&self, ticket: &str) -> PathBuf {
        self.paths().worktree(ticket)
    }

    /// Set `field` in the project's own settings file
    pub(crate) fn set(&self, field: Field, value: &Value) {
        Settings::set(
            self.state.config(),
            LayerFile::Project(&self.project),
            field,
            value,
        )
        .unwrap();
    }

    /// Catch up only on a conflict, as SK's rig does
    pub(crate) fn catch_up_on_conflicts(&self) {
        let value = serde_json::to_value(CatchUp::Conflicts).unwrap();
        self.set(Field::CatchUp, &value);
    }

    /// Wait `minutes` for CI before a queued run is stuck
    pub(crate) fn ci_waits(&self, minutes: u32) {
        self.set(Field::CiWait, &json!(minutes));
    }

    /// Ticket `ticket`'s worker has ended with `file` committed and pushed on its branch
    ///
    /// Its worktree is cut from `origin/main` as a start cuts it, and the branch is pushed
    /// to the origin, whose head is the one returned.
    pub(crate) fn worker_on(&self, ticket: &str, file: &str, text: &str) -> String {
        let branch = format!("kelpie/{ticket}");
        worktree::prepare(
            &self.state.tree(),
            &self.checkout(),
            &self.worktree(ticket),
            &branch,
            Start::Main,
            "main",
            &Remote::Origin,
            &self.paths().build_dir(ticket),
        )
        .unwrap();
        let head = self.commit(ticket, file, text);
        let records = self.state.workers().records(&self.project).unwrap();
        let mut record = Record::new(ticket, branch);
        record.status = Status::Ended;
        records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keep(record)
            .unwrap();
        head
    }

    /// The worker on `ticket` commits `text` at `file` and pushes, and the pushed head is returned
    pub(crate) fn commit(&self, ticket: &str, file: &str, text: &str) -> String {
        let worktree = self.worktree(ticket);
        fs::write(worktree.join(file), text).unwrap();
        git(&worktree, &["add", file]);
        git(
            &worktree,
            &["commit", "--quiet", "-m", &format!("work on {file}")],
        );
        git(
            &worktree,
            &[
                "push",
                "--quiet",
                "origin",
                &format!("HEAD:refs/heads/kelpie/{ticket}"),
            ],
        );
        git(&worktree, &["rev-parse", "HEAD"])
    }

    /// The worker on `ticket` commits what it staged and pushes, and the pushed head is returned
    pub(crate) fn commit_staged(&self, ticket: &str) -> String {
        let worktree = self.worktree(ticket);
        git(&worktree, &["commit", "--quiet", "-m", "more work"]);
        git(
            &worktree,
            &[
                "push",
                "--quiet",
                "origin",
                &format!("HEAD:refs/heads/kelpie/{ticket}"),
            ],
        );
        git(&worktree, &["rev-parse", "HEAD"])
    }

    /// Set the status of `ticket`'s record, as the maintainer's Resume or a turn would
    pub(crate) fn set_status(&self, ticket: &str, status: Status) {
        let records = self.state.workers().records(&self.project).unwrap();
        records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .set_status(ticket, status)
            .unwrap();
    }

    /// The worker on `ticket` waits on the maintainer for `reason`, as a question or a hand-over's ruling would
    pub(crate) fn park(&self, ticket: &str, reason: &str) {
        let records = self.state.workers().records(&self.project).unwrap();
        let mut records = records.lock().unwrap_or_else(PoisonError::into_inner);
        records
            .set_status(ticket, Status::WaitingOnMaintainer)
            .unwrap();
        records.set_reason(ticket, Some(reason.to_owned())).unwrap();
    }

    /// `main` takes a commit that sets `text` at `file`
    pub(crate) fn main_moves(&self, file: &str, text: &str) {
        let other = self.root.join("other");
        if !other.exists() {
            git(&self.root, &["clone", "--quiet", "origin.git", "other"]);
        }
        git(&other, &["pull", "--quiet", "origin", "main"]);
        fs::write(other.join(file), text).unwrap();
        git(&other, &["add", file]);
        git(
            &other,
            &["commit", "--quiet", "-m", &format!("main sets {file}")],
        );
        git(&other, &["push", "--quiet", "origin", "main"]);
    }

    /// What the origin holds at `branch`
    pub(crate) fn origin(&self, branch: &str) -> String {
        git(&self.root.join("origin.git"), &["rev-parse", branch])
    }

    /// The pull request the listing shows for `ticket`'s branch at `head`
    pub(crate) fn pull(&self, ticket: &str, head: &str, checks: HeadStatus) -> OpenPull {
        OpenPull {
            number: NonZeroU64::new(100 + ticket.parse::<u64>().unwrap()).unwrap(),
            branch: format!("kelpie/{ticket}"),
            draft: false,
            closes: vec![TicketId::from_issue(ticket.parse().unwrap()).unwrap()],
            head: head.to_owned(),
            base: "main".to_owned(),
            labels: Vec::new(),
            checks,
        }
    }

    /// Whether the gate read `ticket`'s pull request merged, and has its cleanup to finish
    pub(crate) fn seen_merged(&self, ticket: &str) -> bool {
        self.gate
            .kept(ticket)
            .is_some_and(|kept| kept.merged.is_some())
    }

    /// The worker records, as the board reads them
    pub(crate) fn records(&self) -> Vec<Record> {
        let records = self.state.workers().records(&self.project).unwrap();
        let guard = records.lock().unwrap_or_else(PoisonError::into_inner);
        guard.list().to_vec()
    }

    pub(crate) fn record(&self, ticket: &str) -> Record {
        self.records()
            .into_iter()
            .find(|record| record.ticket == ticket)
            .expect("the ticket has a record")
    }

    /// A read of GitHub that listed `pulls`, and the passes it started, run to their end
    pub(crate) fn read(&self, pulls: &[OpenPull]) {
        self.gate.after_read(&self.records(), pulls);
        self.gate.idle();
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
