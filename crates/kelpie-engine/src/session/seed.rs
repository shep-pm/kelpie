//! What a new session starts from when the old one cannot carry on
//!
//! A session cannot move between harnesses, and a harness can lose a session
//! it was asked to resume. Either way kelpie starts a new one and [`seed`]s it:
//! the standing-decisions file, then what was last said, read from kelpie's own
//! log. No model writes it, so it costs no tokens to make and says nothing the
//! maintainer did not (`docs/design/pm-chat.md`, Switching harness or model).
//!
//! The exchanges kept are the newest, at most [`MAX_EXCHANGES`] and
//! [`MAX_CHARS`] characters of them. SK measured what a read costs on every
//! wake: 32 KiB of notes was about 8k tokens, and its standing notes stopped at
//! 4 KiB, about 1k tokens. The 8,000 characters here are about 2k tokens, paid
//! once on the first turn of a new session. The decisions file is cut at
//! [`MAX_DECISIONS`] bytes for the same reason.

use std::fs;
use std::path::Path;

use kelpie_protocol::{LogEntry, SessionEvent};

/// The most exchanges a seed holds
pub const MAX_EXCHANGES: usize = 10;

/// The most characters of exchanges a seed holds
pub const MAX_CHARS: usize = 8_000;

/// The most bytes of the decisions file a seed holds, SK's `STANDING_MAX`
pub const MAX_DECISIONS: usize = 4 * 1024;

/// One thing the maintainer said and what the PM answered
struct Exchange {
    user: String,
    reply: String,
}

impl Exchange {
    /// The lines of this exchange as the seed shows them
    fn render(&self) -> String {
        if self.reply.is_empty() {
            format!("Maintainer: {}\n", self.user)
        } else {
            format!("Maintainer: {}\nPM: {}\n", self.user, self.reply)
        }
    }
}

/// The exchanges in `log`, oldest first: a message and the reply text that follows it
///
/// A divider ends the exchange before it. Thoughts, tool steps and failures are
/// not part of what was said.
fn exchanges(log: &[LogEntry]) -> Vec<Exchange> {
    let mut found: Vec<Exchange> = Vec::new();
    let mut open = false;
    for entry in log {
        match entry {
            LogEntry::User { text } => {
                found.push(Exchange {
                    user: text.clone(),
                    reply: String::new(),
                });
                open = true;
            }
            LogEntry::Event(SessionEvent::Text { delta }) if open => {
                if let Some(last) = found.last_mut() {
                    last.reply.push_str(delta);
                }
            }
            LogEntry::Divider { .. } => open = false,
            _ => {}
        }
    }
    found
}

/// `text` cut to its last `room` characters, with a mark where it was cut
fn tail(text: &str, room: usize) -> String {
    let count = text.chars().count();
    if count <= room {
        return text.to_owned();
    }
    let kept: String = text.chars().skip(count - room).collect();
    format!("... {kept}")
}

/// What a new session starts from: the file at `decisions`, then the newest exchanges in `log`
///
/// Exchanges are taken newest first until [`MAX_EXCHANGES`] or [`MAX_CHARS`] is
/// reached, and shown oldest first. A file that is missing or empty leaves its
/// section out. An exchange too long to fit when it is the newest is cut to its
/// end, so the last thing said is never lost. Empty when there is nothing to say.
#[must_use]
pub fn seed(decisions: &Path, log: &[LogEntry]) -> String {
    let mut sections = Vec::new();
    if let Ok(file) = fs::read_to_string(decisions)
        && !file.trim().is_empty()
    {
        let mut cut = file.len().min(MAX_DECISIONS);
        while !file.is_char_boundary(cut) {
            cut -= 1;
        }
        let mut body = file[..cut].trim_end().to_owned();
        if cut < file.len() {
            body.push_str("\n... (the file goes on)");
        }
        sections.push(format!("## Standing decisions\n\n{body}\n"));
    }

    let mut kept: Vec<String> = Vec::new();
    let mut used = 0;
    for exchange in exchanges(log).iter().rev().take(MAX_EXCHANGES) {
        let mut text = exchange.render();
        let size = text.chars().count();
        if used + size > MAX_CHARS {
            if !kept.is_empty() {
                break;
            }
            text = tail(&text, MAX_CHARS);
        }
        used += text.chars().count();
        kept.push(text);
    }
    if !kept.is_empty() {
        kept.reverse();
        sections.push(format!("## Recent exchanges\n\n{}", kept.join("\n")));
    }
    sections.join("\n")
}

/// `message` with `seed` ahead of it, or `message` alone when the seed is empty
#[must_use]
pub fn ahead_of(seed: &str, message: &str) -> String {
    if seed.is_empty() {
        return message.to_owned();
    }
    format!(
        "This is a new session that replaces an earlier one. What follows is a record \
         of what was settled and said, kept by kelpie. Treat it as background, not as a \
         message to answer.\n\n{seed}\n## The maintainer's message\n\n{message}"
    )
}
