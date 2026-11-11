//! The comment and the close kelpie makes on an issue, as the maintainer
//!
//! A pull request that merges without naming its issue leaves it open, so kelpie
//! closes it (`docs/design/gate-and-merge.md`, After the merge). Both calls go
//! out as the login `gh` holds, through the source's [`Hold`], as the draft pull
//! request does in `pulls.rs`. `-f` sends the text as it is, so a text that begins
//! with a dash is not read as a flag.

use super::hold::Hold;
use crate::github::ForgeSlug;
use crate::ticket::ForgeError;

/// Adds a comment with `body` to issue `number` of `repo`
pub(super) fn comment(
    hold: &Hold,
    repo: &ForgeSlug,
    number: u64,
    body: &str,
) -> Result<(), ForgeError> {
    let path = format!("repos/{repo}/issues/{number}/comments");
    let field = format!("body={body}");
    hold.call(&["api", "--method", "POST", &path, "-f", &field])
        .map(drop)
}

/// Closes issue `number` of `repo`
pub(super) fn close(hold: &Hold, repo: &ForgeSlug, number: u64) -> Result<(), ForgeError> {
    let path = format!("repos/{repo}/issues/{number}");
    hold.call(&["api", "--method", "PATCH", &path, "-f", "state=closed"])
        .map(drop)
}
