//! The two shapes GitHub lists tickets in, read into [`Listed`]

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::unreadable;
use crate::github::Timestamp;
use crate::ticket::{ForgeError, Listed, TicketId};

/// GitHub's ISO 8601 moment, such as `2026-10-14T08:00:00Z`, in whole seconds
///
/// `None` for text that is not one, or a moment before the Unix epoch.
pub(super) fn time(text: &str) -> Option<Timestamp> {
    let at: jiff::Timestamp = text.parse().ok()?;
    u64::try_from(at.as_second()).ok().map(Timestamp::from_secs)
}

/// What `gh issue list --json number,title,labels,author,createdAt` printed
///
/// An entry that does not parse is left out, with one line on stderr.
///
/// # Errors
///
/// [`ForgeError::Unreadable`] when the output is not a JSON list.
pub(super) fn gh_list(stdout: &[u8]) -> Result<Vec<Listed>, ForgeError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Row {
        number: u64,
        title: String,
        labels: Vec<Label>,
        author: Option<Author>,
        created_at: String,
    }
    let rows: Vec<Value> = serde_json::from_slice(stdout).map_err(|_| unreadable(stdout))?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            skipped(row, |row: Row| {
                Some(Listed {
                    id: TicketId::from_issue(row.number)?,
                    title: row.title,
                    labels: row.labels.into_iter().map(|l| l.name).collect(),
                    author: row.author.map(|a| a.login).unwrap_or_default(),
                    created: time(&row.created_at)?,
                })
            })
        })
        .collect())
}

/// An issue of GitHub's REST list, with when it closed
#[derive(Debug)]
pub(super) struct Entry {
    pub(super) listed: Listed,
    /// `None` for an issue that is open
    pub(super) closed: Option<Timestamp>,
}

/// One page of `repos/<slug>/issues`, leaving out the pull requests it lists
/// beside the issues
///
/// An entry that does not parse is left out, with one line on stderr.
///
/// # Errors
///
/// [`ForgeError::Unreadable`] when the page is not a JSON list.
pub(super) fn rest_page(page: &[u8]) -> Result<Vec<Entry>, ForgeError> {
    #[derive(Deserialize)]
    struct Row {
        number: u64,
        title: String,
        labels: Vec<Label>,
        user: Option<Author>,
        created_at: String,
        closed_at: Option<String>,
    }
    let rows: Vec<Value> = serde_json::from_slice(page).map_err(|_| unreadable(page))?;
    Ok(rows
        .into_iter()
        .filter(|row| row.get("pull_request").is_none())
        .filter_map(|row| {
            skipped(row, |row: Row| {
                Some(Entry {
                    listed: Listed {
                        id: TicketId::from_issue(row.number)?,
                        title: row.title,
                        labels: row.labels.into_iter().map(|l| l.name).collect(),
                        author: row.user.map(|u| u.login).unwrap_or_default(),
                        created: time(&row.created_at)?,
                    },
                    closed: match row.closed_at {
                        Some(text) => Some(time(&text)?),
                        None => None,
                    },
                })
            })
        })
        .collect())
}

/// `at` as GitHub reads a moment in a query, such as `2026-11-05T00:00:00Z`
pub(super) fn moment(at: Timestamp) -> Option<String> {
    let seconds = i64::try_from(at.secs()).ok()?;
    jiff::Timestamp::from_second(seconds)
        .ok()
        .map(|moment| moment.to_string())
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

#[derive(Deserialize)]
struct Author {
    login: String,
}

// `row` read by `read`, or `None` with a line on stderr naming the entry when it
// does not parse
fn skipped<R: DeserializeOwned, T>(row: Value, read: impl FnOnce(R) -> Option<T>) -> Option<T> {
    let number = row.get("number").and_then(Value::as_u64);
    let read = serde_json::from_value(row).ok().and_then(read);
    if read.is_none() {
        match number {
            Some(number) => eprintln!("skipped ticket {number}: its entry does not parse"),
            None => eprintln!("skipped a ticket: its entry does not parse"),
        }
    }
    read
}
