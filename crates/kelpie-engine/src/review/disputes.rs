//! A worker's dispute of a finding it shows is wrong
//!
//! A fix turn that pushed nothing is settled when every finding it was sent that
//! is not a nit was deferred or disputed with a reason (`docs/design/review.md`,
//! Findings, items 3 and 4). The disputes are shown to the next reader, and
//! posted on the pull request when none follows. The rule for a finding raised
//! again is SK's (`runner/review/disputes.rs`): the pass's earlier disputes go to
//! [`answers`], and one disputed again is [`Answers::repeated`]. Everything here
//! is a pure function over the pass's kept disputes; the posting is the app's.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::findings_file::{Sent, deferred_with_reasons, disputed};
use super::{Finding, severity_tag};

/// A finding the worker disputed in a round, and the worker's reason
// wire format: changing this is a breaking change to the state file
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dispute {
    /// The round whose fix turn disputed it
    pub round: u32,
    /// The finding, as a reader raised it
    pub finding: Finding,
    /// Why the worker says it is wrong
    pub reason: String,
    /// Whether a reader was shown it, or it was posted
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shown: bool,
}

/// What a fix turn that pushed nothing said of the findings it was sent that
/// are not nits, each by its index in the findings sent
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Answers {
    /// Shown wrong, with the worker's reason
    disputed: Vec<(usize, String)>,
    /// Disputed with a reason, though a reader had raised it again after an
    /// earlier dispute
    repeated: Vec<usize>,
    /// Deferred as out of scope, and not disputed, with the worker's reason when
    /// it gave one
    deferred: Vec<(usize, Option<String>)>,
    /// Neither deferred nor disputed with a reason
    unanswered: Vec<usize>,
}

/// What a fix turn did with the findings it was sent, when it left none
/// unanswered and disputed nothing a second time
#[derive(Debug, PartialEq, Eq)]
pub struct Settled {
    /// How many it deferred, and not disputed
    pub deferred: usize,
    /// What it disputed with a reason, by the index of the finding sent
    pub disputed: Vec<(usize, String)>,
}

/// How the worker answered `sent` in its files
///
/// `already` is what the pass disputed before `round`: a finding in it that was
/// sent again cannot be disputed again, so the two readers that disagree end
/// with the maintainer. A finding both deferred and disputed counts as disputed.
/// A deferral or dispute an earlier round left in its file answers nothing in
/// this one.
///
/// # Errors
///
/// As [`deferred_with_reasons`] and [`disputed`].
pub fn answers(
    build: &Path,
    sent: Sent<'_>,
    round: u32,
    already: &[Dispute],
) -> Result<Answers, String> {
    let mut deferred = deferred_with_reasons(build)?;
    for before in sent.deferred_before {
        if let Some(at) = (deferred.iter()).position(|d| d.finding.is_same_as(before)) {
            deferred.remove(at);
        }
    }
    let disputed = disputed(build)?;
    let mut answers = Answers::default();
    for (at, f) in (sent.findings.iter().enumerate()).filter(|(_, f)| !f.is_nit()) {
        let reason = (disputed.iter())
            .filter(|d| d.finding.is_same_as(f))
            .find_map(|d| d.reason.clone());
        let again = already
            .iter()
            .any(|a| a.round < round && a.finding.is_same_as(f));
        match reason {
            Some(reason) if !again => answers.disputed.push((at, reason)),
            Some(_) => answers.repeated.push(at),
            None => match deferred.iter().find(|d| d.finding.is_same_as(f)) {
                Some(d) => answers.deferred.push((at, d.reason.clone())),
                None => answers.unanswered.push(at),
            },
        }
    }
    Ok(answers)
}

impl Answers {
    /// What the worker showed wrong, with its reason, by the finding's index
    #[must_use]
    pub fn disputed(&self) -> &[(usize, String)] {
        &self.disputed
    }

    /// Those disputed again after a reader raised them again, by the finding's
    /// index
    #[must_use]
    pub fn repeated_at(&self) -> &[usize] {
        &self.repeated
    }

    /// Those deferred, and not disputed, with the worker's reason when it gave
    /// one, by the finding's index
    #[must_use]
    pub fn deferred(&self) -> &[(usize, Option<String>)] {
        &self.deferred
    }

    /// Whether the worker disputed a finding again that a reader had raised again
    /// after an earlier dispute, which is a ruling: two readers disagree
    #[must_use]
    pub fn repeated(&self) -> bool {
        !self.repeated.is_empty()
    }

    /// What the answers come to when every finding that is not a nit is deferred
    /// or disputed, none of them disputed again, and one at least is disputed
    #[must_use]
    pub fn settled(self) -> Option<Settled> {
        let whole = self.unanswered.is_empty() && self.repeated.is_empty();
        (whole && !self.disputed.is_empty()).then_some(Settled {
            deferred: self.deferred.len(),
            disputed: self.disputed,
        })
    }
}

/// Keeps `raised` among `disputes`
///
/// A dispute kept already for the same round and the same finding is replaced, so
/// a step that runs again records nothing twice.
pub fn record(disputes: &mut Vec<Dispute>, raised: &[Dispute]) {
    for dispute in raised {
        disputes.retain(|d| !(d.round == dispute.round && d.finding.is_same_as(&dispute.finding)));
        disputes.push(dispute.clone());
    }
}

/// Marks every dispute as shown to a reader
pub fn mark_shown(disputes: &mut [Dispute]) {
    for dispute in disputes {
        dispute.shown = true;
    }
}

/// The comment that tells the maintainer of the disputes no reader was shown,
/// or nothing when a reader was shown each
///
/// A file of `build` is named by its name alone, so a reason that quotes the full
/// path the fix turn's prompt gave posts nothing of this machine's. Any other path
/// is left in, for [`crate::local_paths`] to refuse.
#[must_use]
pub fn comment(disputes: &[Dispute], build: &Path) -> Option<String> {
    let unshown: Vec<&Dispute> = disputes.iter().filter(|d| !d.shown).collect();
    if unshown.is_empty() {
        return None;
    }
    let mut text = String::from(
        "The worker disputed these findings, saying each is wrong, and no reviewer follows to read the reasons:\n",
    );
    for Dispute {
        finding, reason, ..
    } in unshown
    {
        text.push_str(&format!(
            "\n{}\n  The worker's reason: {reason}\n",
            finding_line(finding)
        ));
    }
    Some(by_file_name(text, build))
}

/// `1 finding`, or `<count> findings`
#[must_use]
pub fn findings_of(count: usize) -> String {
    match count {
        1 => "1 finding".to_owned(),
        n => format!("{n} findings"),
    }
}

// A file of `build` is named by its name alone. The longest spelling goes first,
// so the canonical form is not left half cut by the plain one.
fn by_file_name(body: String, build: &Path) -> String {
    let mut folders = vec![build.to_string_lossy().into_owned()];
    if let Ok(canonical) = std::fs::canonicalize(build) {
        folders.push(canonical.to_string_lossy().into_owned());
    }
    folders.sort_by_key(|folder| std::cmp::Reverse(folder.len()));
    folders.dedup();
    folders.iter().fold(body, |body, folder| {
        let body = body.replace(&format!("{folder}/"), "");
        body.replace(&format!("{folder}{}", std::path::MAIN_SEPARATOR), "")
    })
}

// One finding as a person reads it: `- HIGH \`src/lib.rs:3\`: what. why`
fn finding_line(finding: &Finding) -> String {
    let place = match finding.line {
        0 => finding.file.clone(),
        line => format!("{}:{line}", finding.file),
    };
    let (what, why) = (finding.what.trim(), finding.why.trim());
    let what = what.strip_suffix('.').unwrap_or(what);
    let severity = severity_tag(finding.severity);
    format!("- {severity} `{place}`: {what}. {why}")
}
