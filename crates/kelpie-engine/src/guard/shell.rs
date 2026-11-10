//! The commands one Bash call runs, as the guard reads them
//!
//! Separators inside quotes are data, and a `$( )`, `( )` or backtick body
//! is a command list of its own. A heredoc body is data too: it is lifted
//! out before the split and kept with each command whose text names it, so
//! `gh pr create --body "$(cat <<'EOF' ... EOF)"` carries its body.
//! This reads commands the way a shell would for the guard's purposes. It
//! is not a shell, and it does not expand anything.

use std::ops::Range;

/// One command a Bash call runs
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Command {
    /// Its words with quotes removed, after any keyword or assignment in front
    pub words: Vec<String>,
    /// How each word was written, one for each of `words`
    pub quoting: Vec<Quoting>,
    /// The heredoc bodies its text reads
    pub heredocs: Vec<String>,
    /// Whether it defines a shell function, as `name() { ... }`
    pub defines_function: bool,
    /// Whether a `|` feeds it the output of the command before
    pub piped: bool,
    // The heredocs its text names, until `commands` has read their substitutions.
    marks: Vec<usize>,
}

/// How one word was written, which its text with quotes removed no longer says
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Quoting {
    /// For each character of the word, whether a quote or a backslash holds it
    /// literal. Double quotes do not hold a `$` or a backtick, which the shell
    /// still expands.
    pub quoted: Vec<bool>,
    /// Whether a `(` follows the word with no space between: the shell may read
    /// both as one extended glob, as `--comment@(|x)`
    pub glued: bool,
}

// A word as `words` scans it.
struct Word {
    text: String,
    quoting: Quoting,
}

// A heredoc's place in the lifted text: `<<`, then its index between these.
const MARK: char = '\u{0}';

// Keywords a command can follow: `if gh pr create`, `do git commit`. The
// programs a command can run behind, such as `env`, are the guard's `wrap`.
const LEADS: [&str; 10] = [
    "if", "while", "until", "do", "then", "else", "elif", "!", "{", "}",
];

/// A command line the guard will not read
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unreadable {
    /// Nested past [`MAX_DEPTH`] quotes and bodies
    TooDeep,
    /// Longer than [`MAX_LEN`] characters
    TooLong,
}

impl std::fmt::Display for Unreadable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooDeep => "it nests quotes and substitutions too deep to read",
            Self::TooLong => "it is too long to read",
        })
    }
}

// No command a worker writes nests this deep or runs this long.
const MAX_DEPTH: usize = 32;
const MAX_LEN: usize = 1 << 20;

/// Every command in `line`, a nested body's before the command around it
///
/// # Errors
///
/// [`Unreadable`] when `line` is too deep or too long to read.
pub(super) fn commands(line: &str) -> Result<Vec<Command>, Unreadable> {
    commands_at(line, 0)
}

// `commands`, `depth` heredoc bodies down: an unquoted body's substitutions are read too.
fn commands_at(line: &str, depth: usize) -> Result<Vec<Command>, Unreadable> {
    if line.len() > MAX_LEN {
        return Err(Unreadable::TooLong);
    }
    if depth > MAX_DEPTH {
        return Err(Unreadable::TooDeep);
    }
    let (text, bodies) = lift_heredocs(line);
    let chars: Vec<char> = text.chars().collect();
    let found: Vec<Command> = split(&chars)?
        .into_iter()
        .filter_map(|range| {
            let piped = piped(&chars, range.start);
            let segment = &chars[range];
            // A word keeps its `<<`, not the mark after it.
            let (words, quoting): (Vec<String>, Vec<Quoting>) = lead_stripped(words(segment))
                .into_iter()
                .map(unmarked)
                .map(|w| (w.text, w.quoting))
                .unzip();
            let text: String = segment.iter().collect();
            (!words.is_empty()).then(|| Command {
                defines_function: defines_function(&text),
                piped,
                heredocs: marks(segment)
                    .into_iter()
                    .filter_map(|i| bodies.get(i).map(|b| b.text.clone()))
                    .collect(),
                words,
                quoting,
                marks: marks(segment),
            })
        })
        .collect();
    // The shell expands an unquoted heredoc's `$( )` and backticks where the
    // command that reads it runs, so their commands come before it.
    let mut out = Vec::new();
    let mut read = vec![false; bodies.len()];
    for mut command in found {
        for i in std::mem::take(&mut command.marks) {
            let Some(body) = bodies.get(i).filter(|b| b.expands) else {
                continue;
            };
            if !std::mem::replace(&mut read[i], true) {
                for inner in substitutions(&body.text) {
                    out.extend(commands_at(&inner, depth + 1)?);
                }
            }
        }
        out.push(command);
    }
    Ok(out)
}

// Where a scan stands: inside quotes, or in a command list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    Single,
    Double,
    // A `$( )` or `( )` body starting at this index.
    Sub(usize),
    Backtick(usize),
}

// A heredoc's body, and whether the shell expands it: its delimiter had no quote.
struct Body {
    text: String,
    expands: bool,
}

// Whether a `#` at `at` starts a comment: at a word's start, which a separator or
// a parenthesis also makes.
fn comment_at(chars: &[char], at: usize) -> bool {
    chars[at] == '#'
        && at.checked_sub(1).is_none_or(|p| {
            chars[p].is_whitespace() || matches!(chars[p], ';' | '&' | '|' | '(' | ')')
        })
}

// Replaces each heredoc body with a mark naming it, and returns the bodies.
fn lift_heredocs(line: &str) -> (String, Vec<Body>) {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut bodies = Vec::new();
    let mut pending: Vec<(String, bool, bool)> = Vec::new();
    let mut stack: Vec<char> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let top = stack.last().copied();
        if top == Some('\'') {
            if ch == '\'' {
                stack.pop();
            }
        } else if ch == '\\' {
            out.push(ch);
            i += 1;
            if let Some(&next) = chars.get(i) {
                out.push(next);
            }
            i += 1;
            continue;
        } else if top == Some('"') && ch == '"' {
            stack.pop();
        } else if ch == '$' && chars.get(i + 1) == Some(&'(') {
            stack.push('(');
            out.push_str("$(");
            i += 2;
            continue;
        } else if top != Some('"') {
            match ch {
                // A comment's apostrophes are not quotes, as in `split`.
                '#' if comment_at(&chars, i) => {
                    let end = chars[i..]
                        .iter()
                        .position(|c| *c == '\n')
                        .map_or(chars.len(), |n| i + n);
                    out.extend(&chars[i..end]);
                    i = end;
                    continue;
                }
                '\'' | '"' => stack.push(ch),
                '(' => stack.push('('),
                ')' if top == Some('(') => {
                    stack.pop();
                }
                // A here-string's `<<<` is no heredoc, and its string is a word.
                '<' if chars[i..].starts_with(&['<', '<', '<']) => {
                    out.push_str("<<<");
                    i += 3;
                    continue;
                }
                '<' if chars.get(i + 1) == Some(&'<') => {
                    if let Some((delimiter, strip, quoted, end)) = delimiter(&chars, i + 2) {
                        out.push_str(&format!("<<{MARK}{}{MARK}", bodies.len() + pending.len()));
                        pending.push((delimiter, strip, !quoted));
                        i = end;
                        continue;
                    }
                }
                '\n' if !pending.is_empty() => {
                    out.push('\n');
                    i += 1;
                    for (delimiter, strip, expands) in pending.drain(..) {
                        let (text, next) = body(&chars, i, &delimiter, strip);
                        bodies.push(Body { text, expands });
                        i = next;
                    }
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
        i += 1;
    }
    // A body with no newline before the end is still read to the end.
    for (delimiter, strip, expands) in pending.drain(..) {
        let (text, _) = body(&chars, chars.len(), &delimiter, strip);
        bodies.push(Body { text, expands });
    }
    (out, bodies)
}

// The delimiter after `<<` at `from`, whether `<<-` strips tabs, whether any of it
// was quoted, and where it ends. The shell reads a whole word, up to a space or a
// metacharacter, with its quotes and backslashes removed.
fn delimiter(chars: &[char], from: usize) -> Option<(String, bool, bool, usize)> {
    let mut i = from;
    let strip = chars.get(i) == Some(&'-');
    if strip {
        i += 1;
    }
    while chars.get(i).is_some_and(|c| *c == ' ' || *c == '\t') {
        i += 1;
    }
    let mut word = String::new();
    let mut quoted = false;
    while let Some(&c) = chars.get(i) {
        match c {
            _ if c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '<' | '>') => break,
            '\'' | '"' => {
                let close = chars[i + 1..].iter().position(|q| *q == c)?;
                word.extend(&chars[i + 1..i + 1 + close]);
                quoted = true;
                i += close + 2;
            }
            '\\' => {
                // A backslash before a newline joins lines; before a character, quotes it.
                quoted = true;
                if let Some(&next) = chars.get(i + 1).filter(|n| **n != '\n') {
                    word.push(next);
                }
                i += 2;
            }
            _ => {
                word.push(c);
                i += 1;
            }
        }
    }
    (!word.is_empty()).then_some((word, strip, quoted, i.min(chars.len())))
}

// The bodies of the `$( )` and backtick substitutions in an unquoted heredoc's text.
// A backslash holds the `$` or backtick after it literal.
fn substitutions(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '$' if chars.get(i + 1) == Some(&'(') => {
                let start = i + 2;
                let mut depth = 1;
                let mut j = start;
                while j < chars.len() {
                    match chars[j] {
                        '\\' => j += 1,
                        '\'' => {
                            j = chars[j + 1..]
                                .iter()
                                .position(|c| *c == '\'')
                                .map_or(chars.len(), |n| j + 1 + n);
                        }
                        '(' => depth += 1,
                        ')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                // Its own nested substitutions are read when its body is.
                out.push(chars[start..j.min(chars.len())].iter().collect());
                i = j + 1;
            }
            '`' => {
                let start = i + 1;
                let mut j = start;
                while j < chars.len() && chars[j] != '`' {
                    j += if chars[j] == '\\' { 2 } else { 1 };
                }
                out.push(chars[start..j.min(chars.len())].iter().collect());
                i = j + 1;
            }
            _ => i += 1,
        }
    }
    out
}

// The body's lines from `from` up to the delimiter's line, and where the text resumes.
fn body(chars: &[char], from: usize, delimiter: &str, strip: bool) -> (String, usize) {
    let mut lines = Vec::new();
    let mut i = from;
    while i < chars.len() {
        let end = chars[i..]
            .iter()
            .position(|c| *c == '\n')
            .map_or(chars.len(), |n| i + n);
        let line: String = chars[i..end].iter().collect();
        let next = (end + 1).min(chars.len());
        let bare = if strip {
            line.trim_start_matches('\t')
        } else {
            &line
        };
        if bare.trim_end() == delimiter {
            return (lines.join("\n"), next);
        }
        lines.push(line);
        i = next;
    }
    (lines.join("\n"), chars.len())
}

// The command segments of `chars`, each with its full text, nested bodies too.
//
// One pass: each open `$( )`, `( )` or backtick body keeps the start of its
// own current segment, so no body is scanned twice.
fn split(chars: &[char]) -> Result<Vec<Range<usize>>, Unreadable> {
    let mut out = Vec::new();
    let mut stack: Vec<Context> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        if stack.len() > MAX_DEPTH {
            return Err(Unreadable::TooDeep);
        }
        let ch = chars[i];
        let top = stack.last().copied();
        if top == Some(Context::Single) {
            if ch == '\'' {
                stack.pop();
            }
            i += 1;
            continue;
        }
        if ch == '\\' {
            i += 2;
            continue;
        }
        if ch == '$' && chars.get(i + 1) == Some(&'(') {
            stack.push(Context::Sub(i + 2));
            i += 2;
            continue;
        }
        if ch == '`' {
            if let Some(Context::Backtick(from)) = top {
                stack.pop();
                out.push(from..i);
            } else {
                stack.push(Context::Backtick(i + 1));
            }
            i += 1;
            continue;
        }
        if top == Some(Context::Double) {
            if ch == '"' {
                stack.pop();
            }
            i += 1;
            continue;
        }
        match ch {
            // A comment runs to the newline, and its apostrophes are not quotes.
            '#' if comment_at(chars, i) => {
                i = chars[i..]
                    .iter()
                    .position(|c| *c == '\n')
                    .map_or(chars.len(), |n| i + n);
                continue;
            }
            '\'' => stack.push(Context::Single),
            '"' => stack.push(Context::Double),
            '(' => stack.push(Context::Sub(i + 1)),
            ')' => {
                if let Some(Context::Sub(from)) = top {
                    stack.pop();
                    out.push(from..i);
                }
            }
            // The `&` of `2>&1`, `<&0` or `&>out` is part of a redirect word, not a
            // separator: what follows it is still the command's.
            '&' if matches!(
                (i.checked_sub(1).map(|p| chars[p]), chars.get(i + 1)),
                (Some('>' | '<'), _) | (_, Some('>'))
            ) => {}
            '\n' | ';' | '&' | '|' => match stack.last_mut() {
                None => {
                    out.push(start..i);
                    start = i + 1;
                }
                Some(Context::Sub(from) | Context::Backtick(from)) => {
                    out.push(*from..i);
                    *from = i + 1;
                }
                Some(_) => {}
            },
            _ => {}
        }
        i += 1;
    }
    out.push(start..chars.len());
    Ok(out)
}

// Whether a segment starting at `start` follows a `|` or `|&`, not a `||`.
fn piped(chars: &[char], start: usize) -> bool {
    let before = |n: usize| start.checked_sub(n).and_then(|i| chars.get(i)).copied();
    match (before(1), before(2), before(3)) {
        (Some('|'), second, _) => second != Some('|'),
        (Some('&'), Some('|'), third) => third != Some('|'),
        _ => false,
    }
}

// The heredoc indexes a segment's text names.
fn marks(segment: &[char]) -> Vec<usize> {
    let text: String = segment.iter().collect();
    text.split(MARK)
        .skip(1)
        .step_by(2)
        .filter_map(|n| n.parse().ok())
        .collect()
}

// A segment's words, with quotes removed. A `$( )` stays whole inside its word.
fn words(segment: &[char]) -> Vec<Word> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quoted: Vec<bool> = Vec::new();
    let mut started = false;
    let mut i = 0;
    while i < segment.len() {
        let ch = segment[i];
        match ch {
            // A comment runs to the segment's end, a newline.
            '#' if !started => break,
            c if c.is_whitespace() || c == '(' || c == ')' => {
                if started {
                    out.push(Word {
                        text: std::mem::take(&mut word),
                        quoting: Quoting {
                            quoted: std::mem::take(&mut quoted),
                            glued: c == '(',
                        },
                    });
                    started = false;
                }
                i += 1;
                continue;
            }
            '\'' => {
                let end = segment[i + 1..]
                    .iter()
                    .position(|c| *c == '\'')
                    .map_or(segment.len(), |n| i + 1 + n);
                word.extend(&segment[i + 1..end]);
                quoted.resize(quoted.len() + (end - i - 1), true);
                i = end + 1;
            }
            '"' => {
                i += 1;
                while i < segment.len() && segment[i] != '"' {
                    let escaped = segment[i] == '\\' && i + 1 < segment.len();
                    if escaped {
                        i += 1;
                    }
                    word.push(segment[i]);
                    // Double quotes leave `$` and a backtick to the shell, but for
                    // the `$(` of a substitution, which is a command of its own.
                    let expands = segment[i] == '`'
                        || (segment[i] == '$' && segment.get(i + 1) != Some(&'('));
                    quoted.push(escaped || !expands);
                    i += 1;
                }
                i += 1;
            }
            '\\' => {
                if let Some(&next) = segment.get(i + 1) {
                    word.push(next);
                    quoted.push(true);
                }
                i += 2;
            }
            _ => {
                word.push(ch);
                quoted.push(false);
                i += 1;
            }
        }
        started = true;
    }
    if started {
        out.push(Word {
            text: word,
            quoting: Quoting {
                quoted,
                glued: false,
            },
        });
    }
    out
}

// A word without its `<<` marks' heredoc numbers, which `words` read as text.
fn unmarked(word: Word) -> Word {
    let mut text = String::new();
    let mut quoted = Vec::new();
    let mut in_mark = false;
    for (ch, held) in word.text.chars().zip(word.quoting.quoted) {
        if ch == MARK {
            in_mark = !in_mark;
        } else if !in_mark {
            text.push(ch);
            quoted.push(held);
        }
    }
    Word {
        text,
        quoting: Quoting {
            quoted,
            glued: word.quoting.glued,
        },
    }
}

// Drops the keywords in front of a command. Assignments stay, for `wrap`.
fn lead_stripped(mut words: Vec<Word>) -> Vec<Word> {
    let lead = words
        .iter()
        .take_while(|w| LEADS.contains(&w.text.as_str()))
        .count();
    words.drain(..lead);
    words
}

// `name() ...` or `name () ...`, after any keywords in front.
fn defines_function(text: &str) -> bool {
    let mut rest = text.trim_start();
    while let Some(word) = LEADS.iter().find_map(|k| {
        rest.strip_prefix(k)
            .filter(|r| r.starts_with(char::is_whitespace))
    }) {
        rest = word.trim_start();
    }
    let name_end = rest
        .find(|c: char| {
            !(c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':')
        })
        .unwrap_or(rest.len());
    name_end > 0 && rest[name_end..].trim_start().starts_with("()")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_of(line: &str) -> Vec<Vec<String>> {
        commands(line)
            .unwrap()
            .into_iter()
            .map(|c| c.words)
            .collect()
    }

    fn w(words: &[&str]) -> Vec<String> {
        words.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn separators_split_commands_and_quotes_keep_them_whole() {
        assert_eq!(
            words_of("cd sub && git commit -m 'a; b | c' || echo \"x && y\"\ngh pr view"),
            [
                w(&["cd", "sub"]),
                w(&["git", "commit", "-m", "a; b | c"]),
                w(&["echo", "x && y"]),
                w(&["gh", "pr", "view"]),
            ]
        );
    }

    #[test]
    fn a_command_behind_a_keyword_or_an_assignment_is_found() {
        for line in [
            "if gh pr create -t x; then echo; fi",
            "for i in 1; do gh pr create -t x; done",
            "x=$(gh pr create -t x)",
            "echo `gh pr create -t x`",
            "(gh pr create -t x)",
            "{ gh pr create -t x; }",
        ] {
            assert!(
                words_of(line).contains(&w(&["gh", "pr", "create", "-t", "x"])),
                "{line}: {:?}",
                words_of(line)
            );
        }
    }

    #[test]
    fn a_function_definition_is_seen() {
        let all = commands("f() { git push; }; f").unwrap();
        assert!(all[0].defines_function, "{all:?}");
        assert!(commands("if g () { x; }").unwrap()[0].defines_function);
        assert!(!commands("echo \"f()\"").unwrap()[0].defines_function);
        assert!(!commands("git commit -m 'a()'").unwrap()[0].defines_function);
    }

    #[test]
    fn a_command_inside_quotes_or_a_comment_runs_nothing() {
        for line in [
            "grep 'a|gh pr create' file",
            "echo \"; gh pr create\"",
            "echo hi # don't gh pr create",
        ] {
            assert!(
                !words_of(line)
                    .iter()
                    .any(|c| c.first().is_some_and(|w| w == "gh")),
                "{line}: {:?}",
                words_of(line)
            );
        }
    }

    #[test]
    fn a_heredoc_body_is_data_and_goes_with_the_command_that_reads_it() {
        let line =
            "git commit -F - <<'EOF'\nfix: x\n\ngh pr create; it runs nothing\nEOF\ngit push";
        let all = commands(line).unwrap();
        assert_eq!(all.len(), 2, "{all:?}");
        assert_eq!(all[0].words, w(&["git", "commit", "-F", "-", "<<"]));
        assert_eq!(all[0].heredocs, ["fix: x\n\ngh pr create; it runs nothing"]);
        assert_eq!(all[1].words, w(&["git", "push"]));
        assert!(all[1].heredocs.is_empty());
    }

    #[test]
    fn a_heredoc_inside_a_quoted_substitution_goes_with_the_outer_command() {
        let line = "gh pr create --title 'fix: x' --body \"$(cat <<'EOF'\nthe body\nEOF\n)\"";
        let all = commands(line).unwrap();
        let gh = all.iter().find(|c| c.words[0] == "gh").unwrap();
        assert_eq!(gh.words[..4], w(&["gh", "pr", "create", "--title"]));
        assert_eq!(gh.heredocs, ["the body"]);
    }

    #[test]
    fn a_heredoc_after_a_comment_with_an_apostrophe_is_still_lifted() {
        let all =
            commands("# don't push yet\ngit commit -F - <<EOF\nfix: x\nEOF\ngit push").unwrap();
        assert_eq!(all[0].heredocs, ["fix: x"], "{all:?}");
        assert_eq!(all[1].words, w(&["git", "push"]));
    }

    // Each level used to re-read every body inside it: 22 levels took 3.9s.
    #[test]
    fn deep_nesting_is_read_once_and_refused_past_the_cap() {
        let nest =
            |depth: usize| format!("{}git push{}", "echo $(".repeat(depth), ")".repeat(depth));
        let all = words_of(&nest(MAX_DEPTH - 1));
        assert!(all.contains(&w(&["git", "push"])), "{all:?}");
        assert_eq!(commands(&nest(MAX_DEPTH + 2)), Err(Unreadable::TooDeep));
        assert_eq!(commands(&"x".repeat(MAX_LEN + 1)), Err(Unreadable::TooLong));
    }

    #[test]
    fn a_list_inside_a_substitution_is_split() {
        let all = words_of("echo \"$(cd sub; git push && echo `gh pr view; true`)\"");
        for command in [&["cd", "sub"][..], &["git", "push"], &["gh", "pr", "view"]] {
            assert!(all.contains(&w(command)), "{command:?}: {all:?}");
        }
    }

    #[test]
    fn a_heredoc_marker_in_quotes_is_text() {
        let all = commands("echo \"a << b\"\ngit push").unwrap();
        assert_eq!(all.len(), 2, "{all:?}");
        assert!(all.iter().all(|c| c.heredocs.is_empty()));
    }

    #[test]
    fn a_tab_stripping_heredoc_ends_at_an_indented_delimiter() {
        let all = commands("cat <<-END\n\tone\n\tEND\ngit push").unwrap();
        assert_eq!(all[0].heredocs, ["\tone"]);
        assert_eq!(all[1].words, w(&["git", "push"]));
    }

    #[test]
    fn a_comment_after_a_separator_does_not_open_a_quote() {
        for line in [
            "true;# don't\ngit push\n# '",
            "true&# don't\ngit push\n# '",
            "true|# don't\ngit push\n# '",
            "(true)# don't\ngit push\n# '",
            "(# don't\ngit push\n)",
        ] {
            let all = words_of(line);
            assert!(all.contains(&w(&["git", "push"])), "{line:?}: {all:?}");
        }
        // A `#` inside a word is not a comment.
        let all = words_of("echo a#b; git push");
        assert!(all.contains(&w(&["git", "push"])), "{all:?}");
    }

    #[test]
    fn a_heredoc_ends_at_the_whole_word_the_shell_reads_as_its_delimiter() {
        let all = commands("cat <<E\"OF\"\nx\nEOF\ngit push\nE\n").unwrap();
        assert!(
            all.iter().any(|c| c.words == w(&["git", "push"])),
            "{all:?}"
        );
        assert_eq!(all[0].heredocs, ["x"]);
        // The delimiter runs to a metacharacter, so `EOF` alone does not end it.
        let all = commands("cat <<EOF.x\nEOF\ngit push\nEOF.x\ngit pull").unwrap();
        assert_eq!(all[0].heredocs, ["EOF\ngit push"], "{all:?}");
        assert!(all.iter().any(|c| c.words == w(&["git", "pull"])));
        assert!(!all.iter().any(|c| c.words == w(&["git", "push"])));
        let all = commands("cat <<END-1\nEND\nEND-1\ngit pull").unwrap();
        assert_eq!(all[0].heredocs, ["END"], "{all:?}");
        let all = commands("cat <<-'E'F\n\tx\n\tEF\ngit pull").unwrap();
        assert_eq!(all[0].heredocs, ["\tx"], "{all:?}");
    }

    #[test]
    fn a_substitution_in_an_unquoted_heredoc_runs_and_in_a_quoted_one_does_not() {
        for line in [
            "cat <<EOF\n$(git push --force)\nEOF",
            "cat <<EOF\nsay ${x:-$(git push --force)}\nEOF",
            "cat <<EOF\n`git push --force`\nEOF",
            "cat <<EOF\n$(echo $(git push --force))\nEOF",
            "cat <<EOF\n$(cat <<EOF2\n$(git push --force)\nEOF2\n)\nEOF",
            "cat <<-EOF\n\t$(git push --force)\n\tEOF",
        ] {
            let all = words_of(line);
            assert!(
                all.contains(&w(&["git", "push", "--force"])),
                "{line:?}: {all:?}"
            );
        }
        for line in [
            "cat <<'EOF'\n$(git push --force)\nEOF",
            "cat <<\"EOF\"\n$(git push --force)\nEOF",
            "cat <<\\EOF\n$(git push --force)\nEOF",
            "cat <<EOF\n\\$(git push --force)\nEOF",
        ] {
            let all = words_of(line);
            assert!(!all.iter().any(|c| c[0] == "git"), "{line:?}: {all:?}");
        }
    }

    #[test]
    fn a_substitution_in_a_heredoc_runs_where_its_command_does() {
        let all = words_of("cd sub\ncat <<EOF\n$(git push)\nEOF\ngit pull");
        let at = |c: &[&str]| all.iter().position(|x| *x == w(c)).unwrap();
        assert!(at(&["cd", "sub"]) < at(&["git", "push"]), "{all:?}");
        assert!(at(&["git", "push"]) < at(&["git", "pull"]), "{all:?}");
    }

    #[test]
    fn a_here_string_is_a_word_of_its_command_not_a_heredoc() {
        for line in [
            "sh <<< 'git push origin HEAD'",
            "sh <<< \"git push origin HEAD\"",
            "sh <<<x",
        ] {
            let found = commands(line).unwrap();
            assert_eq!(found.len(), 1, "{line}");
            assert!(found[0].heredocs.is_empty(), "{line}");
            assert_eq!(found[0].words.first().map(String::as_str), Some("sh"));
        }
        assert_eq!(words_of("sh <<< 'a b'"), [w(&["sh", "<<<", "a b"])]);
    }
}
