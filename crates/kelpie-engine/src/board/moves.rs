//! What a drop on a column does

use super::Column;

/// What the board does with a ticket dropped on a column
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// Starts a worker on the ticket, the same as its Start button
    Start,
    /// The maintainer's yes to merging the ticket's pull request, the same as its Merge button
    Merge,
    /// The ticket is already there
    Stay,
    /// The ticket goes back where it was, and the board shows this line
    SnapBack(&'static str),
}

const READY_LABEL: &str =
    "Writing the ready label waits for Phase 3: mark the ticket ready on GitHub.";
const FOLLOWS: &str = "A ticket's column follows its worker and its issue.";

/// What dropping a ticket from column `from` onto column `to` does
#[must_use]
pub fn plan_move(from: Column, to: Column) -> Move {
    match (from, to) {
        _ if from == to => Move::Stay,
        (Column::Ready, Column::Building) => Move::Start,
        (Column::Draft, Column::Ready) => Move::SnapBack(READY_LABEL),
        (Column::Review, Column::Done) => Move::Merge,
        _ => Move::SnapBack(FOLLOWS),
    }
}
