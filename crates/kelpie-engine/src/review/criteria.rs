//! What the issue asks for, which every review round checks the diff against
//!
//! An issue with an acceptance criteria heading gives that section. One
//! without gives its whole body, since a ticket's bullets are its criteria.
//! The words are SK's (`runner/review/criteria.rs`); reading the issue through
//! the ticket port is the pass's, not here.

/// The most of an issue a round's prompt carries, in bytes, so a long one
/// leaves a small local model room for the diff
const MOST: usize = 4000;

/// The acceptance criteria section of `body`, or the whole body, cut to 4000 bytes
#[must_use]
pub fn acceptance(body: &str) -> String {
    let level = |line: &str| {
        let hashes = line.bytes().take_while(|b| *b == b'#').count();
        (hashes > 0 && line[hashes..].starts_with(' ')).then_some(hashes)
    };
    let lines: Vec<&str> = body.lines().collect();
    let heading = lines.iter().position(|line| {
        level(line).is_some() && line.to_lowercase().contains("acceptance criteria")
    });
    let section = match heading {
        Some(at) => {
            let own = level(lines[at]).unwrap_or(1);
            let rest = &lines[at + 1..];
            let end = rest
                .iter()
                .position(|line| level(line).is_some_and(|l| l <= own))
                .unwrap_or(rest.len());
            rest[..end].join("\n")
        }
        None => body.to_owned(),
    };
    cut(section.trim(), MOST)
}

// Cut at a character boundary, saying so.
fn cut(text: &str, most: usize) -> String {
    if text.len() <= most {
        return text.to_owned();
    }
    let end = (0..=most)
        .rev()
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(0);
    format!("{}\n[the rest of the issue is left out]", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_acceptance_criteria_section_is_taken_up_to_the_next_heading() {
        let body = "## What\n\nA thing.\n\n## Acceptance criteria\n\n- [ ] one\n\
                    ### Detail\n- [ ] two\n\n## Blocked by\n\n- #3\n";
        assert_eq!(acceptance(body), "- [ ] one\n### Detail\n- [ ] two");
    }

    #[test]
    fn an_issue_without_the_heading_gives_its_whole_body() {
        let body = "The third of four tickets.\n\n- Tests: two clean rounds end it\n";
        assert_eq!(
            acceptance(body),
            "The third of four tickets.\n\n- Tests: two clean rounds end it"
        );
    }

    #[test]
    fn a_long_issue_is_cut_on_a_character_boundary() {
        let body = "é".repeat(MOST);
        let cut = acceptance(&body);
        assert!(cut.ends_with("[the rest of the issue is left out]"));
        assert!(cut.len() <= MOST + 40, "{}", cut.len());
    }
}
