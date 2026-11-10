//! Writing one key of the global `pm.md`, for a form that edits it
//!
//! The settings page edits the PM's harness, model and effort as the global
//! `pm` agent's own keys, so the value is kept once (`docs/design/settings.md`).
//! Only that one line changes: comments, other keys and the body stay as the
//! person wrote them. A file that would no longer be usable is put back.

use std::fs;
use std::io;
use std::path::Path;

use super::{Agents, AgentsError, FOLDER, PM, extends};
use crate::settings::ConfigDir;

/// A key of the PM's agent file that the settings page edits
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PmKey {
    /// `harness`
    Harness,

    /// `model`
    Model,

    /// `effort`
    Effort,
}

impl PmKey {
    /// The key as the frontmatter spells it
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Harness => "harness",
            Self::Model => "model",
            Self::Effort => "effort",
        }
    }
}

impl Agents {
    /// Set `key` of the global `pm.md`, or remove it when `value` is `None`
    ///
    /// A `pm.md` that is not there is made with `role: pm` and the one key, and
    /// is not made at all to clear a key. Anything else in the file stays where
    /// it was. The file is replaced by a rename, and read back: a value the
    /// agent cannot use puts the old file back and says why.
    ///
    /// # Errors
    ///
    /// [`AgentsError::Write`] when the file cannot be read or written, and
    /// [`AgentsError::File`] naming the key when the result cannot be used.
    pub fn set_pm_key(dir: &ConfigDir, key: PmKey, value: Option<&str>) -> Result<(), AgentsError> {
        let folder = dir.path().join(FOLDER);
        let file = folder.join(format!("{PM}.md"));
        let write_failed = |path: &Path, error: &io::Error| AgentsError::Write {
            path: path.to_path_buf(),
            kind: error.kind(),
        };
        let before = match fs::read_to_string(&file) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(write_failed(&file, &error)),
        };
        let Some(after) = edited(before.as_deref(), key, value) else {
            return Ok(());
        };
        fs::create_dir_all(&folder).map_err(|error| write_failed(&folder, &error))?;
        replace(&file, &after).map_err(|error| write_failed(&file, &error))?;
        let used = Agents::load(dir).and_then(|agents| agents.pm());
        if let Err(error) = used {
            let restored = match &before {
                Some(text) => replace(&file, text),
                None => fs::remove_file(&file),
            };
            if let Err(error) = restored {
                return Err(write_failed(&file, &error));
            }
            return Err(error);
        }
        Ok(())
    }
}

/// `file` replaced by `text`, through a temporary file beside it that no agent listing reads
pub(super) fn replace(file: &Path, text: &str) -> io::Result<()> {
    let temp = file.with_extension("md.tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, file).inspect_err(|_| {
        let _ = fs::remove_file(&temp);
    })
}

/// The file's text with `key` set or removed, `None` when nothing would change
fn edited(text: Option<&str>, key: PmKey, value: Option<&str>) -> Option<String> {
    let name = key.name();
    let Some(text) = text else {
        let value = value?;
        return Some(format!("---\nrole: {PM}\n{name}: {}\n---\n", scalar(value)));
    };
    let Some((front, body)) = extends::split(text) else {
        // Not a file this can edit: the read-back says what is wrong with it
        return Some(text.to_owned());
    };
    let at = |part: &str| part.as_ptr() as usize - text.as_ptr() as usize;
    let (front_start, front_end) = (at(front), at(front) + front.len());
    let closing = &text[front_end..at(body)];
    let eol = if front.contains("\r\n") { "\r\n" } else { "\n" };

    let mut lines = Vec::new();
    let mut found = false;
    let mut skipping = false;
    for line in front.split_inclusive('\n') {
        if skipping && line.starts_with([' ', '\t']) {
            continue;
        }
        skipping = false;
        if extends::key_of(line) == Some(name) {
            found = true;
            skipping = true;
            if let Some(value) = value {
                lines.push(format!("{name}: {}{eol}", scalar(value)));
            }
            continue;
        }
        lines.push(line.to_owned());
    }
    if !found {
        let value = value?;
        if let Some(last) = lines.last_mut()
            && !last.ends_with('\n')
        {
            last.push_str(eol);
        }
        lines.push(format!("{name}: {}{eol}", scalar(value)));
    }
    let mut out = String::with_capacity(text.len() + 32);
    out.push_str(&text[..front_start]);
    out.extend(lines.iter().map(String::as_str));
    out.push_str(closing);
    out.push_str(body);
    Some(out)
}

/// `value` as a YAML scalar: plain when that reads back as the same string, else quoted
pub(super) fn scalar(value: &str) -> String {
    let plain = value.starts_with(|c: char| c.is_ascii_alphabetic())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
        );
    if plain {
        value.to_owned()
    } else {
        // JSON's string is a YAML double-quoted scalar
        serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""))
    }
}
