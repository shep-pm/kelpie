//! A project's tickets, and the port they are read through
//!
//! A project has one ticket source, and the source is the ticket's truth
//! (`docs/design/tickets.md`, Where tickets come from). [`TicketId`] names a
//! ticket in any source, [`Tickets`] reads a project's tickets from its source,
//! and [`GithubTickets`] adds the two reads only GitHub answers. A source holds
//! its repo, so no method names one. Nothing here talks to GitHub: the source
//! that does is a later slice.
//!
//! [`ForgeError`] is why a read failed, and `fake::FakeTickets` (under the `testing` feature) answers each
//! method from a script for the code that reads the port.
//!
//! # Quick start
//!
//! ```
//! use kelpie_engine::ticket::TicketId;
//!
//! let github: TicketId = "340".parse()?;
//! let local: TicketId = "k12".parse()?;
//! assert_eq!((github.to_string(), local.to_string()), ("340".into(), "k12".into()));
//! # Ok::<(), kelpie_engine::ticket::TicketIdError>(())
//! ```

mod error;
#[cfg(any(test, feature = "testing"))]
pub mod fake;
mod id;
mod issue;
mod port;
#[cfg(test)]
mod tests;

pub use error::ForgeError;
pub use id::{TicketId, TicketIdError};
pub use issue::{Issue, Listed};
pub use port::{GithubTickets, Tickets};
