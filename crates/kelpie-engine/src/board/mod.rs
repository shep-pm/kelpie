//! Which column each ticket is in, and the order it sits in
//!
//! A column is a fact read from GitHub and the engine, never a place the
//! maintainer puts a ticket (`docs/design/tickets.md`, The board). [`place()`]
//! turns what one poll read into a [`Board`], and [`plan_move()`] says what a
//! drop on a column does. Both are pure, so the poller and the Board panel call
//! the same code. How far back Done reaches is the `done-window` setting, which
//! the poller turns into the `since` of its closed list, so nothing here holds a
//! window.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::board::{Column, Move, plan_move};
//!
//! assert_eq!(plan_move(Column::Ready, Column::Building), Move::Start);
//! assert_eq!(plan_move(Column::Review, Column::Review), Move::Stay);
//! ```

mod columns;
mod moves;
#[cfg(test)]
mod tests;

use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

pub use columns::{Read, Rules, place};
pub use moves::{Move, plan_move};

use crate::github::Timestamp;
use crate::ticket::TicketId;

/// One of the board's five columns, in the order the board shows them
// wire format: the Board panel names a column by these lower-case words
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Column {
    /// Its issue is open without the ready label
    Draft,
    /// Its issue carries the ready label
    Ready,
    /// A worker on it is live, Paused and waiting on the maintainer included
    Building,
    /// A pull request closing it, or on its branch, is open
    Review,
    /// Its issue is closed
    Done,
}

/// One ticket, and the column the board puts it in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// Its id
    pub id: TicketId,
    /// Its title
    pub title: String,
    /// The number of its `priority: P<n>` label, the lowest when it has several,
    /// `None` when it has none
    pub priority: Option<u8>,
    /// When it was opened
    pub created: Timestamp,
    /// Where it sits
    pub column: Column,
    /// The lowest-numbered open pull request closing it or on its branch
    pub pull: Option<NonZeroU64>,
}

/// Every ticket of a project, each in its column
///
/// Tickets are held column by column in [`Column`]'s order, and within a column
/// by priority, then oldest first, then id. A ticket with no priority sorts
/// after one with any.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Board {
    tickets: Vec<Placed>,
}

impl Board {
    /// Every ticket, column by column
    #[must_use]
    pub fn tickets(&self) -> &[Placed] {
        &self.tickets
    }

    /// The tickets in `column`, in the order the board shows them
    pub fn column(&self, column: Column) -> impl Iterator<Item = &Placed> {
        self.tickets
            .iter()
            .filter(move |placed| placed.column == column)
    }

    fn sorted(mut tickets: Vec<Placed>) -> Self {
        tickets.sort_by_key(|placed| {
            (
                placed.column,
                placed.priority.unwrap_or(u8::MAX),
                placed.created,
                placed.id,
            )
        });
        Self { tickets }
    }
}
