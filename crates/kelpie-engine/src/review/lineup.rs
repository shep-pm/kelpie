//! Who reads next in a pass, and when the pass has its reads
//!
//! A project lists its reviewers in the order they read (`docs/design/review.md`,
//! Reviewers). [`Lineup::of`] resolves each name through [`Agents::reviewer`], and
//! the next reviewer is the first [`Lineup::due`]: one the pass has not run whose
//! `paths` the diff touches. The pass has its reads once [`RULE_READS`] reviewers
//! that were not passed over have run, and a pass that ends short says who was
//! missed. Nothing here reads a diff or runs a reviewer: the caller passes the
//! changed files in, and the reviewers it passed over.

use super::Review;
use crate::agents::Agents;
use crate::settings::Reviewers;

/// How many reads kelpie's own rule gives a pass, until the PM sets the count at the pick
pub const RULE_READS: u32 = 2;

/// What starts the name of one of kelpie's own agents in the Reviewers setting
const SHIPPED: &str = "kelpie:";

/// A reviewer the project lists that can run
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The name as the project lists it, which a [`Review`] keeps in `ran` and `reviewer`
    pub name: String,

    /// The globs that limit it to diffs that touch one. Empty for every diff.
    pub paths: Vec<String>,

    /// Whether a fresh second session reads again after its first, shown what the first found
    pub second_look: bool,
}

/// A name the project lists that no reviewer file answers to
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    /// The name as the project lists it
    pub name: String,

    /// Why it cannot run, as [`Agents::reviewer`] refused it
    pub why: String,
}

/// The reviewers a project lists, in the order they read
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lineup {
    listed: Vec<Listed>,
    left_out: Vec<LeftOut>,
}

impl Lineup {
    /// The lineup of `reviewers`, each name resolved through [`Agents::reviewer`], in the
    /// setting's order
    ///
    /// A name with no reviewer file, or whose file cannot be used, is left out with its reason.
    /// `kelpie:reviewer` is the agent `reviewer`: the global `reviewer.md` over kelpie's own, or
    /// kelpie's alone. A name that stands for an agent an earlier name already lists is left out,
    /// so one agent never reads twice in a pass.
    #[must_use]
    pub fn of(reviewers: &Reviewers, agents: &Agents) -> Self {
        let mut lineup = Self {
            listed: Vec::new(),
            left_out: Vec::new(),
        };
        for name in reviewers.names() {
            let file = file_of(name);
            // `reviewer` and `kelpie:reviewer` are one agent, which reads once
            if let Some(same) = lineup.listed.iter().find(|r| file_of(&r.name) == file) {
                lineup.left_out.push(LeftOut {
                    name: name.clone(),
                    why: format!(
                        "it is the agent `{file}`, which `{}` already lists",
                        same.name
                    ),
                });
                continue;
            }
            match agents.reviewer(file) {
                Ok(agent) => lineup.listed.push(Listed {
                    name: name.clone(),
                    paths: agent.paths.value,
                    second_look: agent.second_look.value,
                }),
                Err(error) => lineup.left_out.push(LeftOut {
                    name: name.clone(),
                    why: error.to_string(),
                }),
            }
        }
        lineup
    }

    /// The reviewers that can run, in the order they read
    #[must_use]
    pub fn listed(&self) -> &[Listed] {
        &self.listed
    }

    /// The listed reviewer named `name`
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Listed> {
        self.listed.iter().find(|r| r.name == name)
    }

    /// The names that cannot run, with why
    #[must_use]
    pub fn left_out(&self) -> &[LeftOut] {
        &self.left_out
    }

    /// The reviewers `review`'s pass has not run whose paths touch `changed`, in the project's
    /// order: the first is the next to read
    #[must_use]
    pub fn due(&self, review: &Review, changed: &[String]) -> Vec<&Listed> {
        let due = |r: &&Listed| !review.ran.contains(&r.name) && touches(&r.paths, changed);
        self.listed.iter().filter(due).collect()
    }

    /// How many reviewers have read in `review`'s pass: those that ran, but a reviewer in
    /// `passed_over`, whose calls kept failing
    #[must_use]
    pub fn reads(&self, review: &Review, passed_over: &[String]) -> u32 {
        let reads = review.ran.iter().filter(|n| !passed_over.contains(n));
        u32::try_from(reads.count()).unwrap_or(u32::MAX)
    }

    /// Whether `review`'s pass has the `need` reads it calls for
    #[must_use]
    pub fn enough_read(&self, review: &Review, passed_over: &[String], need: u32) -> bool {
        self.reads(review, passed_over) >= need
    }

    /// Why `review`'s pass ends without the read it owes, when the merge is to be told
    ///
    /// `None` when the pass has its reads, or read less than `need` with no reviewer missed: every
    /// one that was left was kept off the diff by its own `paths`. `Some(None)` when no reviewer
    /// has read at all and none was missed either, as a project's list that names none leaves
    /// it. `Some(Some(reason))` names who was missed, and for a pass with some reads, how many it
    /// had.
    #[must_use]
    pub fn pass_unread(
        &self,
        review: &Review,
        passed_over: &[String],
        need: u32,
    ) -> Option<Option<String>> {
        if review.unread {
            return Some(self.missed(review, passed_over));
        }
        let reads = self.reads(review, passed_over);
        if reads >= need {
            return None;
        }
        let why = self.missed(review, passed_over)?;
        Some(Some(format!("{reads} of {need} reviewers read it: {why}")))
    }

    /// Each reviewer that ran and was passed over, and each name left out, as the reason it did
    /// not read, or none when nobody was missed
    fn missed(&self, review: &Review, passed_over: &[String]) -> Option<String> {
        // From `ran`, not the lineup: a reviewer the list has since dropped was still passed over
        let passed = (review.ran.iter())
            .filter(|name| passed_over.contains(name))
            .map(|name| format!("{name} was passed over after its calls kept failing"));
        let left = (self.left_out.iter()).map(|l| format!("{} is left out: {}", l.name, l.why));
        let missed: Vec<String> = passed.chain(left).collect();
        (!missed.is_empty()).then(|| missed.join(", "))
    }
}

/// The agent file a listed name stands for
fn file_of(name: &str) -> &str {
    name.strip_prefix(SHIPPED).unwrap_or(name)
}

/// Whether a change to the `changed` files is one a reviewer limited to `paths` reads: any change
/// when `paths` is empty
#[must_use]
pub fn touches(paths: &[String], changed: &[String]) -> bool {
    paths.is_empty()
        || changed
            .iter()
            .any(|file| paths.iter().any(|glob| matches(glob, file)))
}

/// Whether `path` matches `glob`: `*` and `?` stay inside one folder, `**` spans folders, and
/// `**/` may match none
pub(super) fn matches(glob: &str, path: &str) -> bool {
    matches_bytes(glob.as_bytes(), path.as_bytes())
}

fn matches_bytes(glob: &[u8], path: &[u8]) -> bool {
    match glob {
        [] => path.is_empty(),
        [b'*', b'*', b'/', rest @ ..] => {
            matches_bytes(rest, path)
                || path
                    .iter()
                    .enumerate()
                    .any(|(i, b)| *b == b'/' && matches_bytes(rest, &path[i + 1..]))
        }
        [b'*', b'*', rest @ ..] => (0..=path.len()).any(|i| matches_bytes(rest, &path[i..])),
        [b'*', rest @ ..] => {
            let folder = path.iter().position(|b| *b == b'/').unwrap_or(path.len());
            (0..=folder).any(|i| matches_bytes(rest, &path[i..]))
        }
        [b'?', rest @ ..] => path
            .first()
            .is_some_and(|b| *b != b'/' && matches_bytes(rest, &path[1..])),
        [c, rest @ ..] => path.first() == Some(c) && matches_bytes(rest, &path[1..]),
    }
}
