//! The instructions a worker's session is given

use std::path::PathBuf;

use super::credential::NoCredential;
use super::start::Project;

/// How a worker is told to get its pull request opened: by writing its title and
/// body where kelpie opens the draft from
///
/// `{file}` is that file, and `{ends}` the clause saying what its body ends with.
const HAND_OVER: &str = "When the work is done, or before a question with commits made, push \
                        the branch with `git push origin HEAD`. Do not open or edit the pull \
                        request yourself: your credential cannot, and kelpie does it. Write its \
                        title and body in `{file}`. The first line is the title, a conventional \
                        commit subject such as `fix(parser): keep the last line`, above any \
                        template's text: a template's heading is never the title, and kelpie \
                        opens nothing from a file whose first line is not a conventional commit \
                        subject. The rest is the body{ends}. Write the file before you end your \
                        turn, and write it again to change the title or body; kelpie opens the \
                        draft when a turn ends with the branch pushed, and sets the title and \
                        body to what the file says.";

/// What a worker's pull request body ends with, which merging it closes
const ENDS_WITH_RESOLVES: &str =
    ", which ends with the line `Resolves #<ticket>`, naming your ticket";

/// The worker's instructions, with the built-in agent file's placeholders filled in
///
/// `{kelpie}` is kelpie's own binary, which the worker runs `lease run` through,
/// `{base}` the branch its worktree is cut from, and `{pull_request}` how to hand
/// its pull request over: push, then write the title and body in the hand-over file
/// kelpie opens the draft from (`docs/design/workers.md`, Pushing and the pull
/// request). A project's own agent file that names none of the placeholders is sent
/// as it is.
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
    let file = project.paths.pull_request_file(ticket);
    let hand_over = HAND_OVER
        .replace("{file}", &file.display().to_string())
        .replace("{ends}", ENDS_WITH_RESOLVES);
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
