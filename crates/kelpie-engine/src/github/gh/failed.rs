//! What a failed `gh` call said, and when it told kelpie to ask again
//!
//! Every call runs with `GH_DEBUG=api`, which makes `gh` print each HTTP
//! exchange on stderr, headers prefixed `< `. A failed call keeps its own
//! error lines, and a rate-limited one reads `Retry-After` and
//! `X-RateLimit-Reset` from those headers.

use crate::github::Cooldown;
use crate::ticket::ForgeError;

// gh's debug lines for a request, a response and a note on either
const TRAFFIC: [&str; 3] = ["* ", "> ", "< "];

// What gh says when a limit is used up, lowercased: the REST and GraphQL
// primary limits, the secondary limit, and an HTTP 429 answer
const USED_UP: [&str; 4] = [
    "api rate limit exceeded",
    "api rate limit already exceeded",
    "secondary rate limit",
    "http 429",
];

/// The error for a call that exited unsuccessfully with `stderr`
pub(super) fn failed(stderr: &str) -> ForgeError {
    let lines: Vec<&str> = stderr.lines().collect();
    let is_traffic = |line: &&str| {
        TRAFFIC.iter().any(|mark| line.starts_with(mark)) || matches!(*line, "<" | ">")
    };
    let traffic = lines.iter().any(is_traffic);
    let cooldown = if traffic {
        cooldown_of(&lines)
    } else {
        Cooldown::default()
    };
    let own: Vec<&str> = lines.iter().copied().filter(|l| !is_traffic(l)).collect();
    // What gh adds after the exchange is its own line; the body is not.
    let pick =
        |keep: fn(&str) -> bool| -> Vec<&str> { own.iter().copied().filter(|l| keep(l)).collect() };
    let (gh, status) = (pick(|l| l.starts_with("gh: ")), pick(is_status));
    // A GraphQL call's query and variables are printed as plain lines, so its
    // own error line is picked before the fallback keeps what is left.
    let graphql = pick(|l| l.starts_with("GraphQL: "));
    let said = if !gh.is_empty() {
        gh
    } else if !status.is_empty() {
        status
    } else if !graphql.is_empty() {
        graphql
    } else if traffic {
        pick(|l| !l.trim_start().starts_with(['{', '[', '}', ']', '"']))
    } else {
        return limited_or(stderr.trim_end(), cooldown);
    };
    limited_or(&said.join("\n"), cooldown)
}

// gh's line for a call that got an HTTP error, such as `HTTP 504: 504 Gateway
// Timeout (https://api.github.com/graphql)`, after the page GitHub answered with.
fn is_status(line: &str) -> bool {
    line.strip_prefix("HTTP ")
        .and_then(|rest| rest.split_once(':'))
        .is_some_and(|(code, _)| code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether `stderr` says a rate limit is used up
pub(super) fn says_used_up(stderr: &str) -> bool {
    let lowered = stderr.to_lowercase();
    USED_UP.iter().any(|said| lowered.contains(said))
}

// A used-up limit is `Limited` with what the answer said of when, anything
// else stays what gh printed.
fn limited_or(kept: &str, cooldown: Cooldown) -> ForgeError {
    if says_used_up(kept) {
        ForgeError::Limited {
            said: format!("gh failed: {}", kept.trim()),
            cooldown,
        }
    } else {
        ForgeError::Failed(kept.to_owned())
    }
}

// The two headers of the last response in the debug lines
fn cooldown_of(lines: &[&str]) -> Cooldown {
    let (mut retry_after, mut reset) = ("", "");
    for line in lines {
        let Some(header) = line.strip_prefix("< ") else {
            continue;
        };
        if header.starts_with("HTTP/") {
            (retry_after, reset) = ("", "");
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        match name.to_ascii_lowercase().as_str() {
            "retry-after" => retry_after = value,
            "x-ratelimit-reset" => reset = value,
            _ => {}
        }
    }
    Cooldown::read(retry_after, reset)
}
