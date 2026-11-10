//! Removing a folder tree that holds read-only folders

use std::io;
use std::path::Path;

/// Removes the tree at `path`, making its folders writable first when the
/// removal is refused
///
/// Go writes its module cache with read-only folders, which cannot be emptied
/// until they are made writable.
///
/// # Errors
///
/// The [`io::Error`] of the removal, after the one retry when it was refused.
pub fn remove_tree(path: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            make_writable(path);
            std::fs::remove_dir_all(path)
        }
        other => other,
    }
}

/// Gives the owner read, write and execute on every folder under `root`, and
/// on `root`, so the tree can be unlinked
///
/// A link is never followed and never changed, so nothing outside `root` is
/// touched. What cannot be read or changed is skipped: the removal that
/// follows reports it.
fn make_writable(root: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(root) else {
        return;
    };
    if !meta.is_dir() {
        return;
    }
    let _ = std::fs::set_permissions(root, owner_writes(meta.permissions()));
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        make_writable(&entry.path());
    }
}

#[cfg(unix)]
fn owner_writes(mut permissions: std::fs::Permissions) -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;

    permissions.set_mode(permissions.mode() | 0o700);
    permissions
}

// Windows has no mode bits, only a read-only flag.
#[cfg(not(unix))]
fn owner_writes(mut permissions: std::fs::Permissions) -> std::fs::Permissions {
    // The lint warns that this makes a Unix file world-writable.
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    permissions
}
