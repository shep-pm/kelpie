//! REST reads that send the last answer's `ETag` and keep the answer a 304 confirms
//!
//! GitHub does not count a 304 against the hourly limit. [`Etags`] keeps the
//! tag and body of each path's last answer and sends the tag as
//! `If-None-Match` the next time, through `gh api --include`. A launch starts
//! with nothing kept: the cache is in memory. GraphQL has no conditional
//! requests, so only the REST reads go through here.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use serde::de::IgnoredAny;

use super::failed::failed;
use super::hold::Hold;
use super::{Gh, unreadable};
use crate::ticket::ForgeError;

// The `Accept` every conditional read sends, since a tag holds only for the
// `Accept` it was given
const ACCEPT: &str = "Accept: application/vnd.github+json";

/// The items on a page of a list the reads ask for
pub const PER_PAGE: usize = 100;

// The pages a list is read to, past which it is cut off
const MAX_PAGES: usize = 50;

// The answers kept at once, past which all are dropped
const KEPT: usize = 512;

// What one request answered
enum Fetched {
    // A new answer, with the tag GitHub gave it where it gave one
    Fresh { etag: Option<String>, body: Vec<u8> },
    // The answer the request's tag names is still the answer
    NotModified,
}

#[derive(Debug, Clone)]
struct Kept {
    etag: String,
    body: Vec<u8>,
}

/// The answers kept for conditional reads, keyed by path
#[derive(Debug, Default)]
pub struct Etags {
    kept: Mutex<BTreeMap<String, Kept>>,
    // Set once a request failed naming a 304 that the same one without a
    // tag did not, so `gh` is not passing a 304 on and no tag is sent again
    unsent: AtomicBool,
}

impl Etags {
    /// Nothing kept
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `path`'s answer through `hold`: the kept one when GitHub says it is unchanged
    ///
    /// # Errors
    ///
    /// [`ForgeError::Held`] while a limit is used up, the failure `gh` printed
    /// when the read fails, and [`ForgeError::Unreadable`] when GitHub answers
    /// 304 to a read that sent no tag.
    pub fn get(&self, hold: &Hold, path: &str) -> Result<Vec<u8>, ForgeError> {
        let kept = self
            .kept()
            .get(path)
            .cloned()
            .filter(|_| !self.unsent.load(Ordering::Relaxed));
        let sent = kept.as_ref().map(|k| k.etag.as_str());
        let fetched = match fetch(hold, path, sent) {
            Err(ForgeError::Failed(said)) if sent.is_some() && names_304(&said) => {
                let plain = fetch(hold, path, None)?;
                self.unsent.store(true, Ordering::Relaxed);
                plain
            }
            answer => answer?,
        };
        match (fetched, kept) {
            (Fetched::NotModified, Some(kept)) => Ok(kept.body),
            (Fetched::NotModified, None) => Err(ForgeError::Unreadable(
                "GitHub answered 304 to a read that sent no tag".to_owned(),
            )),
            (Fetched::Fresh { etag, body }, _) => {
                let mut all = self.kept();
                match etag {
                    Some(etag) => {
                        if all.len() >= KEPT {
                            all.clear();
                        }
                        let body = body.clone();
                        all.insert(path.to_owned(), Kept { etag, body });
                    }
                    None => {
                        all.remove(path);
                    }
                }
                Ok(body)
            }
        }
    }

    /// Every page of the list at `path`, which carries `per_page=100` already
    ///
    /// Each page is its own conditional read, and the walk ends at the first
    /// page with fewer than [`PER_PAGE`] items.
    ///
    /// # Errors
    ///
    /// As [`Etags::get`], and [`ForgeError::Unreadable`] when a page is not a
    /// JSON list or the list runs past 50 pages.
    pub fn pages(&self, hold: &Hold, path: &str) -> Result<Vec<Vec<u8>>, ForgeError> {
        let mut pages = Vec::new();
        for page in 1..=MAX_PAGES {
            let body = self.get(hold, &format!("{path}&page={page}"))?;
            let items: Vec<IgnoredAny> =
                serde_json::from_slice(&body).map_err(|_| unreadable(&body))?;
            let full = items.len() >= PER_PAGE;
            pages.push(body);
            if !full {
                return Ok(pages);
            }
        }
        Err(ForgeError::Unreadable(format!(
            "{path} has more than {MAX_PAGES} pages of {PER_PAGE}"
        )))
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Kept>> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

// One request for `path` with `gh api`, with `etag` as `If-None-Match` when given
fn fetch(hold: &Hold, path: &str, etag: Option<&str>) -> Result<Fetched, ForgeError> {
    hold.guarded(|gh| request(gh, path, etag))
}

fn request(gh: &Gh, path: &str, etag: Option<&str>) -> Result<Fetched, ForgeError> {
    let mut args = ["api", "--include", "-H", ACCEPT]
        .map(str::to_owned)
        .to_vec();
    if let Some(etag) = etag {
        args.extend(["-H".to_owned(), format!("If-None-Match: {etag}")]);
    }
    args.push(path.to_owned());
    let output = gh
        .runner
        .run(&args)
        .map_err(|e| ForgeError::Spawn(e.to_string()))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let (status, etag, body) = split(&output.stdout);
    // Whether `gh` exits 0 on a 304 is not measured, so a 304 is read from the
    // output or from the debug lines on stderr whatever the exit code.
    if status == Some(304) || says_not_modified(&stderr) {
        return Ok(Fetched::NotModified);
    }
    if !output.status.success() {
        return Err(failed(&stderr));
    }
    match status {
        Some(200..=299) => Ok(Fetched::Fresh {
            etag,
            body: body.to_vec(),
        }),
        _ => Err(unreadable(&output.stdout)),
    }
}

// Whether a failure says the status was 304, as `HTTP 304: Not Modified`. The
// digits alone do not count: a path in the error's URL may hold them.
fn names_304(said: &str) -> bool {
    let said = said.to_ascii_lowercase();
    said.contains("http 304") || said.contains("not modified")
}

// The debug lines `GH_DEBUG=api` prints for the response
fn says_not_modified(stderr: &str) -> bool {
    stderr.lines().any(|line| {
        let line = line.trim_start_matches("< ");
        let mut words = line.split_whitespace();
        (words.next().is_some_and(|w| w.starts_with("HTTP/")) && words.next() == Some("304"))
            || line.starts_with("gh: HTTP 304")
    })
}

// The status, the `ETag` and the body of an `--include` answer: the status and
// headers, a blank line and the body
fn split(stdout: &[u8]) -> (Option<u16>, Option<String>, &[u8]) {
    let blank = |i: usize| stdout[i..].starts_with(b"\r\n\r\n") || stdout[i..].starts_with(b"\n\n");
    let Some(end) = (0..stdout.len()).find(|&i| blank(i)) else {
        return (None, None, stdout);
    };
    let gap = if stdout[end..].starts_with(b"\r\n\r\n") {
        4
    } else {
        2
    };
    let head = String::from_utf8_lossy(&stdout[..end]);
    let mut lines = head.lines();
    let status = (lines.next())
        .filter(|line| line.starts_with("HTTP/"))
        .and_then(|line| line.split_whitespace().nth(1)?.parse().ok());
    let etag = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("etag")
            .then(|| value.trim().to_owned())
    });
    (status, etag, &stdout[end + gap..])
}
