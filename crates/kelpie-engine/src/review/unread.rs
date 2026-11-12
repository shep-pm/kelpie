//! Which heads a review pass read or vouches for, and what the merge yes is told of one it does not
//!
//! A head pushed after the pass's last read, or a pass that ended short, reaches the merge
//! yes with a warning naming it (`docs/design/review.md`, The unread head). [`Heads`] is what
//! a ticket's pass keeps of the heads it has read, and [`unread_of`] says whether the head
//! the gate is at is one to warn of. A head a review fix turn pushed is vouched for: a
//! reviewer's findings are what it fixed. A nit-only fix's head counts as read, so the pass
//! goes on only while it still owes reads. Lifted from SK's `runner/gate/unread.rs` and
//! `core/work_item/heads.rs`; sending a head back through a pass is note 3's.

use serde::{Deserialize, Serialize};

use crate::gate::short;

/// The heads a ticket's review pass has read, and the ones it vouches for
// wire format: `review.json` holds this
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Heads {
    /// The heads a reviewer read, oldest first
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read: Vec<String>,

    /// The heads a review fix turn pushed
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vouched: Vec<String>,

    /// The heads a fix turn for nits alone pushed, which count as read
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nit_fixed: Vec<String>,
}

impl Heads {
    /// Records that a reviewer read `head`
    pub fn read(&mut self, head: &str) {
        push_once(&mut self.read, head);
    }

    /// Records that a review fix turn pushed `head`, which the pass vouches for
    ///
    /// `nits_only` is whether every finding that turn was sent was a nit: such a head
    /// counts as read too.
    pub fn vouch(&mut self, head: &str, nits_only: bool) {
        push_once(&mut self.vouched, head);
        if nits_only {
            push_once(&mut self.nit_fixed, head);
        }
    }

    /// Whether a reviewer read `head`, or a fix turn for nits alone pushed it
    #[must_use]
    pub fn reviewed(&self, head: &str) -> bool {
        self.read.iter().chain(&self.nit_fixed).any(|h| h == head)
    }

    /// Whether a fix turn for nits alone pushed `head`
    #[must_use]
    pub fn nit_fixed(&self, head: &str) -> bool {
        self.nit_fixed.iter().any(|h| h == head)
    }

    /// Whether a review fix turn pushed `head`
    #[must_use]
    pub fn vouches_for(&self, head: &str) -> bool {
        self.vouched.iter().any(|h| h == head)
    }
}

fn push_once(heads: &mut Vec<String>, head: &str) {
    if !heads.iter().any(|h| h == head) {
        heads.push(head.to_owned());
    }
}

/// A head the merge yes warns of
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// The head was pushed after the pass's last read, by something other than its fix turns
    Pushed {
        /// The head
        head: String,
        /// The head the last read was of, none when no reviewer read
        last_read: Option<String>,
    },
    /// The pass ended with fewer reads than it owed
    Short {
        /// The head
        head: String,
        /// Who was missed, none when no reviewer was listed to read at all
        why: Option<String>,
    },
}

impl Unread {
    /// The warning, as the merge yes shows it
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Pushed {
                head,
                last_read: Some(read),
            } => format!(
                "{} was pushed after the last review read, which was of {}, and no reviewer \
                 read it.",
                short(head),
                short(read)
            ),
            Self::Pushed {
                head,
                last_read: None,
            } => format!("No reviewer read {}.", short(head)),
            Self::Short {
                head,
                why: Some(why),
            } => {
                format!("The review ended short at {}: {why}.", short(head))
            }
            Self::Short { head, why: None } => format!("No reviewer read {}.", short(head)),
        }
    }
}

/// What the merge yes warns of at `head`, none when it is read or vouched for
///
/// `short` is how the pass ended when it ended short, as [`Lineup::pass_unread`] says it: a
/// reason, or none for a pass with no reviewer to read. A pass that ended short warns whatever
/// the head.
///
/// [`Lineup::pass_unread`]: super::Lineup::pass_unread
#[must_use]
pub fn unread_of(heads: &Heads, head: &str, short: Option<Option<String>>) -> Option<Unread> {
    if let Some(why) = short {
        return Some(Unread::Short {
            head: head.to_owned(),
            why,
        });
    }
    if heads.reviewed(head) || heads.vouches_for(head) {
        return None;
    }
    Some(Unread::Pushed {
        head: head.to_owned(),
        last_read: heads.read.last().cloned(),
    })
}
