//! When a pull request behind its project's base is caught up with it
//!
//! The gate reads `catch-up` (`docs/design/gate-and-merge.md`, The order,
//! catch-up): SK's `catch_up` key from its `[git]` table and its default
//! (`core/settings/git.rs`). It is the project's own, since whether a repo runs a
//! merge queue is the repo's. The rest of SK's `Git` table stays behind, because
//! kelpie reads a project's checkout and remote elsewhere.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// When a pull request behind the base is caught up with it
///
/// `Always` rebases it whenever it lacks the latest base, and the rebase is
/// tested again. `Conflicts` rebases it only when merging it into the base
/// conflicts: a branch that merges cleanly goes on unrebased, and what tests the
/// combination is the repo's merge queue or the base's own CI. Where a merge
/// queue tests the combination, rebasing every Review ticket on each base move
/// only reruns CI.
// wire format: a settings file holds these strings
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum CatchUp {
    /// Always
    #[default]
    Always,

    /// Only on conflict
    Conflicts,
}
