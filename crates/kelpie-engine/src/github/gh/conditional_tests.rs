//! Conditional reads, against a scripted `gh` that stands in for GitHub
//!
//! The [`Runner`] answers as `gh api --include` does: 304 to a tag it gave, and
//! a new tag when the answer changed. No test needs `gh`, a login or the network.

use std::collections::BTreeMap;
use std::io;
use std::process::Output;
use std::sync::{Arc, Mutex};

use super::conditional::{Etags, PER_PAGE};
use super::hold::Hold;
use super::tests::{answered, refused};
use super::{Gh, Runner};
use crate::ticket::ForgeError;

// How the stand-in answers a request that carries a tag it gave
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confirms {
    // A 304 in the output, with exit 0
    InOutput,
    // A 304 only in the debug lines, with a failing exit
    InStderr,
    // An error that names a 304 in words kelpie does not know
    Odd,
    // An error that names nothing of a 304
    Never,
}

#[derive(Debug)]
struct State {
    // Each path's version and body: the tag changes when the answer does
    answers: BTreeMap<String, (u32, String)>,
    // Each request's path and the tag it sent
    asked: Vec<(String, Option<String>)>,
    confirms: Confirms,
    // A failure every request gets while it is set
    fails_with: Option<String>,
}

// GitHub's side of the reads, as the `gh` child shows it
#[derive(Debug, Clone)]
struct FakeGithub(Arc<Mutex<State>>);

impl FakeGithub {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            answers: BTreeMap::new(),
            asked: Vec::new(),
            confirms: Confirms::InOutput,
            fails_with: None,
        })))
    }

    fn answers(&self, path: &str, body: &str) {
        let mut state = self.0.lock().unwrap();
        let version = state.answers.get(path).map_or(1, |(v, _)| v + 1);
        state
            .answers
            .insert(path.to_owned(), (version, body.to_owned()));
    }

    fn confirms(&self, confirms: Confirms) {
        self.0.lock().unwrap().confirms = confirms;
    }

    fn fails_with(&self, said: Option<&str>) {
        self.0.lock().unwrap().fails_with = said.map(str::to_owned);
    }

    fn asked(&self) -> Vec<(String, Option<String>)> {
        self.0.lock().unwrap().asked.clone()
    }

    fn hold(&self) -> Hold {
        Hold::new(Gh::with_runner(Box::new(self.clone())))
    }
}

impl Runner for FakeGithub {
    fn run(&self, args: &[String]) -> io::Result<Output> {
        let mut state = self.0.lock().unwrap();
        let path = args.last().expect("a path").clone();
        let sent = args
            .iter()
            .find_map(|arg| arg.strip_prefix("If-None-Match: "))
            .map(str::to_owned);
        state.asked.push((path.clone(), sent.clone()));
        if let Some(said) = &state.fails_with {
            return refused(said);
        }
        let Some((version, body)) = state.answers.get(&path).cloned() else {
            return refused(&format!("gh: Not Found (HTTP 404)\nHTTP 404: {path}"));
        };
        let tag = format!("W/\"{path}-{version}\"");
        if sent.as_deref() == Some(tag.as_str()) {
            return match state.confirms {
                Confirms::InOutput => answered("HTTP/2.0 304 Not Modified\r\n\r\n"),
                Confirms::InStderr => refused("< HTTP/2.0 304 Not Modified\n<\ngh: HTTP 304\n"),
                Confirms::Odd => refused("gh: unexpected 304 from the server"),
                Confirms::Never => refused("gh: Bad Gateway (HTTP 502)"),
            };
        }
        answered(&format!(
            "HTTP/2.0 200 OK\r\nContent-Type: application/json\r\nEtag: {tag}\r\n\r\n{body}"
        ))
    }
}

fn full_page() -> String {
    format!("[{}]", vec!["{}"; PER_PAGE].join(","))
}

#[test]
fn a_304_answers_with_the_cached_body() {
    let github = FakeGithub::new();
    github.answers("repos/o/r/issues/comments", "[1]");
    let (hold, etags) = (github.hold(), Etags::new());

    let first = etags.get(&hold, "repos/o/r/issues/comments").unwrap();
    let second = etags.get(&hold, "repos/o/r/issues/comments").unwrap();

    assert_eq!((first, second), (b"[1]".to_vec(), b"[1]".to_vec()));
    assert_eq!(
        github.asked(),
        [
            ("repos/o/r/issues/comments".to_owned(), None),
            (
                "repos/o/r/issues/comments".to_owned(),
                Some("W/\"repos/o/r/issues/comments-1\"".to_owned())
            ),
        ]
    );
}

#[test]
fn a_changed_tag_replaces_the_cached_body() {
    let github = FakeGithub::new();
    github.answers("p", "[1]");
    let (hold, etags) = (github.hold(), Etags::new());
    etags.get(&hold, "p").unwrap();
    github.answers("p", "[1,2]");

    let changed = etags.get(&hold, "p").unwrap();
    let kept = etags.get(&hold, "p").unwrap();

    assert_eq!((changed, kept), (b"[1,2]".to_vec(), b"[1,2]".to_vec()));
    assert_eq!(github.asked()[2].1.as_deref(), Some("W/\"p-2\""));
}

#[test]
fn a_paged_list_is_walked_to_its_last_page() {
    let github = FakeGithub::new();
    github.answers("list?per_page=100&page=1", &full_page());
    github.answers("list?per_page=100&page=2", &full_page());
    github.answers("list?per_page=100&page=3", "[{}]");
    let (hold, etags) = (github.hold(), Etags::new());

    let first = etags.pages(&hold, "list?per_page=100").unwrap();
    let again = etags.pages(&hold, "list?per_page=100").unwrap();

    assert_eq!(first.len(), 3);
    assert_eq!(first, again);
    assert_eq!(first[2], b"[{}]".to_vec());
    let asked = github.asked();
    assert_eq!(asked.len(), 6, "a page at a time, and no page 4");
    assert!(asked[3..].iter().all(|(_, tag)| tag.is_some()), "{asked:?}");
}

#[test]
fn a_304_printed_only_in_the_debug_lines_still_answers_with_the_cached_body() {
    let github = FakeGithub::new();
    github.answers("p", "[1]");
    github.confirms(Confirms::InStderr);
    let (hold, etags) = (github.hold(), Etags::new());
    etags.get(&hold, "p").unwrap();

    assert_eq!(etags.get(&hold, "p").unwrap(), b"[1]".to_vec());
}

#[test]
fn a_read_that_fails_naming_a_304_is_made_again_without_a_tag_and_no_tag_is_sent_after() {
    let github = FakeGithub::new();
    github.answers("p", "[1]");
    github.confirms(Confirms::Odd);
    let (hold, etags) = (github.hold(), Etags::new());
    etags.get(&hold, "p").unwrap();

    let read = etags.get(&hold, "p").unwrap();
    etags.get(&hold, "p").unwrap();

    assert_eq!(read, b"[1]".to_vec());
    let asked = github.asked();
    assert_eq!(asked.len(), 4, "{asked:?}");
    assert_eq!(asked[1].1.as_deref(), Some("W/\"p-1\""));
    assert_eq!(asked[2].1, None);
    assert_eq!(asked[3].1, None, "no tag after one failed");
}

#[test]
fn a_failure_that_names_no_304_is_that_reads_failure_and_tags_stay_on() {
    let github = FakeGithub::new();
    github.answers("p", "[1]");
    github.confirms(Confirms::Never);
    let (hold, etags) = (github.hold(), Etags::new());
    etags.get(&hold, "p").unwrap();

    let error = etags.get(&hold, "p").unwrap_err();

    assert!(matches!(error, ForgeError::Failed(_)), "{error:?}");
    assert!(github.asked()[1].1.is_some());
    assert_eq!(github.asked().len(), 2, "not made again");
}

#[test]
fn an_answer_that_is_not_a_list_is_unreadable() {
    let github = FakeGithub::new();
    github.answers("list?per_page=100&page=1", "{}");
    let (hold, etags) = (github.hold(), Etags::new());

    let error = etags.pages(&hold, "list?per_page=100").unwrap_err();

    assert!(matches!(error, ForgeError::Unreadable(_)), "{error:?}");
}

#[test]
fn a_failed_read_is_the_failure_gh_printed() {
    let github = FakeGithub::new();
    github.fails_with(Some("gh: Not Found (HTTP 404)"));
    let (hold, etags) = (github.hold(), Etags::new());

    assert_eq!(
        etags.get(&hold, "p"),
        Err(ForgeError::Failed("gh: Not Found (HTTP 404)".to_owned()))
    );
}
