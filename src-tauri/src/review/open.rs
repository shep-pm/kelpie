//! The reviewer's session on the harness its binding names
//!
//! A read's session is built by the app, as a worker's is (ADR 0007): on Claude Code that
//! is the `claude-acp` bridge kelpie installs, and on goose the person's own `goose acp`,
//! which speaks ACP itself. The engine's [`open_on`] then starts the reviewer's sandbox
//! around it and opens the session in the ticket's worktree, so a reviewer reaches no
//! folder but the worktree and the tests folder (`docs/design/review.md`, Reviewers).
//!
//! A read is one turn and never resumes, so there is no bridge version to swap to partway
//! through it: kelpie's own pin is the one a read runs on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use kelpie_engine::bridge;
use kelpie_engine::harness::{claude, goose};
use kelpie_engine::login::Store;
use kelpie_engine::review::{self, Open, Reviewer, SessionReviewer, open_on};
use kelpie_engine::session::Harness as SessionHarness;
use kelpie_engine::settings::Harness;
use kelpie_engine::tree::ProcessTree;
use kelpie_engine::worker::Paths;

use crate::pm;
use crate::workers::WorkerPlan;
use crate::workers::git_common_dir;

use super::Make;

/// What a project's reviewer sessions are built from, held across its reads
#[derive(Debug, Clone)]
pub struct ReviewOpen {
    /// kelpie's data folder, which holds the installed bridges
    data_dir: PathBuf,

    /// Where the person's harness logins are kept
    logins: Store,
}

impl ReviewOpen {
    /// What this project's reviewer sessions are built from
    #[must_use]
    pub fn new(data_dir: &Path, logins: &Store) -> Self {
        Self {
            data_dir: data_dir.to_owned(),
            logins: logins.clone(),
        }
    }
}

/// The opener of a read's session, on the harness the reviewer's binding names
///
/// Each session it builds is new: it resumes nothing, and it carries no kelpie tools and
/// no lease.
pub fn reviewer_open(on: &ReviewOpen, plan: &WorkerPlan, tree: &Arc<ProcessTree>) -> Box<Open> {
    let tree = Arc::clone(tree);
    let (on, plan) = (on.clone(), plan.clone());
    Box::new(move |opening| {
        let harness = harness(&on, &plan, opening)?;
        open_on(Arc::clone(&tree), harness)(opening)
    })
}

/// kelpie's own reviewer builder: a ticket's reads as fresh sessions of [`SessionReviewer`]
///
/// `folder` is the project's checkout, whose git dir each reviewer's worktree clone
/// borrows, and `paths` where each ticket's worktree and review state folders are.
#[must_use]
pub fn app_make(
    on: &ReviewOpen,
    paths: &Paths,
    folder: &Path,
    tree: &Arc<ProcessTree>,
) -> Arc<Make> {
    let (on, paths, folder, tree) = (
        on.clone(),
        paths.clone(),
        folder.to_owned(),
        Arc::clone(tree),
    );
    Arc::new(move |ticket, binding, plan| {
        let project = review::ReviewProject {
            folder: folder.clone(),
            git_common_dir: git_common_dir(&tree, &folder)?,
            network_allowlist: plan.network_allowlist.clone(),
            reads: plan.reads,
        };
        Ok(Arc::new(SessionReviewer::new(
            paths.clone(),
            ticket,
            binding.clone(),
            project,
            reviewer_open(&on, plan, &tree),
        )) as Arc<dyn Reviewer>)
    })
}

/// The harness program a read runs on, in the reviewer's own state folder
///
/// A binding that names no harness runs on kelpie's own default ([`pm::PM_DEFAULT`]), as a
/// worker's and the PM's do.
///
/// # Errors
///
/// The harness is not wired to reviewers, goose has no provider set, the bridge cannot be
/// installed, or the harness is not installed or has no login. The text says which.
fn harness(
    on: &ReviewOpen,
    plan: &WorkerPlan,
    opening: &review::Opening<'_>,
) -> Result<SessionHarness, String> {
    let named = opening
        .binding
        .harness
        .value
        .unwrap_or(pm::PM_DEFAULT.harness);
    std::fs::create_dir_all(opening.state_dir)
        .map_err(|error| format!("could not make {}: {error}", opening.state_dir.display()))?;
    match named {
        Harness::Goose => {
            let provider = pm::goose_provider(plan.provider.as_deref(), plan.host.as_deref())?;
            goose::harness(opening.state_dir, provider, plan.harness_global_folder)
                .map_err(pm::no_goose)
        }
        Harness::ClaudeCode => {
            let pinned = bridge::ensure(&on.data_dir, claude::pin()).map_err(|e| e.to_string())?;
            claude::harness(
                pinned,
                opening.state_dir,
                &on.logins,
                plan.harness_global_folder,
            )
            .map_err(|error| error.to_string())
        }
        // `Harness` grows with each harness's slice, and none but these two are built here
        // parity: harness.reviewer
        other => Err(format!(
            "{} is not wired to reviewers yet",
            pm::describe(other, None)
        )),
    }
}
