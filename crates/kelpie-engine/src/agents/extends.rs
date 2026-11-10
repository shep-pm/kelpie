//! `extends`: an agent file that builds on another
//!
//! A file with `extends: <name>` starts from that agent's frontmatter and
//! body. The keys it sets replace the parent's, and a file with no body takes
//! the parent's. A file with `body: append` puts its body after the parent's
//! in place of replacing it. The merge is on the text, before the typed parse,
//! so every check on a plain file applies to the merged one.
//!
//! Each key of the merged text also keeps the [`Origin`] it came from, so an
//! editor can say *from kelpie:pm* beside it.

use std::collections::BTreeMap;

use super::Origin;

/// The key whose origin is the body's, in [`Merged::origins`]
pub(super) const BODY: &str = "body";

/// A file's text with its parent merged in
///
/// The file's own frontmatter lines stay where they are, and the keys it
/// inherits follow them, so a refusal's line in the merged text is the
/// file's own line for a key the file sets.
#[derive(Debug)]
pub(super) struct Merged {
    /// The text to parse
    pub(super) text: String,
    /// The layer each key of `text` came from, and [`BODY`] for the body
    pub(super) origins: BTreeMap<String, Origin>,
    // The last line of the file's own frontmatter, and the parent it
    // inherits through. None for a file with no `extends`.
    inherits: Option<(usize, String)>,
}

impl Merged {
    /// `message`, a refusal of `text`, with a line past the file's own
    /// frontmatter replaced by the parent the key came through
    pub(super) fn place(&self, message: String) -> String {
        let Some((own_last, parent)) = &self.inherits else {
            return message;
        };
        let Some((head, at)) = message.rsplit_once(" at line ") else {
            return message;
        };
        let line = at
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .and_then(|n| n.parse::<usize>().ok());
        match line {
            Some(line) if line > *own_last => {
                format!("{head} inherited through `{parent}`")
            }
            _ => message,
        }
    }
}

/// What an `extends` name starts with to name one of kelpie's own agents, built
/// into the binary, in place of a file
pub(super) const SHIPPED: &str = "kelpie:";

/// The text of a parent: a file, which may extend another, or a shipped agent
#[derive(Debug, Clone, Copy)]
pub(super) enum ParentText<'a> {
    File(&'a str),
    Shipped { name: &'a str, text: &'a str },
}

/// What a merge asks of the loader
pub(super) struct Context<'a> {
    /// The text of the agent an `extends` name stands for, or the refusal for
    /// a name that stands for none
    pub(super) parent: &'a dyn Fn(&str) -> Result<ParentText<'a>, String>,
    /// The parent a file of this name builds on when it has no `extends`
    pub(super) implicit: &'a dyn Fn(&str) -> Option<String>,
}

/// `text` with its parent merged in, if it has an `extends` key or an implicit parent
///
/// `own` is the layer `text` is from, whose name a chain back to it is refused
/// as a cycle. Text with no parent comes back as it is.
pub(super) fn merged(own: Origin, text: &str, context: &Context<'_>) -> Result<Merged, String> {
    let mut chain = vec![own.to_string()];
    let resolved = resolve(text, &own, None, context, &mut chain)?;
    Ok(Merged {
        text: resolved.text,
        origins: resolved.origins,
        inherits: resolved.inherits,
    })
}

// The merged text, the layer of each of its keys, and for a file with a
// parent the last line of its own frontmatter and the parent it inherits
// through.
struct Resolved {
    text: String,
    origins: BTreeMap<String, Origin>,
    inherits: Option<(usize, String)>,
}

// The layer of every key `front` sets, and of the body if there is one
fn own_origins(entries: &[Entry], body: &str, layer: &Origin) -> BTreeMap<String, Origin> {
    let mut origins: BTreeMap<String, Origin> = entries
        .iter()
        .filter(|e| !matches!(e.key.as_str(), "extends" | "body"))
        .map(|e| (e.key.clone(), layer.clone()))
        .collect();
    if !body.trim().is_empty() {
        origins.insert(BODY.to_owned(), layer.clone());
    }
    origins
}

// `layer` is the file `text` is from, and `from` its name when it is a parent
// of the file being loaded, None for that file itself. A refusal of a
// parent's own text names it, since its line is no line of the file being
// loaded.
fn resolve<'a>(
    text: &str,
    layer: &Origin,
    from: Option<&str>,
    context: &Context<'a>,
    chain: &mut Vec<String>,
) -> Result<Resolved, String> {
    let Some((front, body)) = split(text) else {
        return Ok(Resolved {
            text: text.to_owned(),
            origins: BTreeMap::new(),
            inherits: None,
        });
    };
    let child = entries(front);
    let named = match child.iter().find(|e| e.key == "extends") {
        Some(extended) => Some(extended.value()),
        None => (context.implicit)(&layer.to_string()),
    };
    let Some(name) = named else {
        return Ok(Resolved {
            text: text.to_owned(),
            origins: own_origins(&child, body, layer),
            inherits: None,
        });
    };
    let refuse = |message: String| match from {
        Some(file) => format!("in `{file}.md`: {message}"),
        None => message,
    };
    let append = appends(&child).map_err(refuse)?;
    if chain.contains(&name) {
        let path: Vec<&str> = chain.iter().map(String::as_str).chain([&*name]).collect();
        return Err(refuse(format!(
            "`extends` runs in a circle: {}",
            path.join(" extends ")
        )));
    }
    let (parent_text, mut origins) = match (context.parent)(&name).map_err(refuse)? {
        ParentText::Shipped {
            name: shipped,
            text,
        } => {
            let extends = split(text)
                .is_some_and(|(front, _)| entries(front).iter().any(|e| e.key == "extends"));
            if extends {
                return Err(refuse(format!(
                    "`extends: {name}` names one of kelpie's own agents that itself extends \
                     another, which is not supported: write a file `{name}.md` to extend"
                )));
            }
            let origins = split(text).map_or_else(BTreeMap::new, |(front, body)| {
                own_origins(&entries(front), body, &Origin::Shipped(shipped.to_owned()))
            });
            (text.to_owned(), origins)
        }
        ParentText::File(text) => {
            chain.push(name.clone());
            let resolved = resolve(
                text,
                &Origin::File(name.clone()),
                Some(&name),
                context,
                chain,
            );
            chain.pop();
            let resolved = resolved?;
            (resolved.text, resolved.origins)
        }
    };
    let (parent_front, parent_body) = split(&parent_text).ok_or_else(|| {
        refuse(format!(
            "`extends: {name}` names a file with no frontmatter"
        ))
    })?;
    let mut keys = own_lines(front);
    // The file's frontmatter plus the opening `---`, as the lines it spans.
    let own_last = 1 + keys.lines().count();
    for entry in entries(parent_front) {
        if !child.iter().any(|c| c.key == entry.key) {
            keys.push_str(&entry.text);
        }
    }
    for (key, layer) in own_origins(&child, body, layer) {
        origins.insert(key, layer);
    }
    let body = match (body.trim().is_empty(), append) {
        (true, _) => parent_body.to_owned(),
        (false, false) => body.to_owned(),
        (false, true) if parent_body.trim().is_empty() => body.to_owned(),
        (false, true) => format!(
            "{}\n\n{}",
            parent_body.trim_end(),
            body.trim_start_matches(['\n', '\r'])
        ),
    };
    Ok(Resolved {
        text: format!("---\n{keys}---\n{body}"),
        origins,
        inherits: Some((own_last, name)),
    })
}

// Whether the file's `body` key says `append`. The key takes no other value,
// and the file sets it once.
fn appends(child: &[Entry]) -> Result<bool, String> {
    let mut bodies = child.iter().filter(|e| e.key == "body");
    let Some(body) = bodies.next() else {
        return Ok(false);
    };
    if let Some(again) = bodies.next() {
        return Err(format!(
            "`body` is set twice, which is not allowed, at line {}",
            again.line
        ));
    }
    match body.value().as_str() {
        "append" => Ok(true),
        other => Err(format!(
            "`body: {other}` is not a value `body` takes: only `append` is, at line {}",
            body.line
        )),
    }
}

// The file's frontmatter as written, with the lines of its `extends` and
// `body` keys left blank so every other line keeps its number.
fn own_lines(front: &str) -> String {
    let mut in_extends = false;
    let mut kept = String::new();
    for line in front.split_inclusive('\n') {
        if !line.starts_with([' ', '\t', '#', '-', '\r', '\n']) {
            in_extends = line
                .split_once(':')
                .is_some_and(|(k, _)| matches!(k.trim(), "extends" | "body"));
        }
        kept.push_str(if in_extends { "\n" } else { line });
    }
    if !kept.is_empty() && !kept.ends_with('\n') {
        kept.push('\n');
    }
    kept
}

// One top-level key of the frontmatter, with the lines under it: its
// continuation, comments and blank lines, up to the next key.
pub(super) struct Entry {
    pub(super) key: String,
    pub(super) text: String,
    // The line of the file the key is on, which counts the opening `---`.
    line: usize,
}

impl Entry {
    // The text after `key:`, without a trailing comment or quotes.
    pub(super) fn value(&self) -> String {
        let line = self.text.lines().next().unwrap_or("");
        let value = line.split_once(':').map_or("", |(_, v)| v);
        let value = value.trim();
        // A quoted value runs to its closing quote, so a ` #` inside is not a comment.
        for quote in ['"', '\''] {
            if let Some(rest) = value.strip_prefix(quote) {
                return rest.split(quote).next().unwrap_or("").to_owned();
            }
        }
        value.split(" #").next().unwrap_or("").trim().to_owned()
    }
}

// The frontmatter's keys in order. Lines before the first key are dropped.
pub(super) fn entries(front: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for (at, line) in front.split_inclusive('\n').enumerate() {
        let key = key_of(line).map(str::to_owned);
        match (key, entries.last_mut()) {
            (Some(key), _) => entries.push(Entry {
                key,
                text: line.to_owned(),
                line: at + 2,
            }),
            (None, Some(last)) => last.text.push_str(line),
            (None, None) => {}
        }
    }
    for entry in &mut entries {
        if !entry.text.ends_with('\n') {
            entry.text.push('\n');
        }
    }
    entries
}

// The top-level key a frontmatter line starts, if it starts one.
pub(super) fn key_of(line: &str) -> Option<&str> {
    (!line.starts_with([' ', '\t', '#', '-', '\r', '\n']))
        .then(|| line.split_once(':'))
        .flatten()
        .map(|(key, _)| key.trim())
}

/// The frontmatter between the opening and closing `---` lines, and the body after
pub(super) fn split(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\n', '\r']) == "---" {
            return Some((&rest[..at], &rest[at + line.len()..]));
        }
        at += line.len();
    }
    None
}

/// The line `key` starts in `front`, counting the opening `---` as line 1
pub(super) fn line_of(front: &str, key: &str) -> Option<usize> {
    entries(front)
        .into_iter()
        .find(|e| e.key == key)
        .map(|e| e.line)
}
