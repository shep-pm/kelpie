//! Telling a worker something from its tab: a Note, Stop and tell, Stop, and an answer
//!
//! Each test starts a real worker through [`Workers::start`] on a [`FakeAgent`] whose turn
//! waits to be cancelled, so the test decides when it ends, and then calls the app's own
//! steering functions. No test needs `gh`, a model or the network.
//!
//! The last one dials the worker's kelpie tools door as a harness's relay does, so the
//! sandbox request it asks about is the worker's own, and the answer reaches the worker's
//! session.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kelpie_engine::sandbox::Refusal;
use kelpie_engine::session::fake::{FakeAgent, FakeNetwork, Turn};
use kelpie_engine::session::{SandboxAnswer, Session};
use kelpie_engine::tools::SessionTools;
use kelpie_engine::tools::fake::FakeTools;
use kelpie_engine::worker::{Opening, Record, Seat, Status, Ticket};
use kelpie_protocol::SessionEvent;

use super::Rig;
use crate::workers::{NoteOutcome, OpenOn, answer_on, cancel_on, lock, note_on, stop_and_tell_on};

/// How long a test waits for something it forced to happen
const WAIT: Duration = Duration::from_secs(20);

/// The ticket every test here starts
pub(super) const TICKET: &str = "12";

/// The feed watch id these tests attach under
const WATCH: u32 = 1;

/// Every event a worker's feed carried
pub(super) type Heard = Arc<Mutex<Vec<SessionEvent>>>;

/// The kelpie tools the worker's session was handed, which a test dials as a relay does
pub(super) type Kept = Arc<Mutex<Option<SessionTools>>>;

/// The events and the tools of one worker's session
pub(super) fn session() -> (Heard, Kept) {
    (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(None)))
}

/// The opener of ticket 12's worker on `agent`, keeping what its session was handed
///
/// `network` stands for the sandbox the session runs in, which an answer widens. `heard`
/// keeps every event the worker's feed carried, which is why the feed is watched from
/// inside the opener: the turn is in flight as soon as the session exists. `kept` holds the
/// session's own kelpie tools.
pub(super) fn opener<'a>(
    rig: &'a Rig,
    agent: &FakeAgent,
    network: &FakeNetwork,
    heard: &Heard,
    kept: &Kept,
) -> impl Fn(&OpenOn<'_>, &Opening<'_>) -> Result<Session, String> + 'a {
    let (state, project) = (&rig.state, &rig.project);
    let (agent, network) = (agent.clone(), network.clone());
    let (heard, kept) = (Arc::clone(heard), Arc::clone(kept));
    move |_on: &OpenOn<'_>, opening: &Opening<'_>| {
        *lock(&kept) = opening.tools.clone();
        let send = {
            let heard = Arc::clone(&heard);
            move |event: SessionEvent| {
                lock(&heard).push(event);
                true
            }
        };
        state
            .workers()
            .watch(project, TICKET, WATCH, send)
            .expect("the worker has a record to watch");
        Ok(agent.clone().sandbox_network(&network).session(
            opening.worktree.to_path_buf(),
            opening.resume.clone(),
            opening.choice.clone(),
        ))
    }
}

/// Start ticket 12's worker on `agent`, as [`opener`] builds its session
pub(super) fn started(
    rig: &Rig,
    agent: &FakeAgent,
    network: &FakeNetwork,
    heard: &Heard,
    kept: &Kept,
) {
    let state = &rig.state;
    let opener = opener(rig, agent, network, heard, kept);
    let folder = state.project_path_of(&rig.project).unwrap();
    let ticket = Ticket {
        id: TICKET.to_owned(),
        text: "Build the thing.\n\nOne turn.".to_owned(),
    };
    state
        .workers()
        .start(&rig.project, &folder, state.tools(), &ticket, &opener)
        .expect("the worker starts");
}

/// Wait for `ask` to answer true
pub(super) fn until(mut ask: impl FnMut() -> bool) {
    let since = Instant::now();
    loop {
        if ask() {
            return;
        }
        assert!(since.elapsed() < WAIT, "it never happened");
        std::thread::sleep(Duration::from_millis(20));
    }
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

/// Wait for the worker's turn to have said something, which it does once its harness has
/// the prompt
pub(super) fn until_working(heard: &Heard) {
    until(|| {
        lock(heard)
            .iter()
            .any(|event| matches!(event, SessionEvent::Text { .. }))
    });
}

/// Wait for `n` of the worker's turns to have ended
pub(super) fn until_turns(heard: &Heard, n: usize) {
    until(|| {
        lock(heard)
            .iter()
            .filter(|event| matches!(event, SessionEvent::TurnEnded { .. }))
            .count()
            >= n
    });
}

/// Wait for the sandbox request the worker asked the maintainer about, and return its id
fn until_request(heard: &Heard) -> String {
    until_some(|| {
        lock(heard).iter().find_map(|event| match event {
            SessionEvent::SandboxRequest { id, .. } => Some(id.clone()),
            _ => None,
        })
    })
}

/// The text of each prompt the harness was sent, oldest first
pub(super) fn prompts(agent: &FakeAgent) -> Vec<String> {
    agent
        .params("session/prompt")
        .iter()
        .map(|call| call["prompt"][0]["text"].as_str().unwrap().to_owned())
        .collect()
}

/// Ticket 12's record, as the app keeps it
pub(super) fn record(rig: &Rig) -> Record {
    let records = rig.state.workers().records(&rig.project).unwrap();
    let records = records.lock().unwrap();
    records.get(TICKET).expect("ticket 12 has a record").clone()
}

/// What ticket 12's record says it is doing, and what it holds
pub(super) fn standing(rig: &Rig) -> (Status, Seat) {
    let record = record(rig);
    (record.status, record.seat)
}

#[test]
fn a_note_on_a_running_worker_calls_the_engines_note_and_answers_queued() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new().turn(Turn::new().text("working").until_cancelled());
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_working(&heard);

    let noted = note_on(&rig.state, "12", "use the new name").unwrap();

    assert_eq!(noted, NoteOutcome::Queued);
    assert_eq!(record(&rig).notes, ["use the new name"]);
    assert_eq!(
        prompts(&agent).len(),
        1,
        "the note waits for the turn's end, as the engine keeps it"
    );
    assert_eq!(
        standing(&rig),
        (Status::Running, Seat::Held),
        "the worker runs on, holding its slot"
    );
}

#[test]
fn a_note_between_turns_is_the_workers_next_turn_and_answers_sent() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_turns(&heard, 1);
    assert_eq!(
        standing(&rig),
        (Status::Ended, Seat::Without),
        "the turn ended with nothing left on the branch",
    );

    let noted = note_on(&rig.state, "12", "one more thing").unwrap();

    assert_eq!(
        noted,
        NoteOutcome::Sent,
        "a worker whose turn ended keeps its session, so the note is its next turn",
    );
    until_turns(&heard, 2);
    assert_eq!(
        prompts(&agent)[1..],
        ["one more thing"],
        "the note went out as a prompt, and not into the record"
    );
    assert!(record(&rig).notes.is_empty(), "a sent note is not kept");
    assert_eq!(
        standing(&rig),
        (Status::Ended, Seat::Without),
        "the turn the note started settles the record as any other"
    );
}

#[test]
fn stop_and_tell_and_stop_call_their_engine_functions() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("working").until_cancelled())
        .turn(Turn::new().text("redirected").until_cancelled());
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_working(&heard);

    let told = stop_and_tell_on(&rig.state, "12", "do the other thing").unwrap();

    assert_eq!(told, NoteOutcome::Sent);
    until_turns(&heard, 1);
    assert_eq!(
        standing(&rig),
        (Status::Running, Seat::Held),
        "the cancelled turn's end settles nothing"
    );
    until(|| prompts(&agent).len() == 2);
    assert_eq!(
        prompts(&agent)[1..],
        ["do the other thing"],
        "what the maintainer said is the next prompt"
    );

    cancel_on(&rig.state, "12").unwrap();
    until_turns(&heard, 2);
    assert_eq!(
        standing(&rig),
        (Status::Paused, Seat::Without),
        "Stop leaves the worker Paused and gives its slot back"
    );
}

#[test]
fn a_ticket_with_no_live_session_is_refused_by_name_by_each_command() {
    let rig = Rig::new();
    rig.with_origin();

    let told = [
        note_on(&rig.state, "12", "use the new name").map(drop),
        stop_and_tell_on(&rig.state, "12", "do the other thing").map(drop),
        cancel_on(&rig.state, "12"),
        answer_on(&rig.state, "12", "net-1", SandboxAnswer::Once),
    ];

    for (name, told) in ["Note", "Stop and tell", "Stop", "answer"]
        .into_iter()
        .zip(told)
    {
        let said = told.expect_err("{name} reached a ticket with no live session");
        assert!(
            said.contains("12"),
            "{name} did not name the ticket: {said}"
        );
    }
    assert!(rig.recorded().is_empty(), "nothing was started to be told");
}

#[test]
fn an_answer_reaches_the_workers_session_and_not_the_pms() {
    let rig = Rig::new();
    rig.with_origin();
    let network = FakeNetwork::default();
    let agent = FakeAgent::new().turn(Turn::new().until_cancelled());
    let (heard, kept) = session();
    started(&rig, &agent, &network, &heard, &kept);
    until(|| {
        lock(&heard)
            .iter()
            .any(|event| matches!(event, SessionEvent::Started { .. }))
    });
    let tools = lock(&kept)
        .take()
        .expect("the worker was given kelpie's tools");
    let mut asked = FakeTools::connect(&tools).unwrap();

    asked.ask("docs.rs", "to read the crate's docs").unwrap();
    let id = until_request(&heard);
    assert_eq!(
        network.allowed(),
        Vec::<String>::new(),
        "refused until the answer"
    );

    answer_on(&rig.state, "12", &id, SandboxAnswer::Once).unwrap();

    assert_eq!(
        asked.answered().unwrap(),
        ("docs.rs is allowed for this session".to_owned(), false),
        "the worker's session took the answer"
    );
    assert_eq!(network.allowed(), ["docs.rs"]);
    assert!(
        rig.state.held_session().is_none(),
        "the PM's session was never asked, and none was started to answer"
    );
    assert!(
        rig.state
            .workers()
            .feed(&rig.project, TICKET)
            .waiting()
            .is_empty(),
        "the answered request is no longer a needs-you line on the ticket"
    );
}

/// What the worker on ticket 12 is told as a Note, with `agent` to open its session if it needs one
///
/// No source is given: ticket 12's worker has had a turn, so its text is never read.
fn noted(rig: &Rig, agent: &FakeAgent, text: &str) -> Result<NoteOutcome, String> {
    let (heard, kept) = session();
    let open = opener(rig, agent, &FakeNetwork::default(), &heard, &kept);
    let folder = rig.state.project_path_of(&rig.project).unwrap();
    rig.state.workers().note(
        &rig.project,
        &folder,
        rig.state.tools(),
        TICKET,
        text,
        Err("no ticket is read for a worker that has had a turn".to_owned()),
        &open,
    )
}

/// Park ticket 12's worker the way a ruling does: waiting on the maintainer, with `reason`
///
/// Its session is given up, as the next ask of one does.
fn ruled(rig: &Rig, reason: &str) {
    {
        let records = rig.state.workers().records(&rig.project).unwrap();
        let mut records = lock(&records);
        records
            .set_status(TICKET, Status::WaitingOnMaintainer)
            .unwrap();
        records.set_reason(TICKET, Some(reason.to_owned())).unwrap();
    }
    assert!(
        rig.state.workers().session(&rig.project, TICKET).is_none(),
        "a worker waiting on the maintainer keeps no session"
    );
}

#[test]
fn a_note_to_a_worker_waiting_on_a_ruling_sends_it_back_with_the_text() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_turns(&heard, 1);
    ruled(&rig, "the pull request is still red");

    let told = noted(&rig, &agent, "the failure is in the fixture, not the code").unwrap();

    assert_eq!(told, NoteOutcome::Sent, "the text went out as a turn");
    until_turns_in(&agent, 2);
    let sent = &prompts(&agent)[1];
    assert!(
        sent.ends_with("the failure is in the fixture, not the code"),
        "the worker's next prompt carries the maintainer's text: {sent:?}",
    );
    until(|| record(&rig).status == Status::Ended);
    let record = record(&rig);
    assert_eq!(
        record.reason, None,
        "the ruling's reason is gone with the ruling"
    );
    assert!(
        record.notes.is_empty(),
        "the text went out, not into the record"
    );
}

#[test]
fn a_note_to_a_paused_worker_is_kept_for_its_next_start() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new()
        .turn(Turn::new().text("working").until_cancelled())
        .turn(Turn::new().text("on it"));
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_working(&heard);
    cancel_on(&rig.state, TICKET).unwrap();
    until_turns(&heard, 1);
    assert_eq!(standing(&rig), (Status::Paused, Seat::Without));

    let told = noted(&rig, &agent, "use the new name").unwrap();

    assert_eq!(
        told,
        NoteOutcome::Queued,
        "a Paused worker is not sent back"
    );
    assert_eq!(record(&rig).notes, ["use the new name"]);
    assert_eq!(standing(&rig), (Status::Paused, Seat::Without));
    assert_eq!(prompts(&agent).len(), 1, "no turn started");

    let folder = rig.state.project_path_of(&rig.project).unwrap();
    let (heard, kept) = session();
    let open = opener(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    rig.state
        .workers()
        .resume(
            &rig.project,
            &folder,
            rig.state.tools(),
            TICKET,
            Err("no ticket is read for a worker that has had a turn".to_owned()),
            &open,
        )
        .unwrap();
    until(|| prompts(&agent).len() == 2);
    assert!(
        prompts(&agent)[1].ends_with("Carry on from where you stopped.\n\nuse the new name"),
        "its next start's prompt does not end with the note"
    );
    assert!(record(&rig).notes.is_empty());
}

/// What the worker on ticket 12 is told as a Note when its session cannot be opened, as `open` fails it
fn noted_through(
    rig: &Rig,
    text: &str,
    open: &crate::workers::Opener<'_>,
) -> Result<NoteOutcome, String> {
    let folder = rig.state.project_path_of(&rig.project).unwrap();
    rig.state.workers().note(
        &rig.project,
        &folder,
        rig.state.tools(),
        TICKET,
        text,
        Err("no ticket is read for a worker that has had a turn".to_owned()),
        open,
    )
}

#[test]
fn a_note_whose_fix_turn_hits_a_refused_sandbox_leaves_the_worker_paused_with_the_refusal() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_turns(&heard, 1);
    ruled(&rig, "the pull request is still red");

    let refused = noted_through(&rig, "try again", &|_, opening: &Opening<'_>| {
        Err(opening.refuse(Refusal::NoCapabilities))
    });

    assert!(refused.is_err(), "the start was refused");
    let record = record(&rig);
    assert_eq!(
        record.status,
        Status::Paused,
        "a refused sandbox waits Paused, so Resume takes it"
    );
    assert_eq!(
        record.reason,
        Some(format!(
            "its sandbox cannot start because {}",
            Refusal::NoCapabilities
        )),
        "the ticket says why, not what the ruling was"
    );
}

#[test]
fn a_note_sent_again_after_its_fix_turn_failed_to_start_goes_out_once() {
    let rig = Rig::new();
    rig.with_origin();
    let agent = FakeAgent::new()
        .can_resume()
        .knows("fake-1")
        .turn(Turn::new().text("done"))
        .turn(Turn::new().text("on it"));
    let (heard, kept) = session();
    started(&rig, &agent, &FakeNetwork::default(), &heard, &kept);
    until_turns(&heard, 1);
    ruled(&rig, "the pull request is still red");
    let broken = |_: &OpenOn<'_>, _: &Opening<'_>| Err::<Session, _>("no harness".to_owned());

    noted_through(&rig, "use the new name", &broken).unwrap_err();
    assert_eq!(
        standing(&rig).0,
        Status::WaitingOnMaintainer,
        "the ruling is put back"
    );
    noted_through(&rig, "use the new name", &broken).unwrap_err();
    let told = noted(&rig, &agent, "use the new name").unwrap();

    assert_eq!(told, NoteOutcome::Sent);
    until_turns_in(&agent, 2);
    assert_eq!(
        prompts(&agent)[1].matches("use the new name").count(),
        1,
        "the worker's first prompt carries the text once, however many tries failed"
    );
}

/// Wait for `agent` to have been sent `n` prompts
fn until_turns_in(agent: &FakeAgent, n: usize) {
    until(|| prompts(agent).len() >= n);
}
