//! The reviewers a project lists and how a reviewer's failing test is run
//!
//! The review pass reads these (`docs/design/review.md`, Settings). Each is the
//! project's own, since which agents read a repo's pull requests and how its
//! tests run are the repo's. A reviewer's harness, model and effort are
//! `Option<Harness>` and `Option<String>` in [`Settings`](super::Settings) and
//! [`Resolved`](super::Resolved), as the worker's are, so they need no type here.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// The reviewer agents a project uses, by name, in the order they read
///
/// An empty list is allowed and means no reviewer can run, which the merge reads
/// as a head marked unread. A name listed twice is refused: a reviewer reads a
/// head once.
// wire format: a settings file holds it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Reviewers(Vec<String>);

impl Reviewers {
    /// The agents' names, in the order they read
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.0
    }
}

impl JsonSchema for Reviewers {
    fn schema_name() -> Cow<'static, str> {
        "Reviewers".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "array",
            "items": { "type": "string", "minLength": 1 },
            "uniqueItems": true,
        })
    }
}

/// The reviewers a project lists when no layer says: kelpie's own
pub const DEFAULT_REVIEWERS: &[&str] = &["kelpie:reviewer"];

impl Reviewers {
    /// The built-in list
    pub(super) fn built_in() -> Self {
        Self(
            DEFAULT_REVIEWERS
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        )
    }
}

/// A command that runs one failing test a reviewer wrote, with the test file's path at `{test}` or
/// its file stem at `{name}`, such as `cargo test --test {name}`
///
/// Set, a reviewer is asked to leave a failing test for each finding it can, in a folder it may
/// write. Absent, no reviewer is asked for one.
// wire format: a settings file holds it
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TestCommand(String);

/// Why a command cannot be a project's test command, as the person writing it reads it
///
/// Read after the field's key, as `SettingsError::ValueRefused` writes it.
const PLACEHOLDER_RULE: &str =
    "must hold `{test}`, the test file's path, or `{name}`, its file stem";

impl TestCommand {
    /// The command as written
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// What `value` breaks, where it is not a command that names its test file
    ///
    /// JSON Schema's `pattern` could say it, but the schema check here does not know the keyword,
    /// so the rule is kelpie's own beside it (`Field::own_rule`).
    pub(super) fn rule(value: &str) -> Option<&'static str> {
        (!(value.contains("{test}") || value.contains("{name}"))).then_some(PLACEHOLDER_RULE)
    }
}

impl TryFrom<String> for TestCommand {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match Self::rule(&value) {
            None => Ok(Self(value)),
            Some(reason) => Err(reason),
        }
    }
}

impl<'de> Deserialize<'de> for TestCommand {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::try_from(text).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for TestCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl JsonSchema for TestCommand {
    fn schema_name() -> Cow<'static, str> {
        "TestCommand".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "description": "Holds `{test}` or `{name}`",
        })
    }
}
