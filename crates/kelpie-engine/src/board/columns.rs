//! The rule that puts a ticket in a column

use std::collections::BTreeMap;

use super::{Board, Column, Placed};
use crate::settings::PickUp;
use crate::ticket::{Listed, OpenPull, TicketId};
use crate::worker::{Record, Status};

/// What one poll read
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
    /// The open issues carrying the ready label
    pub ready: Vec<Listed>,
    /// Every open issue
    pub open: Vec<Listed>,
    /// The issues closed within the Done window
    pub closed: Vec<Listed>,
    /// Every open pull request
    pub pulls: Vec<OpenPull>,
}

/// The project's choices that say which issues are tickets and which are ready
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    /// Whose issues are picked up
    pub pick_up: PickUp,
    /// The label a Ready ticket carries
    pub ready_label: String,
    /// The signed-in login, whose issues Me picks up
    pub login: String,
}

/// The prefix of a priority label, `priority: P0` to `priority: P4`
const PRIORITY: &str = "priority: P";

/// The highest number a priority label may carry, the lowest priority
const LOWEST_PRIORITY: u8 = 4;

/// Puts each ticket `read` lists in its column
///
/// A listed issue is a ticket when it carries the ready label, when the
/// setting is Everyone, or when its author is the signed-in login, compared
/// without regard to case. Every entry of the ready list is a ticket, since
/// GitHub matched its label without regard to case. An issue listed more than once is one ticket: the
/// closed list's entry wins, then the ready list's, then the open list's.
///
/// A ticket's column is the first that holds: Done when its issue is closed,
/// Building when a worker on it is live, Review when an open pull request
/// closes it or is opened from its `kelpie/<id>` branch, Ready when it is
/// ready, and Draft otherwise.
///
/// A worker is live while it runs or may yet run: Running, Paused, or waiting on
/// the maintainer with no pull request of its own, such as one whose draft could
/// not open. A worker waiting on the maintainer with its pull request open is not
/// live, since nothing runs for it, and its ticket stays in Review: the wait is
/// the ticket's needs-you line there, and the card must not jump to Building
/// (`docs/design/gate-and-merge.md`, The Review column).
#[must_use]
pub fn place(read: &Read, workers: &[Record], rules: &Rules) -> Board {
    let mut entries: BTreeMap<TicketId, Entry<'_>> = BTreeMap::new();
    let lists = [
        (&read.open, false),
        (&read.ready, true),
        (&read.closed, false),
    ];
    for (index, (list, listed_ready)) in lists.into_iter().enumerate() {
        let closed = index == 2;
        for listed in list
            .iter()
            .filter(|listed| listed_ready || is_ticket(listed, rules))
        {
            let ready = listed_ready || carries(listed, &rules.ready_label);
            entries
                .entry(listed.id)
                .and_modify(|entry| {
                    entry.ready |= ready;
                    entry.closed |= closed;
                    if closed || listed_ready {
                        entry.listed = listed;
                    }
                })
                .or_insert(Entry {
                    listed,
                    ready,
                    closed,
                });
        }
    }
    Board::sorted(
        entries
            .into_values()
            .map(|entry| placed(&entry, workers, &read.pulls))
            .collect(),
    )
}

/// One ticket as the lists read it
struct Entry<'a> {
    listed: &'a Listed,
    ready: bool,
    closed: bool,
}

fn carries(listed: &Listed, label: &str) -> bool {
    listed.labels.iter().any(|name| name == label)
}

fn is_ticket(listed: &Listed, rules: &Rules) -> bool {
    carries(listed, &rules.ready_label)
        || rules.pick_up == PickUp::Everyone
        || listed.author.eq_ignore_ascii_case(&rules.login)
}

fn placed(entry: &Entry<'_>, workers: &[Record], pulls: &[OpenPull]) -> Placed {
    let id = entry.listed.id;
    let name = id.to_string();
    let branch = format!("kelpie/{name}");
    let pull = pulls
        .iter()
        .filter(|pull| pull.closes.contains(&id) || pull.branch == branch)
        .map(|pull| pull.number)
        .min();
    // A worker waiting on the maintainer runs nothing and holds no slot, so with its
    // pull request open it is not live: its ticket stays in Review, where the wait
    // shows as its needs-you line (`docs/design/gate-and-merge.md`, The Review column).
    // A worker whose pull request is being read holds a slot, and its ticket stays in
    // Review for the read (`docs/design/review.md`, What the maintainer sees)
    let live = |status: Status| {
        status != Status::Ended
            && (pull.is_none()
                || !matches!(status, Status::WaitingOnMaintainer | Status::Reviewing))
    };
    let building = workers
        .iter()
        .any(|record| record.ticket == name && live(record.status));
    let column = if entry.closed {
        Column::Done
    } else if building {
        Column::Building
    } else if pull.is_some() {
        Column::Review
    } else if entry.ready {
        Column::Ready
    } else {
        Column::Draft
    };
    Placed {
        id,
        title: entry.listed.title.clone(),
        priority: priority(&entry.listed.labels),
        created: entry.listed.created,
        column,
        pull,
    }
}

fn priority(labels: &[String]) -> Option<u8> {
    labels
        .iter()
        .filter_map(|label| label.strip_prefix(PRIORITY)?.parse::<u8>().ok())
        .filter(|number| *number <= LOWEST_PRIORITY)
        .min()
}
