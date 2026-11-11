//! The draft a finished turn hands over, and what its worker is told instead
//!
//! Each test starts a real worker through [`Workers::start`] on a [`FakeAgent`],
//! with a [`FakePulls`] and a [`FakeTickets`] stood in for the project's GitHub, so
//! none needs `gh`, a model or the network. The draft is opened on a thread of its
//! own, which is what the tests that name a thread are about.
//!
//! The last three are about the ruling a repeated refusal draws: the worker is told
//! once, its next turn answers the telling, and a refusal that comes back the same is
//! the worker leaving its file as it was.

use std::collections::BTreeMap;
use std::fs;
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use kelpie_engine::github::{Clock, ForgeSlug, Timestamp};
use kelpie_engine::session::fake::{FakeAgent, Turn};
use kelpie_engine::settings::PickUp;
use kelpie_engine::ticket::fake::{FakePulls, FakeTickets, PullCall};
use kelpie_engine::ticket::{
    ForgeError, GithubTickets, Issue, Listed, OpenPull, Pulls, TicketId, Tickets,
};
use kelpie_engine::worker::{Opening, Paths, Record, Seat, Status, Ticket, owes_a_draft};
use kelpie_protocol::SessionEvent;

use super::{Rig, git, git_out, id, issue};
use crate::board::poll::{Choices, Feed, Poller, Retry};
use crate::workers::{Forge, Hands, OpenOn, WorkerStarted, lock, plan};

/// How long a test waits for a hand-over its own turn must cause
const WAIT: Duration = Duration::from_secs(20);

/// The ticket every test here starts
const TICKET: &str = "12";

/// The branch ticket 12's worker works on
const BRANCH: &str = "kelpie/12";

/// The feed watch id these tests attach under
const WATCH: u32 = 1;

/// A hand-over file with a title kelpie opens a draft from
const FILE: &str = "feat(board): open the draft\n\nIt opens the draft as the maintainer.\n";

/// Which [`Pulls`] method a call was
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Asked {
    Create,
    Head,
    Edit,
}

/// [`FakePulls`], keeping the thread each of its calls came from
///
/// A hand-over runs git and `gh`, so it must not run on the session's sink thread.
/// The thread a call arrived on is the only way to see that from the outside.
#[derive(Debug, Default)]
struct OnThreads {
    pulls: FakePulls,
    asked: Mutex<Vec<(Asked, ThreadId)>>,
}

impl OnThreads {
    /// What the stand-in was asked, in order
    fn calls(&self) -> Vec<PullCall> {
        self.pulls.calls()
    }

    /// Each call and the thread it came from, in order
    fn threads(&self) -> Vec<(Asked, ThreadId)> {
        lock(&self.asked).clone()
    }

    /// How many times a draft was opened
    fn creates(&self) -> usize {
        self.threads()
            .iter()
            .filter(|(asked, _)| *asked == Asked::Create)
            .count()
    }

    /// What [`Pulls::branch_head`] answers for the ticket's branch
    fn script_head(&self, head: &str) {
        self.pulls
            .script_branch_head(BRANCH, Ok(Some(head.to_owned())));
    }

    /// What [`Pulls::create_draft`] answers
    fn script_create(&self, answer: Result<NonZeroU64, ForgeError>) {
        self.pulls.script_create(answer);
    }

    fn heard(&self, asked: Asked) {
        lock(&self.asked).push((asked, thread::current().id()));
    }
}

impl Pulls for OnThreads {
    fn create_draft(
        &self,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<NonZeroU64, ForgeError> {
        self.heard(Asked::Create);
        self.pulls.create_draft(head, base, title, body)
    }

    fn branch_head(&self, branch: &str) -> Result<Option<String>, ForgeError> {
        self.heard(Asked::Head);
        self.pulls.branch_head(branch)
    }

    fn edit(&self, number: NonZeroU64, title: &str, body: &str) -> Result<(), ForgeError> {
        self.heard(Asked::Edit);
        self.pulls.edit(number, title, body)
    }
}

/// An event the worker's feed carried, and the thread it arrived on
#[derive(Debug, Clone)]
struct Seen {
    event: SessionEvent,

    /// The session's own event sink thread, which is where the feed forwards from
    on: ThreadId,
}

/// Every event each test's worker carried
type Heard = Arc<Mutex<Vec<Seen>>>;

/// The tickets holding one open issue 12
fn tickets() -> FakeTickets {
    let tickets = FakeTickets::new();
    tickets.script_issue(id(12), issue("Open the draft", "As the maintainer.", true));
    tickets
}

/// Where the project's workers live
fn paths(rig: &Rig) -> Paths {
    Paths::new(rig.state.data_dir(), rig.project.clone())
}

/// Put `text` in ticket 12's hand-over file, as its worker would
fn left(rig: &Rig, text: &str) {
    let file = paths(rig).pull_request_file(TICKET);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, text).unwrap();
}

/// The text of `at`'s prompt to the agent, oldest first, `None` before it arrives
fn prompt(agent: &FakeAgent, at: usize) -> Option<String> {
    agent
        .params("session/prompt")
        .into_iter()
        .nth(at)
        .and_then(|params| params["prompt"][0]["text"].as_str().map(str::to_owned))
}

/// Wait for `ask` to answer, and return what it answered
fn until_some<T>(mut ask: impl FnMut() -> Option<T>) -> T {
    let since = Instant::now();
    loop {
        if let Some(found) = ask() {
            return found;
        }
        assert!(since.elapsed() < WAIT, "it never happened");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait for the worker's turn to end
fn until_turn_ends(heard: &Heard) {
    until_some(|| {
        lock(heard)
            .iter()
            .any(|seen| matches!(seen.event, SessionEvent::TurnEnded { .. }))
            .then_some(())
    });
}

/// The thread the worker's turn end reached its feed on
fn sink_thread(heard: &Heard) -> ThreadId {
    until_some(|| {
        lock(heard)
            .iter()
            .find(|seen| matches!(seen.event, SessionEvent::TurnEnded { .. }))
            .map(|seen| seen.on)
    })
}

/// Wait for ticket 12's worker to be ruled on, and return its record
///
/// A ruling is what a hand-over makes of a refusal the worker was told and left as it
/// was: the worker waits on the maintainer, naming the refusal, and holds no slot.
fn ruled(rig: &Rig) -> Record {
    until_some(|| {
        let record = rig.record();
        (record.status == Status::WaitingOnMaintainer).then_some(record)
    })
}

/// The project's board source, stood in for what the poller reads
///
/// kelpie builds a [`Poller`]'s feed on a project's [`GithubSource`], which a test
/// has no `gh` to make. This answers the same calls from a [`FakeTickets`].
struct Board {
    tickets: Arc<FakeTickets>,
    repo: ForgeSlug,
}

impl Tickets for Board {
    fn issue(&self, id: &TicketId) -> Result<Issue, ForgeError> {
        self.tickets.issue(id)
    }

    fn ready_issues(&self, label: &str) -> Result<Vec<Listed>, ForgeError> {
        self.tickets.ready_issues(label)
    }
}

impl GithubTickets for Board {
    fn open_issues(&self) -> Result<Vec<Listed>, ForgeError> {
        self.tickets.open_issues()
    }

    fn closed_issues(&self, since: Timestamp) -> Result<Vec<Listed>, ForgeError> {
        self.tickets.closed_issues(since)
    }

    fn open_pulls(&self) -> Result<Vec<OpenPull>, ForgeError> {
        self.tickets.open_pulls()
    }
}

impl Feed for Board {
    fn repo(&self) -> &ForgeSlug {
        &self.repo
    }

    fn viewer(&self) -> Result<String, ForgeError> {
        Ok("octocat".to_owned())
    }

    fn lifts_at(&self) -> Option<Timestamp> {
        None
    }
}

struct Fixed(u64);

impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::from_secs(self.0)
    }
}

impl Rig {
    /// Stand `tickets` and `pulls` in for the project's GitHub
    ///
    /// kelpie builds a project's forge from its origin and the `gh` on this machine,
    /// which these tests have neither of.
    fn stand_in(&self, tickets: &Arc<FakeTickets>, pulls: &Arc<OnThreads>) {
        self.state.workers().stand_in_forge(
            &self.project,
            Arc::new(Forge::new(
                Arc::clone(tickets) as Arc<dyn GithubTickets>,
                Arc::clone(pulls) as Arc<dyn Pulls>,
            )),
        );
    }

    /// Start ticket 12's worker on `agent`, with `pulls` as the project's drafts
    ///
    /// The opener scripts the forge's answer for the ticket's branch as the commit its
    /// worktree holds, which is what a turn that pushed leaves on the forge. The feed
    /// is watched from inside the opener, so no event of the turn is missed, and each
    /// event is kept with the thread it arrived on.
    fn started(&self, agent: &FakeAgent, pulls: &Arc<OnThreads>, heard: &Heard) -> WorkerStarted {
        let (state, project) = (&self.state, &self.project);
        let (pulls, heard) = (Arc::clone(pulls), Arc::clone(heard));
        let opener = &|_on: &OpenOn<'_>, opening: &Opening<'_>| {
            pulls.script_head(&git_out(opening.worktree, &["rev-parse", "HEAD"]));
            let heard = Arc::clone(&heard);
            state
                .workers()
                .watch(project, TICKET, WATCH, move |event| {
                    lock(&heard).push(Seen {
                        event,
                        on: thread::current().id(),
                    });
                    true
                })
                .expect("the worker has a record to watch");
            Ok(agent.session(
                opening.worktree.to_path_buf(),
                opening.resume.clone(),
                opening.choice.clone(),
            ))
        };
        let ticket = Ticket {
            id: TICKET.to_owned(),
            text: "Open the draft.\n\nAs the maintainer.".to_owned(),
        };
        self.state
            .workers()
            .start(
                &self.project,
                &self.dir.join("checkout"),
                None,
                &ticket,
                opener,
            )
            .expect("the worker starts")
    }

    /// What the project's finished turns hand over
    fn hands(&self) -> Hands {
        let plan = plan(self.config(), &self.project).unwrap();
        let folder = self.state.project_path_of(&self.project).unwrap();
        self.state
            .workers()
            .hands(&self.project, &folder, &plan)
            .expect("the project's records can be read")
    }

    /// The worker records, oldest first
    fn listed(&self) -> Vec<Record> {
        let records = self.state.workers().records(&self.project).unwrap();
        records.lock().unwrap().list().to_vec()
    }

    /// Ticket 12's record
    fn record(&self) -> Record {
        self.listed()
            .into_iter()
            .find(|record| record.ticket == TICKET)
            .expect("ticket 12 has a record")
    }

    /// A poller of the project's board, whose read hands over for each worker that
    /// still owes the forge a draft
    fn poller(&self, tickets: &Arc<FakeTickets>, hands: &Hands) -> Poller<Board> {
        let choose = Box::new(|| {
            Ok(Choices {
                pick_up: PickUp::Me,
                ready_label: "ready-for-agent".to_owned(),
                done_days: 14,
            })
        });
        let requests = Box::new(BTreeMap::new);
        let hands = hands.clone();
        let retry: Retry = Box::new(move |records: &[Record]| hands.retry(records));
        Poller::new(
            Arc::new(Board {
                tickets: Arc::clone(tickets),
                repo: ForgeSlug::try_from("shep-pm/kelpie".to_owned()).unwrap(),
            }),
            choose,
            self.state.workers().records(&self.project).unwrap(),
            requests,
            retry,
            Arc::new(Fixed(1_700_000_000)),
        )
    }
}

/// A rig with a project on a repo of its own, a forge stood in, and `text` left in
/// ticket 12's hand-over file
fn rig_with(text: &str) -> (Rig, Arc<FakeTickets>, Arc<OnThreads>) {
    let rig = Rig::new();
    rig.with_origin();
    let tickets = Arc::new(tickets());
    let pulls = Arc::new(OnThreads::default());
    rig.stand_in(&tickets, &pulls);
    left(&rig, text);
    (rig, tickets, pulls)
}

/// Start ticket 12's worker on `agent` and wait for its turn to end
fn ran(rig: &Rig, agent: &FakeAgent, pulls: &Arc<OnThreads>) -> Heard {
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    rig.started(agent, pulls, &heard);
    until_turn_ends(&heard);
    heard
}

#[test]
fn a_turn_that_ends_done_runs_the_hand_over_off_the_sink_thread() {
    let (rig, _tickets, pulls) = rig_with(FILE);
    let agent = FakeAgent::new().turn(Turn::new().text("Pushed."));
    let heard = ran(&rig, &agent, &pulls);

    let (asked, on) = until_some(|| {
        pulls
            .threads()
            .into_iter()
            .find(|(asked, _)| *asked == Asked::Create)
    });
    assert_eq!(asked, Asked::Create);
    assert_ne!(
        on,
        sink_thread(&heard),
        "the draft was opened on a thread of its own, not on the session's sink",
    );

    assert_eq!(
        pulls.calls(),
        [
            PullCall::BranchHead(BRANCH.to_owned()),
            PullCall::Create {
                head: BRANCH.to_owned(),
                base: "main".to_owned(),
                title: "feat(board): open the draft".to_owned(),
                body: "It opens the draft as the maintainer.\n\nResolves #12".to_owned(),
            },
        ],
        "the draft is opened from the file the turn left, once it has read the branch",
    );
    let record = rig.record();
    assert_eq!(record.status, Status::Ended, "the worker's turn ended");
    assert_eq!(record.reason, None, "and nothing waits on the maintainer");
}

#[test]
fn a_turn_that_ends_held_runs_no_hand_over() {
    let (rig, _tickets, pulls) = rig_with(FILE);
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    rig.started(&agent, &pulls, &heard);

    // What the fence holds: a harness's own file, committed on the branch
    let worktree = paths(&rig).worktree(TICKET);
    fs::create_dir_all(worktree.join(".claude")).unwrap();
    fs::write(worktree.join(".claude/settings.json"), "{\"hooks\":{}}\n").unwrap();
    git(&worktree, &["add", ".claude/settings.json"]);
    git(&worktree, &["commit", "--quiet", "-m", "settings"]);

    let session = rig
        .state
        .workers()
        .session(&rig.project, TICKET)
        .expect("the worker holds a slot");
    session.cancel();
    until_turn_ends(&heard);

    let record = rig.record();
    assert_eq!(record.status, Status::WaitingOnMaintainer);
    assert!(
        record
            .reason
            .as_deref()
            .is_some_and(|why| why.contains("harness's own files")),
        "{:?}",
        record.reason,
    );
    assert!(
        pulls.calls().is_empty(),
        "a branch the fence holds asks nothing of the forge: {:?}",
        pulls.calls(),
    );
}

#[test]
fn a_told_outcome_is_sent_to_the_worker_as_a_note() {
    let (rig, _tickets, pulls) = rig_with("# Open the draft\n\nA heading is no title.\n");
    // The turn the Note starts is the second one, and it runs until the test ends
    let agent = FakeAgent::new()
        .turn(Turn::new().text("Pushed."))
        .turn(Turn::new().text("Rewriting the file.").until_cancelled());
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    rig.started(&agent, &pulls, &heard);

    let told = until_some(|| prompt(&agent, 1));
    assert!(
        told.contains("is not a conventional commit subject"),
        "the worker is told what is wrong with its file: {told}",
    );
    assert!(
        told.contains("pull-request.md"),
        "and which file to fix: {told}",
    );
    assert!(
        pulls.calls().is_empty(),
        "a file with no title opens nothing: {:?}",
        pulls.calls(),
    );
}

#[test]
fn a_failed_open_records_waiting_with_the_reason_and_holds_no_slot() {
    let (rig, _tickets, pulls) = rig_with(FILE);
    pulls.script_create(Err(ForgeError::Failed(
        "gh: HTTP 500: Bad Gateway".to_owned(),
    )));
    let agent = FakeAgent::new().turn(Turn::new().text("Pushed."));
    ran(&rig, &agent, &pulls);

    let record = until_some(|| {
        let record = rig.record();
        (record.status == Status::WaitingOnMaintainer).then_some(record)
    });
    let why = record.reason.clone().unwrap_or_default();
    assert!(
        why.starts_with("Could not open the draft pull request"),
        "{why}",
    );
    assert!(
        why.contains("HTTP 500"),
        "the reason names the refusal: {why}"
    );
    assert!(
        why.contains("kelpie tries again each minute"),
        "and what kelpie then does: {why}",
    );
    assert_eq!(record.seat, Seat::Without, "a parked worker holds no slot");
}

#[test]
fn a_later_success_sets_the_worker_ended_and_clears_the_reason() {
    let (rig, _tickets, pulls) = rig_with(FILE);
    pulls.script_create(Err(ForgeError::Failed(
        "gh: HTTP 500: Bad Gateway".to_owned(),
    )));
    let agent = FakeAgent::new().turn(Turn::new().text("Pushed."));
    ran(&rig, &agent, &pulls);
    until_some(|| (rig.record().status == Status::WaitingOnMaintainer).then_some(()));

    pulls.script_create(Ok(NonZeroU64::MIN));
    rig.hands().retry(&rig.listed());

    let record = rig.record();
    assert_eq!(record.status, Status::Ended, "the draft went through");
    assert_eq!(record.reason, None, "so its reason is gone");
    assert_eq!(pulls.creates(), 2, "the retry opened the draft");
}

#[test]
fn the_retry_skips_a_worker_whose_file_the_forge_already_has() {
    let (rig, _tickets, pulls) = rig_with(FILE);
    let agent = FakeAgent::new().turn(Turn::new().text("Pushed."));
    ran(&rig, &agent, &pulls);
    // The hand-over is over once the forge has the file, which is when the worker
    // stops owing one: waiting on the call instead would run on before the copy of it
    // is kept, and the retry would be answering a hand-over still in flight
    until_some(|| (!owes_a_draft(&paths(&rig), TICKET)).then_some(()));

    let hands = rig.hands();
    hands.retry(&rig.listed());
    hands.retry(&rig.listed());

    assert_eq!(
        pulls.creates(),
        1,
        "the draft the forge already holds is not opened again",
    );
}

#[test]
fn the_retry_runs_on_each_github_read_for_a_worker_that_owes_a_pull_request() {
    let (rig, tickets, pulls) = rig_with(FILE);
    pulls.script_create(Err(ForgeError::Failed(
        "gh: HTTP 500: Bad Gateway".to_owned(),
    )));
    let agent = FakeAgent::new().turn(Turn::new().text("Pushed."));
    ran(&rig, &agent, &pulls);
    until_some(|| (pulls.creates() >= 1).then_some(()));

    let mut poller = rig.poller(&tickets, &rig.hands());
    poller.read();
    assert_eq!(pulls.creates(), 2, "the read handed the draft over again");
    assert_eq!(
        rig.record().status,
        Status::WaitingOnMaintainer,
        "and it is still owed",
    );

    poller.read();
    assert_eq!(pulls.creates(), 3, "so the next read tries once more");

    poller.place();
    assert_eq!(
        pulls.creates(),
        3,
        "and a placing, which asks nothing of GitHub, hands over nothing",
    );
}

#[test]
fn a_file_the_guard_keeps_refusing_parks_on_a_ruling_naming_the_refusal() {
    let (rig, _tickets, pulls) = rig_with(FILE);
    // What the forge's leak guard answers for a text naming this machine
    pulls.script_create(Err(ForgeError::LocalPath));
    let agent = FakeAgent::new()
        .turn(Turn::new().text("Pushed."))
        .turn(Turn::new().text("Rewrote it, and the same."));
    ran(&rig, &agent, &pulls);

    let record = ruled(&rig);
    let why = record.reason.clone().unwrap_or_default();
    assert!(
        why.starts_with(
            "Kelpie told the worker to fix its hand-over file, and the same refusal stands"
        ),
        "{why}",
    );
    assert!(
        why.contains("path or address on this machine"),
        "the ruling names the refusal: {why}",
    );
    assert!(
        why.contains("pull-request.md"),
        "and the file it is about: {why}"
    );
    assert_eq!(record.seat, Seat::Without, "a ruled worker holds no slot");
    assert_eq!(
        agent.params("session/prompt").len(),
        2,
        "the worker was told once, and its next turn answered the telling",
    );
    assert_eq!(
        pulls.creates(),
        2,
        "the file was handed over at each turn's end, and at no third",
    );
}

#[test]
fn a_title_that_stays_unconventional_parks_on_a_ruling_naming_the_reason() {
    let (rig, _tickets, pulls) = rig_with("# Open the draft\n\nA heading is no title.\n");
    let agent = FakeAgent::new()
        .turn(Turn::new().text("Pushed."))
        .turn(Turn::new().text("Rewrote the title, still a heading."));
    ran(&rig, &agent, &pulls);

    let record = ruled(&rig);
    let why = record.reason.clone().unwrap_or_default();
    assert!(
        why.contains("is not a conventional commit subject"),
        "the ruling names why the file was refused: {why}",
    );
    assert!(
        why.contains("Open the draft"),
        "and the line that is no title: {why}",
    );
    assert!(
        why.contains("kelpie tells it no more until the file changes"),
        "and what kelpie does next: {why}",
    );
    assert_eq!(record.seat, Seat::Without, "a ruled worker holds no slot");
    assert_eq!(
        agent.params("session/prompt").len(),
        2,
        "the worker was told once, and its next turn answered the telling",
    );
    assert!(
        pulls.calls().is_empty(),
        "a file with no title never reaches the forge: {:?}",
        pulls.calls(),
    );
}

#[test]
fn a_file_fixed_after_a_ruling_opens_the_draft_and_clears_the_ruling() {
    let (rig, _tickets, pulls) = rig_with("# Open the draft\n\nA heading is no title.\n");
    let agent = FakeAgent::new()
        .turn(Turn::new().text("Pushed."))
        .turn(Turn::new().text("Rewrote the title, still a heading."));
    ran(&rig, &agent, &pulls);
    let ruled = ruled(&rig);
    assert!(
        ruled.refused.is_some(),
        "the refusal is kept for the next hand-over to answer",
    );

    // The maintainer's fix, and the worker started again to carry it
    left(&rig, FILE);
    pulls.script_create(Ok(NonZeroU64::MIN));
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    let fixed = FakeAgent::new().turn(Turn::new().text("Fixed the title."));
    rig.started(&fixed, &pulls, &heard);
    until_turn_ends(&heard);

    let record = until_some(|| {
        let record = rig.record();
        (record.refused.is_none() && record.status == Status::Ended).then_some(record)
    });
    assert_eq!(record.reason, None, "and the ruling goes with the refusal");
    assert_eq!(pulls.creates(), 1, "the fixed file opened the draft");
}
