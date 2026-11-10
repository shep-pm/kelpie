//! The id of a ticket

use core::fmt;
use core::num::NonZeroU64;
use core::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The id of a ticket
///
/// A GitHub ticket's id is its issue number, written `340`. A local ticket's is
/// `k` and a counter, written `k12`. An id displays as it parses. Ordered by
/// source, GitHub's before the local ones, then by number, so the board's age
/// tie-breaker holds within a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TicketId {
    /// A GitHub issue, by its number
    Github(NonZeroU64),
    /// A local ticket, by its counter
    Local(NonZeroU64),
}

/// Why text is not a [`TicketId`]
// kelpie-engine is a library: an out-of-tree match would break on a new variant
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TicketIdError {
    /// It is not digits, or `k` and digits, or it has a sign, a space or a leading zero
    Malformed,
    /// Its number is zero, which no source numbers a ticket
    Zero,
    /// Its number is more than a `u64` holds
    TooBig,
}

impl fmt::Display for TicketIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "a ticket id is digits, or `k` and digits, with no leading zero",
            Self::Zero => "a ticket id is not zero",
            Self::TooBig => "a ticket id is more than a u64 holds",
        })
    }
}

impl core::error::Error for TicketIdError {}

impl TicketId {
    /// The id of a GitHub issue, or `None` for zero
    #[must_use]
    pub fn from_issue(number: u64) -> Option<Self> {
        NonZeroU64::new(number).map(Self::Github)
    }

    /// The issue number of a GitHub ticket, for code only GitHub's tickets
    /// reach, or `None` for a local one
    #[must_use]
    pub fn github(&self) -> Option<u64> {
        match self {
            Self::Github(number) => Some(number.get()),
            Self::Local(_) => None,
        }
    }
}

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Github(number) => write!(f, "{number}"),
            Self::Local(counter) => write!(f, "k{counter}"),
        }
    }
}

impl FromStr for TicketId {
    type Err = TicketIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (digits, local) = match text.strip_prefix('k') {
            Some(digits) => (digits, true),
            None => (text, false),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(TicketIdError::Malformed);
        }
        if digits.len() > 1 && digits.starts_with('0') {
            return Err(TicketIdError::Malformed);
        }
        let number: u64 = digits.parse().map_err(|_| TicketIdError::TooBig)?;
        let number = NonZeroU64::new(number).ok_or(TicketIdError::Zero)?;
        Ok(if local {
            Self::Local(number)
        } else {
            Self::Github(number)
        })
    }
}

// wire format: the text an id displays as; a bare number from before ids had
// sources still reads, as a GitHub issue's
impl Serialize for TicketId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TicketId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IdVisitor;

        impl Visitor<'_> for IdVisitor {
            type Value = TicketId;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an issue number, or a ticket id such as `340` or `k12`")
            }

            fn visit_u64<E: de::Error>(self, number: u64) -> Result<TicketId, E> {
                TicketId::from_issue(number)
                    .ok_or_else(|| E::invalid_value(de::Unexpected::Unsigned(number), &self))
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<TicketId, E> {
                text.parse()
                    .map_err(|_| E::invalid_value(de::Unexpected::Str(text), &self))
            }
        }

        deserializer.deserialize_any(IdVisitor)
    }
}
