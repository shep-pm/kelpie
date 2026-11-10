//! What is opened back inside what the policy closes

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::secrets::denied_folders;
use super::{Project, home, real, text};
use crate::session::Harness;

/// How far a session reads, from the project's `read-anywhere` setting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    /// The project, the folders beside it, the session's state folder and the
    /// harness's own install, with the rest of the home folder closed
    Project,

    /// The whole file system, less every harness's login, every secret and every
    /// other session's state
    Anywhere,
}

impl Reads {
    /// The reads the project's `read-anywhere` setting asks for
    ///
    /// A change lands at the session's next start, since crabpot widens no
    /// running sandbox.
    #[must_use]
    pub const fn from_setting(read_anywhere: bool) -> Self {
        if read_anywhere {
            Self::Anywhere
        } else {
            Self::Project
        }
    }
}

/// The folders opened back inside what the session's reads close
///
/// crabpot masks a denied path again when an allowed folder sits above it, but
/// an allow at or inside a denied folder opens it: a deny whose spelling ends in
/// `/**` is the folder itself, and only a deny strictly under an allow is landed
/// after it. So an allow is kept off every folder at or inside a login or a
/// secret, and off every one at or above a closed folder, which would open what
/// it hides: the logins under the home folder, or the state of every other
/// session. This is the second line: [`holds_home`] and [`in_denied`] refuse the
/// project folder itself.
///
/// An allow is kept off a closed folder by the folder it really names, since
/// crabpot matches real paths, and is named as the session was given it.
///
/// The one allow meant to sit inside a closed folder is an entry of the person's
/// global folder their harness linked into the session: it is named as the single
/// file or folder, never as the global folder whole, which holds their login and
/// every secret. crabpot lands every allow after every deny, so the name carves
/// only itself out of the deny around it.
pub(in crate::sandbox) fn reads(
    harness: &Harness,
    project: &Project<'_>,
    state_dir: &Path,
) -> Vec<String> {
    let closed = closed(project);
    let denied = denied_folders();
    let mut folders = vec![project.folder];
    // The folders beside the project: its sibling repos and worktrees. Named
    // only where the reads are closed to the project, and never when they are
    // the root, which would open everything.
    if project.reads == Reads::Project
        && let Some(beside) = project.folder.parent()
        && beside.parent().is_some()
    {
        folders.push(beside);
    }
    folders.push(state_dir);
    folders.extend(harness.installs.iter().map(PathBuf::as_path));
    let mut seen = HashSet::new();
    let mut allowed: Vec<String> = folders
        .into_iter()
        .filter(|folder| {
            let reached = real(folder);
            !closed.iter().any(|held| held.starts_with(&reached))
                && !denied.iter().any(|held| reached.starts_with(held))
                && seen.insert(reached)
        })
        .map(text)
        .collect();
    // One entry per entry of the person's global folder their harness linked into
    // the session, never the folder whole: that folder holds their login and every
    // secret, which stay denied. Each sits inside a folder the policy denies (goose
    // keeps its config beside its keys), so the #108 filter above would refuse it
    // and it is checked on its own: refused when it is itself a login or a secret,
    // or at or above a closed folder, which it would open.
    allowed.extend(
        harness
            .global_reads
            .iter()
            .filter(|entry| {
                let reached = real(entry);
                !closed.iter().any(|held| held.starts_with(&reached))
                    && !denied.contains(&reached)
                    && seen.insert(reached)
            })
            .map(|entry| text(entry)),
    );
    allowed
}

/// The folders an allow may not sit at or above, each of which holds something
/// the policy denies under it, as crabpot matches them
fn closed(project: &Project<'_>) -> Vec<PathBuf> {
    let mut closed = vec![real(project.sessions_root)];
    closed.extend(home().map(|home| real(&home)));
    closed
}
