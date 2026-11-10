//! Guard: what kelpie reads of a command a worker runs, before it judges it
//!
//! The worker's policy (`docs/design/workers.md`, check 2) judges each command
//! a worker runs, so it first reads that command as a shell would. `shell`
//! splits a Bash call into the commands it runs, `wrap` takes off the
//! programs wrapped around one (`env`, `sudo`, `timeout`) and refuses what the
//! guard cannot read, and `script` reads a script the call runs from a file.
//! Kelpie judges in the ACP permission handler, not in a hook, so none of
//! these know a harness's tool JSON.
//!
//! Only the reader is here. The judge that uses it comes in its own issue.

// The judge is not here yet, so nothing outside these modules' tests calls them.
#![allow(dead_code, unused_imports)]

mod script;
mod shell;
mod wrap;

use std::path::{Path, PathBuf};

// How many characters of a word a refusal quotes back to the worker.
pub(crate) const SHOWN_CHARS: usize = 40;

// Where `cd` or `git -C` moves from `cwd`: `None` when it cannot be told.
fn moved(cwd: Option<&Path>, to: &str, home: Option<&Path>) -> Option<PathBuf> {
    // A folder the shell works out when it runs.
    if to.contains(['$', '`', '*', '?', '[']) {
        return None;
    }
    match to.strip_prefix('~') {
        Some("") => Some(home?.to_owned()),
        Some(rest) => Some(home?.join(rest.strip_prefix('/')?)),
        None if to == "-" => None,
        None if Path::new(to).is_absolute() => Some(PathBuf::from(to)),
        None => Some(cwd?.join(to)),
    }
}
