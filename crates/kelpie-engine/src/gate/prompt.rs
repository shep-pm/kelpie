//! What a worker is told when the gate sends something back to it
//!
//! A fix turn is a prompt over ACP to the worker's own session, so it reaches
//! every harness (`docs/design/gate-and-merge.md`, Ports). The words are SK's
//! (`runner/gate/mod.rs` `red_prompt`, `runner/gate/catch_up.rs`
//! `conflict_prompt`, `runner/merge/queue.rs` `queue_prompt`), with kelpie's
//! terms: the worker's credential cannot read CI, so the failing log travels in
//! the prompt, and the pull request's body is the hand-over file's.

use super::red::Logs;
use super::short;
use crate::ci::FailedCheck;

/// What a prompt asks of the body, as SK's `KEEP_BODY_TRUE` has it
pub(crate) const KEEP_BODY_TRUE: &str = "Update the pull request's title and body in your hand-over file \
    so they match the head you push: what changed, the files touched, the test evidence, and \
    the decisions you made.";

/// Where a fix starts and ends, which is `origin`'s head and never a rewrite
const PUSH: &str = "Fix it on this branch, starting from `origin`'s head, commit, and push with \
    `git push origin HEAD`. Do not rewrite the branch or force-push.";

/// A red run's fix turn: the failing checks' names and the end of each failed log
pub(super) fn red(head: &str, checks: &[FailedCheck], logs: &Logs, repeated: bool) -> String {
    let repeated = if repeated {
        " Its log named no file this branch changed, so kelpie reran this head's failed runs \
         and they failed again: the failure repeats, so do not call it a flake."
    } else {
        ""
    };
    let names: Vec<&str> = checks.iter().map(|check| check.name.as_str()).collect();
    format!(
        "CI failed on your pull request at {}: {}.{repeated} {} {PUSH} {KEEP_BODY_TRUE}",
        short(head),
        names.join(", "),
        read_help(logs),
    )
}

/// A conflict's fix turn, with a merge of the base already started in the worktree
pub(super) fn conflict(files: &[String]) -> String {
    format!(
        "Your pull request conflicts with its base in {}. A merge of the base is already in \
         progress in your worktree, with the base's other changes staged. Resolve those files, \
         run `git add` on each, and run the checks. Then finish the merge with \
         `git commit --no-edit` and push with `git push origin HEAD`. Do not start another \
         merge or rebase, and do not force-push.",
        files.join(", ")
    )
}

/// A merge queue removal's fix turn, which the project's fix attempts do not count
pub(super) fn queue(head: &str, reason: &str) -> String {
    format!(
        "The merge queue removed your pull request at {}: {reason}. The queue tests a pull \
         request on top of the base and the pull requests ahead of it, so this can come from \
         the combination and not from your branch alone. `gh run list --event merge_group` \
         lists the queue's runs, and `gh run view <run id> --log-failed` shows what failed. \
         {PUSH} {KEEP_BODY_TRUE}",
        short(head)
    )
}

// What a worker whose credential cannot read CI is told about the logs kelpie
// read for it
fn read_help(logs: &Logs) -> String {
    match logs {
        Logs::Read(logs) if logs.is_empty() => "Your credential cannot read CI, and kelpie \
            found no failed run to read for you: reproduce the failing checks with the repo's \
            own commands."
            .to_owned(),
        Logs::Read(logs) => {
            let text: Vec<String> = logs
                .iter()
                .map(|log| {
                    format!(
                        "--- {} (run {}, {})\n{}",
                        log.check, log.run, log.url, log.tail
                    )
                })
                .collect();
            format!(
                "Your credential cannot read CI, so kelpie read the end of each failed run's \
                 log for you:\n\n{}\n",
                text.join("\n")
            )
        }
        Logs::Refused(why) => format!(
            "Your credential cannot read CI, and kelpie could not read it for you ({why}): \
             reproduce the failing checks with the repo's own commands."
        ),
    }
}
