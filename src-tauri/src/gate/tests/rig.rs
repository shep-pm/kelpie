//! The gate's test rig: a bare origin, a project checkout, ticket worktrees and stand-ins
//!
//! The origin is a bare repo at an absolute path, so a worker's clone can push to it. The
//! forge is stood in with [`FakePulls`], [`FakeTickets`] and [`FakeCi`], the workers run on
//! a [`FakeAgent`], and the clock stands where a test puts it, so no test needs `gh`, a
//! model, the network or a wait.

use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use kelpie_engine::ci::fake::FakeCi;
use kelpie_engine::ci::{Ci, FailedLog, HeadStatus};
use kelpie_engine::github::{Clock, Timestamp};
use kelpie_engine::login::Store;
use kelpie_engine::run::RunId;
use kelpie_engine::session::fake::FakeAgent;
use kelpie_engine::settings::{CatchUp, ConfigDir, Field, LayerFile, Settings};
use kelpie_engine::ticket::fake::{FakePulls, FakeTickets};
use kelpie_engine::ticket::{ForgeError, GithubTickets, Issue, OpenPull, Pulls, TicketId};
use kelpie_engine::worker::{Paths, Record, Status};
use kelpie_engine::worktree::{self, Remote, Start};
use serde_json::{Value, json};

use crate::gate::Gate;
use crate::state::AppState;
use crate::workers::{Forge, Open};

/// The second the rig's clock starts at
pub(super) const START: u64 = 1_700_000_000;

/// A clock that stands where a test puts it
#[derive(Debug, Clone)]
pub(super) struct Ticks(Arc<AtomicU64>);

impl Ticks {
    pub(super) fn advance(&self, seconds: u64) {
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
pub(super) struct Held {
    inner: FakeCi,
    holding: AtomicBool,
    asked: AtomicUsize,
}

impl Held {
    /// Hold each rerun until [`Held::release`]
    pub(super) fn hold(&self) {
        self.holding.store(true, Ordering::SeqCst);
    }

    pub(super) fn release(&self) {
        self.holding.store(false, Ordering::SeqCst);
    }

    /// How many reruns were asked for, held or not
    pub(super) fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// The runs it was asked to rerun, in order
    pub(super) fn reruns(&self) -> Vec<u64> {
        self.inner.reruns()
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
pub(super) fn git(folder: &Path, args: &[&str]) -> String {
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
pub(super) struct Rig {
    pub(super) root: PathBuf,
    pub(super) state: AppState,
    pub(super) project: String,
    pub(super) pulls: Arc<FakePulls>,
    pub(super) ci: Arc<Held>,
    pub(super) agent: FakeAgent,
    pub(super) clock: Ticks,
    pub(super) gate: Arc<Gate>,
}

impl Rig {
    pub(super) fn new(agent: FakeAgent) -> Self {
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
                tickets as Arc<dyn GithubTickets>,
                Arc::clone(&pulls) as Arc<dyn Pulls>,
                Arc::clone(&ci) as Arc<dyn Ci>,
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
        let gate = state
            .workers_arc()
            .gate(&project, &checkout, None, Arc::new(clock.clone()), open)
            .unwrap();
        Self {
            root,
            state,
            project,
            pulls,
            ci,
            agent,
            clock,
            gate,
        }
    }

    pub(super) fn checkout(&self) -> PathBuf {
        self.root.join("checkout")
    }

    pub(super) fn paths(&self) -> Paths {
        Paths::new(self.state.data_dir(), self.project.clone())
    }

    pub(super) fn worktree(&self, ticket: &str) -> PathBuf {
        self.paths().worktree(ticket)
    }

    /// Set `field` in the project's own settings file
    pub(super) fn set(&self, field: Field, value: &Value) {
        Settings::set(
            self.state.config(),
            LayerFile::Project(&self.project),
            field,
            value,
        )
        .unwrap();
    }

    /// Catch up only on a conflict, as SK's rig does
    pub(super) fn catch_up_on_conflicts(&self) {
        let value = serde_json::to_value(CatchUp::Conflicts).unwrap();
        self.set(Field::CatchUp, &value);
    }

    /// Wait `minutes` for CI before a queued run is stuck
    pub(super) fn ci_waits(&self, minutes: u32) {
        self.set(Field::CiWait, &json!(minutes));
    }

    /// Ticket `ticket`'s worker has ended with `file` committed and pushed on its branch
    ///
    /// Its worktree is cut from `origin/main` as a start cuts it, and the branch is pushed
    /// to the origin, whose head is the one returned.
    pub(super) fn worker_on(&self, ticket: &str, file: &str, text: &str) -> String {
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
    pub(super) fn commit(&self, ticket: &str, file: &str, text: &str) -> String {
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

    /// `main` takes a commit that sets `text` at `file`
    pub(super) fn main_moves(&self, file: &str, text: &str) {
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
    pub(super) fn origin(&self, branch: &str) -> String {
        git(&self.root.join("origin.git"), &["rev-parse", branch])
    }

    /// The pull request the listing shows for `ticket`'s branch at `head`
    pub(super) fn pull(&self, ticket: &str, head: &str, checks: HeadStatus) -> OpenPull {
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

    /// The worker records, as the board reads them
    pub(super) fn records(&self) -> Vec<Record> {
        let records = self.state.workers().records(&self.project).unwrap();
        let listed = records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .list()
            .to_vec();
        listed
    }

    pub(super) fn record(&self, ticket: &str) -> Record {
        self.records()
            .into_iter()
            .find(|record| record.ticket == ticket)
            .expect("the ticket has a record")
    }

    /// A read of GitHub that listed `pulls`, and the passes it started, run to their end
    pub(super) fn read(&self, pulls: &[OpenPull]) {
        self.gate.after_read(&self.records(), pulls);
        self.gate.idle();
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
