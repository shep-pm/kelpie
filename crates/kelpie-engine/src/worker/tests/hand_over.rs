//! The hand-over: the file a turn leaves, and the draft its branch gets

use std::num::NonZeroU64;
use std::path::Path;

use super::*;
use crate::ci::HeadStatus;
use crate::ticket::fake::{FakePulls, FakeTickets, PullCall};
use crate::ticket::{ForgeError, OpenPull};
use crate::worker::hand_over::{APPLIED, HandOver, Outcome, Text, Untitled, after_turn, parse};

/// The ticket every test here starts
const TICKET: &str = "74";

/// The branch ticket 74's worker works on
const BRANCH: &str = "kelpie/74";

/// Ticket 74's worker in `rig`, with its turn ended and its worktree on disk
fn worker(rig: &Rig) -> PathBuf {
    let binding = binding();
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket(TICKET, "Build the thing"),
        sink,
    )
    .unwrap();
    let Started::Running { worktree, .. } = started else {
        panic!("the ticket was not started: {started:?}");
    };
    until_turn_ends(&heard);
    worktree
}

/// The hand-over of ticket 74's turn, over these sources
fn hand_over<'a>(rig: &'a Rig, pulls: &'a FakePulls, tickets: &'a FakeTickets) -> HandOver<'a> {
    HandOver {
        paths: &rig.paths,
        ticket: TICKET,
        tree: &rig.tree,
        folder: &rig.repo,
        base: "main",
        pulls,
        tickets,
    }
}

/// What the worker left in its hand-over file
fn left(rig: &Rig, text: &str) {
    let file = rig.paths.pull_request_file(TICKET);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, text).unwrap();
}

/// The hand-over file, as kelpie left it after sending it
fn applied(rig: &Rig) -> Option<String> {
    std::fs::read_to_string(rig.paths.state_dir(TICKET).join(APPLIED)).ok()
}

/// Commit on the branch, push it to the rig's own origin, and script the forge's
/// answer for the branch as the commit that push put there
fn pushed(rig: &Rig, worktree: &Path, pulls: &FakePulls) {
    git(worktree, &["push", "--quiet", "origin", BRANCH]);
    let head = git(&rig.root.join("origin.git"), &["rev-parse", BRANCH]);
    pulls.script_branch_head(BRANCH, Ok(Some(head)));
}

/// An open draft pull request of ticket 74's branch, as the ticket source lists it
fn open_draft(pulls: NonZeroU64) -> Vec<OpenPull> {
    vec![OpenPull {
        number: pulls,
        branch: BRANCH.to_owned(),
        draft: true,
        closes: Vec::new(),
        head: "0".repeat(40),
        base: "main".to_owned(),
        labels: Vec::new(),
        checks: HeadStatus::None,
    }]
}

/// The first line of a `Told` refusal, with the file it names cut off
fn told(outcome: Outcome) -> String {
    let Outcome::Told(text) = outcome else {
        panic!("the turn was not told: {outcome:?}");
    };
    text
}

// The worker is told how to hand its pull request over: push, write the title and
// body in the hand-over file, and what makes a title one kelpie opens from.
#[test]
fn the_instructions_name_the_file_the_push_the_title_rule_and_the_resolves_line() {
    let mut rig = Rig::new("hand over instructions");
    rig.on_github("shep-pm/koji");
    let home = tempfile::tempdir().unwrap();
    let github = FakeGithub::new(FakeClock::at(1_000_000), "shep-pm");
    github.registered(home.path(), "shep-pm/koji");
    rig.tokens = Some(github.tokens(home.path()));

    let binding = binding_on("Do the ticket.\n\n{pull_request}\n");
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket(TICKET, "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(started, Started::Running { .. }), "{started:?}");
    until_turn_ends(&heard);

    let prompt = agent.params("session/prompt")[0]["prompt"][0]["text"]
        .as_str()
        .expect("the prompt holds no text")
        .to_owned();
    assert!(
        prompt.contains(&rig.paths.pull_request_file(TICKET).display().to_string()),
        "{prompt}"
    );
    assert!(prompt.contains("git push origin HEAD"), "{prompt}");
    assert!(
        prompt.contains(
            "opens nothing from a file whose first line is not a conventional commit subject"
        ),
        "{prompt}"
    );
    assert!(
        prompt.contains("kelpie opens the draft when a turn ends with the branch pushed"),
        "{prompt}"
    );
    assert!(
        prompt.contains("Resolves #<ticket>"),
        "the body's last line is named: {prompt}"
    );
    assert!(
        !prompt.contains("{pull_request}")
            && !prompt.contains("{file}")
            && !prompt.contains("{ends}"),
        "no placeholder reaches the worker: {prompt}"
    );
    assert!(
        !prompt.contains("git push is refused"),
        "a worker with a credential is not told it cannot push: {prompt}"
    );
}

// A project's own agent file that names no placeholder is the worker's own text,
// with no hand-over bolted onto it.
#[test]
fn a_project_agent_file_with_no_placeholder_is_sent_as_it_is() {
    let rig = Rig::new("own agent file");
    let binding = binding_on("Do the ticket on your own branch.");
    let agent = FakeAgent::new().turn(Turn::new().text("done"));
    let open = |opening: &Opening<'_>| -> Result<Session, String> { Ok(on(&agent, opening)) };
    let (sink, heard) = events();
    let started = start(
        &rig.project(&binding, &open, 2),
        &ticket(TICKET, "Build the thing"),
        sink,
    )
    .unwrap();
    assert!(matches!(started, Started::Running { .. }), "{started:?}");
    until_turn_ends(&heard);

    let prompt = agent.params("session/prompt")[0]["prompt"][0]["text"]
        .as_str()
        .expect("the prompt holds no text")
        .to_owned();
    assert!(
        prompt.starts_with("Do the ticket on your own branch."),
        "{prompt}"
    );
    assert!(
        !prompt.contains("Do not open or edit the pull request")
            && !prompt.contains("Resolves #")
            && !prompt.contains(&rig.paths.pull_request_file(TICKET).display().to_string()),
        "no hand-over is added to a file that asks for none: {prompt}"
    );
}

// A turn that pushed and left a file has its draft opened from it, as the file
// words it.
#[test]
fn a_pushed_branch_with_a_file_has_a_draft_opened_with_the_files_title_and_body() {
    let rig = Rig::new("hand over opened");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    left(
        &rig,
        "feat(worker): open the draft from the file\n\nWhy.\n\nResolves #74\n",
    );
    let tickets = FakeTickets::new();

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    assert_eq!(outcome, Outcome::Opened(NonZeroU64::MIN));
    let calls = pulls.calls();
    assert_eq!(
        calls,
        [
            PullCall::BranchHead(BRANCH.to_owned()),
            PullCall::Create {
                head: BRANCH.to_owned(),
                base: "main".to_owned(),
                title: "feat(worker): open the draft from the file".to_owned(),
                body: "Why.\n\nResolves #74".to_owned(),
            },
        ]
    );
    assert_eq!(
        applied(&rig).as_deref(),
        Some("feat(worker): open the draft from the file\n\nWhy.\n\nResolves #74\n"),
        "the file as sent is kept beside it"
    );
}

// A worker that forgot the line still has its pull request linked to its ticket.
#[test]
fn a_body_without_a_resolves_line_gets_one_for_the_ticket() {
    let rig = Rig::new("hand over resolves");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    left(&rig, "fix(worker): keep the ticket linked\n\nWhy.\n");
    let tickets = FakeTickets::new();

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    assert_eq!(outcome, Outcome::Opened(NonZeroU64::MIN));
    let calls = pulls.calls();
    let [_, PullCall::Create { body, .. }] = calls.as_slice() else {
        panic!("no draft was opened: {calls:?}");
    };
    assert_eq!(body, "Why.\n\nResolves #74", "{body}");
}

// A worker that wrapped its body at eighty columns would have it shown as a
// narrow ragged column, since GitHub breaks a body at every newline.
#[test]
fn a_hard_wrapped_body_opens_with_its_paragraphs_joined() {
    let rig = Rig::new("hand over wrapped");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    left(&rig, WRAPPED_FILE);
    let tickets = FakeTickets::new();

    // A fixture whose paragraph was already one line would test nothing
    assert!(
        WRAPPED_FILE.contains("wrapped at\neighty columns"),
        "the fixture's paragraph must span lines for kelpie to join one"
    );

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    assert_eq!(outcome, Outcome::Opened(NonZeroU64::MIN));
    let calls = pulls.calls();
    let [_, PullCall::Create { title, body, .. }] = calls.as_slice() else {
        panic!("no draft was opened: {calls:?}");
    };
    assert_eq!(title, WRAPPED_TITLE);
    assert_eq!(body, WRAPPED_BODY);
}

// The file a worker leaves when it wrapped its body at eighty columns. Each
// piece here is one line of the file, so its paragraph really is soft-wrapped
const WRAPPED_FILE: &str = concat!(
    "fix(worker): join a body's lines\n",
    "\n",
    "GitHub shows every newline in a body as a line break, so a body wrapped at\n",
    "eighty columns reads as a narrow ragged column.\n",
    "\n",
    "- Kelpie joins each soft-wrapped line\n",
    "  onto the one before it\n",
    "\n",
    "Resolves #74\n",
);

const WRAPPED_TITLE: &str = "fix(worker): join a body's lines";

// The same body with each paragraph and each list item on one line
const WRAPPED_BODY: &str = concat!(
    "GitHub shows every newline in a body as a line break, so a body ",
    "wrapped at eighty columns reads as a narrow ragged column.\n",
    "\n",
    "- Kelpie joins each soft-wrapped line onto the one before it\n",
    "\n",
    "Resolves #74",
);

// A worker that shows a command as an indented code block must keep it one: GitHub
// renders four spaces as code, and a body whose first line lost its indent would
// have its command folded into a paragraph on one line.
#[test]
fn a_body_that_opens_with_an_indented_code_block_opens_as_a_code_block() {
    let rig = Rig::new("hand over indented code");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    left(
        &rig,
        concat!(
            "feat(worker): keep the command readable\n",
            "\n",
            "    cargo test --workspace\n",
            "    cargo clippy --all-targets\n",
            "\n",
            "Run both from the repo root.\n",
            "\n",
            "Resolves #74\n",
        ),
    );
    let tickets = FakeTickets::new();

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    assert_eq!(outcome, Outcome::Opened(NonZeroU64::MIN));
    let calls = pulls.calls();
    let [_, PullCall::Create { body, .. }] = calls.as_slice() else {
        panic!("no draft was opened: {calls:?}");
    };
    assert_eq!(
        body,
        concat!(
            "    cargo test --workspace\n",
            "    cargo clippy --all-targets\n",
            "\n",
            "Run both from the repo root.\n",
            "\n",
            "Resolves #74",
        ),
        "{body}"
    );
}

// A branch whose commit the forge does not hold is a turn that never got its work
// out: kelpie opens nothing for it.
#[test]
fn a_branch_not_yet_pushed_opens_nothing() {
    let rig = Rig::new("hand over unpushed");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    // The turn commits again and never pushes it, so the forge holds an older commit
    commit(&worktree, "src/more.rs", "more work\n");
    left(
        &rig,
        "feat(worker): work not pushed\n\nWhy.\n\nResolves #74\n",
    );
    let tickets = FakeTickets::new();

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    assert_eq!(outcome, Outcome::NotPushed);
    assert_eq!(pulls.calls(), [PullCall::BranchHead(BRANCH.to_owned())]);
    assert_eq!(tickets.asked(), []);
    assert_eq!(applied(&rig), None);
}

// A turn that left no file hands over nothing, and kelpie asks the forge nothing
// for it.
#[test]
fn a_turn_with_no_file_opens_nothing() {
    let rig = Rig::new("hand over none");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    let tickets = FakeTickets::new();

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    assert_eq!(outcome, Outcome::NoFile);
    assert_eq!(pulls.calls(), []);
    assert_eq!(tickets.asked(), []);
}

// A template's first heading is not a title kelpie can send, and the worker is the
// one who can fix its file.
#[test]
fn a_first_line_that_is_not_a_conventional_subject_is_told_and_opens_nothing() {
    let rig = Rig::new("hand over title");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    left(&rig, "## Short summary\n\nWhy.\n\nResolves #74\n");
    let tickets = FakeTickets::new();

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    let text = told(outcome);
    assert!(text.contains("pull-request.md"), "{text}");
    assert!(text.contains("`Short summary`"), "{text}");
    assert!(
        text.contains("conventional commit subject"),
        "the worker is told what a title is: {text}"
    );
    assert!(text.contains("template's heading"), "{text}");
    assert_eq!(pulls.calls(), []);
    assert_eq!(tickets.asked(), []);
    assert_eq!(applied(&rig), None, "the file is left for the next turn");
}

// The forge refuses a body that names this machine. The worker is told why, and
// never told what was found.
#[test]
fn a_text_naming_this_machine_is_told_without_repeating_it() {
    let rig = Rig::new("hand over leak");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let leak = rig.root.join("notes/plan.md").display().to_string();
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    pulls.script_create(Err(ForgeError::LocalPath));
    left(
        &rig,
        &format!("feat(worker): a body with a path\n\nSee {leak}.\n"),
    );
    let tickets = FakeTickets::new();

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    let text = told(outcome);
    assert!(
        text.contains("names a path or address on this machine"),
        "{text}"
    );
    assert!(
        text.contains("Rewrite the file without the path or address it names"),
        "{text}"
    );
    assert!(
        !text.contains("notes/plan.md"),
        "the leak is not repeated: {text}"
    );
    assert!(!text.contains(&leak), "the leak is not repeated: {text}");
    assert_eq!(applied(&rig), None, "the file is left for the next turn");
}

// The worker can plant a link in its own folder. Kelpie reads it as no file, and
// never as what the link names.
#[cfg(unix)]
#[test]
fn a_file_that_is_a_link_is_refused_and_never_followed() {
    let rig = Rig::new("hand over link");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);

    let secret = rig.root.join("app-key.pem");
    std::fs::write(&secret, "-----BEGIN PRIVATE KEY-----\nthe-apps-key\n").unwrap();
    let file = rig.paths.pull_request_file(TICKET);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&secret, &file).unwrap();
    let tickets = FakeTickets::new();

    let outcome = after_turn(&hand_over(&rig, &pulls, &tickets));

    let text = told(outcome);
    assert!(text.contains("pull-request.md"), "{text}");
    assert!(text.contains("not a regular file"), "{text}");
    assert!(
        !text.contains("the-apps-key"),
        "the file is not read: {text}"
    );
    assert_eq!(pulls.calls(), []);
    assert_eq!(tickets.asked(), []);
}

// A later turn's file moves the open pull request, and one that left the file
// alone leaves an edit made on GitHub as it is.
#[test]
fn an_open_pull_request_is_edited_only_when_the_file_changed() {
    let rig = Rig::new("hand over edited");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    let tickets = FakeTickets::new();
    tickets.script_pulls(Ok(open_draft(NonZeroU64::new(5).unwrap())));
    let ctx = hand_over(&rig, &pulls, &tickets);

    left(&rig, "feat(worker): the first text\n\nbody\n");
    assert_eq!(
        after_turn(&ctx),
        Outcome::Edited(NonZeroU64::new(5).unwrap())
    );
    assert_eq!(
        pulls.calls().last(),
        Some(&PullCall::Edit {
            number: NonZeroU64::new(5).unwrap(),
            title: "feat(worker): the first text".to_owned(),
            body: "body\n\nResolves #74".to_owned(),
        })
    );

    assert_eq!(after_turn(&ctx), Outcome::Unchanged);
    assert_eq!(
        pulls
            .calls()
            .iter()
            .filter(|call| matches!(call, PullCall::Edit { .. }))
            .count(),
        1,
        "a file kelpie already sent edits nothing: {:?}",
        pulls.calls()
    );

    left(&rig, "fix(worker): the second text\n\nbody\n");
    assert_eq!(
        after_turn(&ctx),
        Outcome::Edited(NonZeroU64::new(5).unwrap())
    );
    assert_eq!(
        pulls.calls().last(),
        Some(&PullCall::Edit {
            number: NonZeroU64::new(5).unwrap(),
            title: "fix(worker): the second text".to_owned(),
            body: "body\n\nResolves #74".to_owned(),
        })
    );
}

// A forge that cannot be asked leaves the file unapplied, so the next turn's end
// tries the same file again.
#[test]
fn a_forge_failure_is_failed_and_leaves_the_file_unapplied_for_a_retry() {
    let rig = Rig::new("hand over failed");
    let worktree = worker(&rig);
    commit(&worktree, "src/lib.rs", "work\n");
    let pulls = FakePulls::new();
    pushed(&rig, &worktree, &pulls);
    pulls.script_create(Err(ForgeError::Failed("HTTP 502: Bad Gateway".to_owned())));
    left(
        &rig,
        "feat(worker): a turn the forge refused\n\nWhy.\n\nResolves #74\n",
    );
    let tickets = FakeTickets::new();
    let ctx = hand_over(&rig, &pulls, &tickets);

    assert_eq!(
        after_turn(&ctx),
        Outcome::Failed(ForgeError::Failed("HTTP 502: Bad Gateway".to_owned()))
    );
    assert_eq!(
        applied(&rig),
        None,
        "nothing was sent, so nothing is applied"
    );

    pulls.script_create(Ok(NonZeroU64::new(9).unwrap()));
    assert_eq!(
        after_turn(&ctx),
        Outcome::Opened(NonZeroU64::new(9).unwrap())
    );
    assert_eq!(
        applied(&rig).as_deref(),
        Some("feat(worker): a turn the forge refused\n\nWhy.\n\nResolves #74\n"),
        "the file that went through is applied"
    );
}

#[test]
fn the_first_line_is_the_title_and_the_rest_the_body() {
    let text = parse("feat(guard): scope the token\n\nWhy.\n\nResolves #420\n").unwrap();

    assert_eq!(
        text,
        Text {
            title: "feat(guard): scope the token".to_owned(),
            body: "Why.\n\nResolves #420".to_owned(),
        }
    );
}

#[test]
fn a_heading_mark_and_leading_blank_lines_are_not_part_of_the_title() {
    let text = parse("\n\n# fix: a thing\nbody").unwrap();

    assert_eq!(text.title, "fix: a thing");
    assert_eq!(text.body, "body");
}

#[test]
fn a_file_with_no_title_opens_nothing() {
    assert_eq!(parse(""), Err(Untitled::Blank));
    assert_eq!(parse("  \n\n##\nbody"), Err(Untitled::Blank));
}

#[test]
fn a_breaking_scoped_subject_is_a_title() {
    let text = parse("feat(guard)!: scope the token\nbody").unwrap();

    assert_eq!(text.title, "feat(guard)!: scope the token");
}

#[test]
fn a_first_line_that_is_not_a_conventional_commit_subject_is_no_title() {
    for line in ["## Short summary", "feat:", "feat: ", "Feat: x"] {
        let title = line.trim_start_matches('#').trim().to_owned();
        assert_eq!(
            parse(&format!("{line}\nbody")),
            Err(Untitled::NotConventional(title)),
            "{line}"
        );
    }
}

// Only the blank lines around the body go: str::trim would shave the four spaces
// off a body that opens with an indented code block, and join would then read its
// lines as one paragraph.
#[test]
fn a_body_keeps_the_indent_of_its_first_line_and_drops_its_blank_ends() {
    let text = parse("feat(x): a thing\n\n    cargo test --all\n    second line\n\n \n").unwrap();

    assert_eq!(text.body, "    cargo test --all\n    second line");
    assert_eq!(parse("feat(x): a thing\n\n   \n").unwrap().body, "");
}
