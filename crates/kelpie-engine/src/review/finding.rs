//! A reviewer's findings: their shape, and reading them from a reviewer's reply

use serde::{Deserialize, Serialize};

/// How serious a review finding is
///
/// Every reviewer reports one of these three. Low is the nit level.
// wire format: changing this is a breaking change to the state file
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// A nit
    Low,
    /// Worth fixing before merge
    Medium,
    /// A real defect
    High,
}

/// One review finding, as a reviewer reports it
// wire format: changing this is a breaking change to the state file
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// How serious the reviewer thinks it is
    pub severity: Severity,
    /// The file it is in
    pub file: String,
    /// The line, or 0 when the reviewer names none
    pub line: u32,
    /// What is wrong
    pub what: String,
    /// Why it matters
    pub why: String,
    /// The failing test the reviewer wrote for it, as the path it should have
    /// in the repo
    ///
    /// A relative path with no `..` and no spaces; see [`is_test_path`]. Left out
    /// of the state file while none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
}

impl Finding {
    /// A finding with no test of the reviewer's
    pub fn new(
        severity: Severity,
        file: impl Into<String>,
        line: u32,
        what: impl Into<String>,
        why: impl Into<String>,
    ) -> Self {
        Self {
            severity,
            file: file.into(),
            line,
            what: what.into(),
            why: why.into(),
            test: None,
        }
    }

    /// The same finding with the failing test the reviewer wrote for it
    #[must_use]
    pub fn with_test(self, test: impl Into<String>) -> Self {
        Self {
            test: Some(test.into()),
            ..self
        }
    }

    /// Whether `other` is the same finding: in the same file, at the same
    /// line, saying the same thing, whoever else wrote it up and however
    /// differently it says why
    ///
    /// The key held findings are remembered by, readers' lists are merged by,
    /// and a deferred finding is matched to the one the worker was sent by.
    #[must_use]
    pub fn is_same_as(&self, other: &Self) -> bool {
        (&self.file, self.line, &self.what) == (&other.file, other.line, &other.what)
    }

    /// Whether it is a nit: low, the lowest severity
    #[must_use]
    pub fn is_nit(&self) -> bool {
        self.severity <= Severity::Low
    }
}

/// Parses a reviewer's reply in the `SEVERITY|file:line|what|why` format
///
/// A line that does not fit the shape is skipped rather than failing the
/// whole read: one that does not fit is safer to drop than to invent a
/// location for.
#[must_use]
pub fn parse_findings(text: &str) -> Vec<Finding> {
    text.lines().filter_map(parse_finding_line).collect()
}

/// What a reviewer writes, alone on its last line, when it found nothing
pub const CLEAN: &str = "CLEAN";

/// Reads a model's review reply: its findings, or none when its last
/// non-empty line is exactly `CLEAN`
///
/// A summary ahead of that line is fine. `CLEAN` anywhere else, or inside a
/// longer line such as "not CLEAN", is not a verdict.
///
/// # Errors
///
/// The reply, trimmed, when it holds no finding and does not end on `CLEAN`:
/// an empty reply or prose reviewed nothing, which is not clean.
pub fn read_review(text: &str) -> Result<Vec<Finding>, String> {
    let findings = parse_findings(text);
    let ends_clean = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| line.trim() == CLEAN);
    if findings.is_empty() && !ends_clean {
        return Err(text.trim().to_owned());
    }
    Ok(findings)
}

fn parse_finding_line(line: &str) -> Option<Finding> {
    let mut parts = line.splitn(5, '|');
    let severity = match parts.next()? {
        "LOW" => Severity::Low,
        "MEDIUM" => Severity::Medium,
        "HIGH" => Severity::High,
        _ => return None,
    };
    let location = parts.next()?;
    let what = parts.next()?;
    let mut why = parts.next()?.to_owned();
    // A fifth segment that reads as a path is taken as the test. Anything else
    // is a pipe in the why.
    let mut test = None;
    if let Some(fifth) = parts.next() {
        match is_test_path(fifth) {
            true => test = Some(fifth.to_owned()),
            false => why = format!("{why}|{fifth}"),
        }
    }
    let (file, line) = location.rsplit_once(':')?;
    let line = line.parse().ok()?;
    Some(Finding {
        test,
        ..Finding::new(severity, file, line, what, why)
    })
}

/// Whether `path` is one a reviewer may name as its finding's test: not empty,
/// relative, with no `..` step, no space and no pipe
#[must_use]
pub fn is_test_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains([' ', '|'])
        && path.split('/').all(|step| step != "..")
}
