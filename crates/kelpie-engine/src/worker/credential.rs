//! What a worker is given to push with, and why it sometimes has none

use std::path::Path;

use crate::github::{GithubApp, TokenError, origin_of};

use super::login;
use super::start::{Project, StartError};

/// What a worker is given to push with
pub(super) struct Credential {
    /// Why it has no token for its repo, `None` where it has one
    pub(super) why: Option<NoCredential>,

    /// The variables its harness and every command it runs carry
    pub(super) env: Vec<(String, String)>,
}

/// Why a worker got no token for its one repo, as its instructions tell the
/// maintainer
///
/// Each is a different thing to go after, and the worker's final message is where
/// the maintainer hears about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoCredential {
    /// kelpie keeps no App, or none installed on this repo
    NoApp,

    /// The `PATH` its git runs on holds no `gh` to answer github.com with
    NoHelper,

    /// Its remote is not GitHub, so kelpie names no repo to mint for
    NotGithub,
}

impl NoCredential {
    /// The cause, as the worker is told to pass it on
    pub(super) fn describe(self) -> &'static str {
        match self {
            Self::NoApp => "kelpie has no GitHub App installed on it",
            Self::NoHelper => {
                "no `gh` is installed where its git runs, and `gh` is what answers github.com for it"
            }
            Self::NotGithub => "its remote is not GitHub, so kelpie names no repo to mint for",
        }
    }
}

/// Mint the worker a token for its one repo, write it into the `gh` folder of its
/// own, and build the environment that points git at it
///
/// The token is minted fresh for this turn and kept nowhere else, so no refresh
/// timer runs and the turn has the token's whole hour to run in. Nothing in the
/// environment is the token: it names the folder and git's helper
/// ([`login`], `docs/design/workers.md`, Pushing and the pull request).
///
/// A repo kelpie's App does not cover, a repo that is not GitHub at all, and a
/// machine whose `PATH` holds no `gh` for git to answer github.com with each leave
/// the worker with no credential. It is told which one in its instructions, and git
/// is pointed at no helper at all, so nothing the maintainer logged into is asked.
/// Nothing is minted, and no `gh` folder is written, where the worker could not
/// have pushed with it; whatever an earlier turn left in that folder is taken out,
/// since it is still a live token until its hour is up. A token kelpie would not
/// hand a worker, or a GitHub that cannot be reached, fails the start instead: the
/// maintainer sees why, and the worker never runs as them.
///
/// # Errors
///
/// [`StartError::Login`]: the login folder could not be written, or the last
/// turn's could not be taken out.
/// [`StartError::Token`]: no token could be minted for the worker.
pub(super) fn credential(
    project: &Project<'_>,
    ticket: &str,
    state_dir: &Path,
) -> Result<Credential, StartError> {
    let build = project.paths.build_dir(ticket);
    let mut env = vec![("CARGO_TARGET_DIR".to_owned(), text(&build))];
    let (worker, why) = match (
        project.tokens,
        project.gh,
        origin_of(project.tree, project.folder),
    ) {
        (Some(tokens), Some(_), Some(repo)) => match tokens.worker(&repo) {
            Ok(worker) => (Some(worker), None),
            // No App for the owner, or one the repo does not cover: the worker
            // has no credential, and says so.
            Err(
                error @ (TokenError::Owner(_)
                | TokenError::NoApp(_)
                | TokenError::NotInstalled { .. }),
            ) => {
                eprintln!("kelpie: ticket {ticket} cannot push: {error}");
                (None, Some(NoCredential::NoApp))
            }
            Err(error) => return Err(StartError::Token(error)),
        },
        // Not a GitHub remote, so there is no repo to mint a token for at all
        (_, _, None) => (None, Some(NoCredential::NotGithub)),
        (Some(_), None, Some(_)) => {
            eprintln!(
                "kelpie: ticket {ticket} cannot push: no `{}` on the PATH its git runs on",
                login::HELPER
            );
            (None, Some(NoCredential::NoHelper))
        }
        // kelpie keeps no Apps
        (None, _, Some(_)) => (None, Some(NoCredential::NoApp)),
    };
    env.extend(login::env(state_dir));
    let helper = match worker {
        Some(worker) => {
            login::write(state_dir, &worker.token).map_err(StartError::Login)?;
            login::GIT_CONFIG
        }
        None => {
            login::clear(state_dir).map_err(StartError::Login)?;
            login::NO_HELPER
        }
    };
    env.push(("GIT_CONFIG_PARAMETERS".to_owned(), helper.to_owned()));
    Ok(Credential { why, env })
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
