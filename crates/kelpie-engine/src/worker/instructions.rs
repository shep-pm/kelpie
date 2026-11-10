//! The instructions a worker's session is given

use std::path::PathBuf;

use super::credential::NoCredential;
use super::start::Project;

/// The worker's instructions, with the built-in agent file's placeholders filled in
///
/// `{kelpie}` is kelpie's own binary, which the worker runs `lease run` through,
/// `{base}` the branch its worktree is cut from, and `{pull_request}` the
/// hand-over file it writes a pull request's title and body in
/// (`docs/design/workers.md`, Pushing and the pull request). A project's own
/// agent file that names none of them is sent as it is.
///
/// A worker with no credential is told so outright, whatever its agent file
/// says about pushing: it has no way to get its work out, and saying so is the
/// most the maintainer can do about it. The reason is named with it, since a
/// missing App, a missing `gh` and a repo that is not GitHub are three different
/// things for the maintainer to go after, and the worker's final message is where
/// they hear which.
pub(super) fn instructions(
    project: &Project<'_>,
    ticket: &str,
    why: Option<NoCredential>,
) -> String {
    let kelpie = std::env::current_exe()
        .unwrap_or_else(|_| PathBuf::from("kelpie"))
        .display()
        .to_string();
    let hand_over = project
        .paths
        .pull_request_file(ticket)
        .display()
        .to_string();
    let body = project
        .binding
        .body
        .value
        .clone()
        .unwrap_or_default()
        .replace("{kelpie}", &kelpie)
        .replace("{base}", project.base)
        .replace("{pull_request}", &hand_over);
    match why {
        None => body,
        Some(why) => format!("{body}{}", no_credential(why)),
    }
}

/// What a worker with no credential is told, after every other instruction
fn no_credential(why: NoCredential) -> String {
    format!(
        "\nYou have no credential for this repo: {}, so `git push` is refused and every \
         instruction above that says to push means to commit. Commit your work, and name this \
         same reason in your final message, so the maintainer knows what to fix and can get the \
         branch out by hand.\n",
        why.describe()
    )
}
