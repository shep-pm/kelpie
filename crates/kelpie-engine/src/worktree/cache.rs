//! What a ticket's build folder holds that cargo can rebuild, and when a turn
//! began, to tell what the turn touched
//!
//! A cargo output folder is a profile folder (`debug`, `release`, or either
//! under a target triple) that holds a `.fingerprint` folder. Its `incremental/`
//! and `deps/` are cache: a file there that no turn wrote to costs only a
//! rebuild when it goes.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What marks a folder as a cargo profile folder
const FINGERPRINT: &str = ".fingerprint";

/// The folders of a profile folder that are cache
const CACHE: [&str; 2] = ["incremental", "deps"];

/// How far below the build folder a profile folder is looked for
const DEPTH: usize = 4;

/// The file that records when the latest turn began, by its modified time
const BEGAN: &str = "kelpie-turn-began";

/// When a worker turn began, in the clock of the file system that holds its build folder
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnMark(PathBuf);

impl TurnMark {
    /// The mark of the ticket whose build folder is `build`
    pub fn in_build(build: &Path) -> Self {
        Self(build.join(BEGAN))
    }

    /// Records that a turn begins now
    ///
    /// # Errors
    ///
    /// The write's error, when the folder cannot take the file.
    pub fn begin(&self) -> io::Result<()> {
        fs::write(&self.0, "")
    }

    /// When the turn began, or `None` when none is recorded
    pub fn began(&self) -> Option<SystemTime> {
        fs::metadata(&self.0).ok()?.modified().ok()
    }
}

/// What a prune removed
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pruned {
    /// Files removed
    pub files: u64,
    /// The bytes those files held
    pub bytes: u64,
}

/// Removes the cache files in `build`'s cargo output that were last written before `since`
///
/// Leaves every other file in the folder, a link, and any file written at
/// or after `since`. A file that cannot be removed stays and is not counted.
pub fn prune(build: &Path, since: SystemTime) -> Pruned {
    let mut pruned = Pruned::default();
    for profile in profile_folders(build) {
        for cache in CACHE {
            prune_below(&profile.join(cache), since, &mut pruned);
        }
    }
    pruned
}

// The profile folders in `build`, which are not looked into further
fn profile_folders(build: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![(build.to_owned(), 0)];
    while let Some((dir, depth)) = pending.pop() {
        if is_dir(&dir.join(FINGERPRINT)) {
            found.push(dir);
            continue;
        }
        if depth == DEPTH {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if is_dir(&entry.path()) {
                pending.push((entry.path(), depth + 1));
            }
        }
    }
    found
}

// A real folder: one that a link points to is not looked into
fn is_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
}

fn prune_below(dir: &Path, since: SystemTime, pruned: &mut Pruned) {
    // A link named `deps` or `incremental` may lead anywhere.
    if !is_dir(dir) {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            prune_below(&path, since, pruned);
            // Only an empty folder goes.
            let _ = fs::remove_dir(&path);
        } else if meta.is_file()
            && meta.modified().is_ok_and(|written| written < since)
            && fs::remove_file(&path).is_ok()
        {
            pruned.files += 1;
            pruned.bytes += meta.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{File, FileTimes};
    use std::time::Duration;

    use super::*;
    use crate::worktree::tests::Dir;

    const DAY: Duration = Duration::from_secs(86_400);

    // Writes `path`, last written `age` before `now`
    fn write(path: &Path, now: SystemTime, age: Duration) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "cache").unwrap();
        let file = File::options().write(true).open(path).unwrap();
        file.set_times(FileTimes::new().set_modified(now - age))
            .unwrap();
    }

    #[test]
    fn stale_cache_goes_and_what_the_turn_wrote_or_cargo_does_not_own_stays() {
        let dir = Dir::new("cache");
        let build = dir.path();
        let began = SystemTime::now();
        let kept = [
            // Written by the turn
            ("debug/deps/fresh-1", Duration::ZERO),
            ("debug/incremental/crate/s-1/fresh", Duration::ZERO),
            // Not cache: a final artifact, a build script's output, kelpie's own
            ("debug/kelpie-app", DAY),
            ("debug/build/ring/out", DAY),
            ("debug/.fingerprint/dep/dep-lib", DAY),
            ("kelpie-turn-began", DAY),
            ("deferred-findings.md", DAY),
            // The worker's own, with names that look like cargo's
            ("clone/deps/mine", DAY),
            ("notes/incremental/mine", DAY),
        ];
        let gone = [
            "debug/deps/old-1",
            "debug/deps/old-2.rlib",
            "debug/incremental/crate/s-0/old",
            "x86_64-unknown-linux-gnu/release/deps/old-3",
        ];
        for (path, age) in kept {
            write(&build.join(path), began, age);
        }
        for path in gone {
            write(&build.join(path), began, DAY);
        }
        for profile in ["debug", "x86_64-unknown-linux-gnu/release"] {
            fs::create_dir_all(build.join(profile).join(FINGERPRINT)).unwrap();
        }

        let pruned = prune(build, began);

        assert_eq!(pruned.files, 4);
        assert_eq!(pruned.bytes, 4 * "cache".len() as u64);
        for path in gone {
            assert!(!build.join(path).exists(), "{path} stayed");
        }
        for (path, _) in kept {
            assert!(build.join(path).exists(), "{path} went");
        }
        assert!(
            !build.join("debug/incremental/crate/s-0").exists(),
            "the folder it emptied stayed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_cache_is_left_and_not_followed() {
        let dir = Dir::new("cache-link");
        let build = dir.path().join("build");
        let outside = dir.path().join("outside");
        let began = SystemTime::now();
        write(&outside.join("precious"), began, DAY);
        fs::create_dir_all(build.join("debug/.fingerprint")).unwrap();
        fs::create_dir_all(build.join("debug/deps")).unwrap();
        std::os::unix::fs::symlink(&outside, build.join("debug/deps/link")).unwrap();

        assert_eq!(prune(&build, began), Pruned::default());
        assert!(outside.join("precious").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_cache_folder_that_is_a_link_is_not_followed() {
        let dir = Dir::new("cache-folders");
        let build = dir.path().join("build");
        let outside = dir.path().join("outside");
        let began = SystemTime::now();
        write(&outside.join("precious"), began, DAY);
        fs::create_dir_all(build.join("debug/.fingerprint")).unwrap();
        for cache in CACHE {
            std::os::unix::fs::symlink(&outside, build.join("debug").join(cache)).unwrap();
        }

        assert_eq!(prune(&build, began), Pruned::default());
        assert!(outside.join("precious").exists());
    }

    #[test]
    fn a_folder_with_no_cargo_output_loses_nothing() {
        let dir = Dir::new("cache-none");
        let began = SystemTime::now();
        write(&dir.path().join("deps/old"), began, DAY);
        assert_eq!(prune(dir.path(), began), Pruned::default());
        assert_eq!(prune(&dir.path().join("gone"), began), Pruned::default());
    }

    #[test]
    fn the_mark_tells_when_the_last_turn_began() {
        let dir = Dir::new("cache-mark");
        let mark = TurnMark::in_build(dir.path());
        assert_eq!(mark.began(), None);
        mark.begin().unwrap();
        assert!(mark.began().is_some());
    }
}
