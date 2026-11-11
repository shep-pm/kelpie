//! The board's commands: what the Board panel calls
//!
//! [`board_watch`] starts the project's poller and streams a [`BoardEvent`] to
//! the window whenever the board changes. The [`Poller`](poll::Poller) reads
//! GitHub once a minute and places the tickets again every two seconds from the
//! workers' records, so a Start moves its ticket to Building at once
//! (`docs/design/tickets.md`, The board and GitHub). [`board_refresh`] reads at
//! once, and [`board_move`] runs what a drop on a column does
//! ([`kelpie_engine::board::plan_move`]). [`open_link::open_link`] sends a
//! ticket's page to the OS's own broker, so the browser never descends from
//! kelpie (ADR 0001).
//!
//! The window's types are here, mapped from the engine's [`kelpie_engine::board`]
//! ones, so the engine gains no specta dependency. Kelpie polls while it runs
//! and nothing polls once it quits: [`BoardHost::stop`] ends the poller.

use kelpie_engine::board::{self as engine, Board};
use kelpie_engine::github::{ForgeSlug, Timestamp};
use serde::{Deserialize, Serialize};
use specta::Type;

pub mod link;
pub mod poll;

#[cfg(test)]
mod tests;

/// One of the board's five columns, as the window names it
// wire format: the Board panel names a column by these lower-case words
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    /// Its issue is open without the ready label
    Draft,
    /// Its issue carries the ready label
    Ready,
    /// A worker on it is live
    Building,
    /// A pull request closing it is open
    Review,
    /// Its issue is closed
    Done,
}

impl From<engine::Column> for Column {
    fn from(column: engine::Column) -> Self {
        match column {
            engine::Column::Draft => Self::Draft,
            engine::Column::Ready => Self::Ready,
            engine::Column::Building => Self::Building,
            engine::Column::Review => Self::Review,
            engine::Column::Done => Self::Done,
        }
    }
}

/// One ticket on the board, with the pages it links out to
// wire format: the Board panel reads these fields
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct TicketView {
    /// Its number
    pub id: String,
    /// Its title
    pub title: String,
    /// The number of its priority label, `None` when it has none
    pub priority: Option<u8>,
    /// Where it sits
    pub column: Column,
    /// Its issue's page on GitHub
    pub url: String,
    /// The page of the pull request closing it, `None` when none is open
    pub pull_url: Option<String>,
}

/// Every ticket of the project, column by column, as the window shows them
// wire format: the Board panel reads these fields
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
pub struct BoardView {
    /// The tickets, in the order the board shows them
    pub tickets: Vec<TicketView>,
    /// While a rate limit is used up, the Unix second the hold lifts
    pub held_until: Option<u64>,
}

impl BoardView {
    /// The view of `board`, whose tickets live in `repo`
    #[must_use]
    pub fn of(board: &Board, repo: &ForgeSlug, held_until: Option<Timestamp>) -> Self {
        let page = |kind: &str, number: &dyn std::fmt::Display| {
            format!("https://github.com/{repo}/{kind}/{number}")
        };
        let tickets = board
            .tickets()
            .iter()
            .map(|placed| TicketView {
                id: placed.id.to_string(),
                title: placed.title.clone(),
                priority: placed.priority,
                column: placed.column.into(),
                url: page("issues", &placed.id),
                pull_url: placed.pull.map(|number| page("pull", &number)),
            })
            .collect();
        Self {
            tickets,
            held_until: held_until.map(Timestamp::secs),
        }
    }
}

/// What the board streams to the window
// wire format: the Board panel switches on `kind`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BoardEvent {
    /// The board as it stands now
    Board(BoardView),
    /// GitHub cannot be read, and what to do about it
    Unavailable {
        /// What to tell the maintainer
        message: String,
    },
}
