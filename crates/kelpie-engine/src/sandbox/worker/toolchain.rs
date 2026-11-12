//! The tool homes a worker's build reads back inside the closed home folder
//!
//! Cargo's, rustup's and bun's homes are read whole, and Go's `GOPATH` gives its
//! `bin` and module cache. Each comes from the tool's own variable, or its default
//! home where that is unset, and one not on this machine adds nothing. A variable
//! naming a folder relative to where the tool ran names nothing: each call runs its
//! tool from a different folder. None that holds the home is read back, or it would
//! undo the home's deny. A credential inside one stays denied by name, in
//! [`cargo_credentials`]; Go's and bun's homes hold none of their own.
//!
//! Lifted from SK's `dep_sources.rs`, less its source and docs folders, which a
//! reviewer reads and a worker's build does not need.

use std::ffi::OsString;
use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf};

/// One environment variable, or `None` when it is unset
pub(in crate::sandbox) trait Env {
    /// The variable `name`, or `None` when it is unset
    fn get(&self, name: &str) -> Option<String>;
}

/// The process's own environment, which is what each tool the call runs reads
#[derive(Debug)]
pub(in crate::sandbox) struct ProcessEnv;

impl Env for ProcessEnv {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }
}

/// The variables a test sets, as an [`Env`]
#[cfg(test)]
impl Env for std::collections::BTreeMap<&'static str, String> {
    fn get(&self, name: &str) -> Option<String> {
        Self::get(self, name).cloned()
    }
}

/// The tool homes themselves, where they are on this machine: what each tool
/// reads to run at all, its binaries, its config and its cache
pub(super) fn toolchain_homes(home: &Path, env: &dyn Env) -> Vec<PathBuf> {
    let gopaths = tool_homes(env, "GOPATH", home, "go");
    let found = [("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")]
        .into_iter()
        .flat_map(|(name, default)| tool_homes(env, name, home, default))
        .chain(gopaths.iter().map(|gopath| gopath.join("bin")))
        .chain(named(env, "GOMODCACHE"))
        .chain(gopaths.iter().map(|gopath| gopath.join("pkg/mod")))
        .chain(tool_homes(env, "BUN_INSTALL", home, ".bun"));
    let mut homes: Vec<PathBuf> = Vec::new();
    for dir in found {
        if dir.is_dir() && !home.starts_with(&dir) && !homes.contains(&dir) {
            homes.push(dir);
        }
    }
    homes
}

/// The credential files of each cargo home in `homes`, spelled for a deny rule
///
/// `SECRETS` names the default home's alone, so a grant of the whole home
/// `CARGO_HOME` moved elsewhere would open the registry token beside it. Each is
/// named whether or not the file is there: a call that runs `cargo login` writes
/// one while it runs, and the deny is made before it starts.
pub(super) fn cargo_credentials(homes: &[PathBuf]) -> Vec<String> {
    homes
        .iter()
        .flat_map(|home| {
            ["credentials", "credentials.toml"].map(|name| format!("{}/{}", home.display(), name))
        })
        .collect()
}

// The homes a tool keeps its cache in: the one its variable names, then the
// tool's `default` under `home`, each spelled once. Both are kept, since a
// moved cache may leave crates the default folder fetched earlier. A relative
// variable names nothing, as each call runs its tool from its own folder.
fn tool_homes(env: &dyn Env, name: &str, home: &Path, default: &str) -> Vec<PathBuf> {
    let default = normalize(&home.join(default));
    let mut homes = Vec::new();
    homes.extend(named(env, name).filter(|named| *named != default));
    homes.push(default);
    homes
}

// The absolute folder the variable `name` names, normalized, or `None`
fn named(env: &dyn Env, name: &str) -> Option<PathBuf> {
    env.get(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|named| named.is_absolute())
        .map(|named| normalize(&named))
}

// `path` with its `.` and `..` resolved and its trailing separator gone, by
// spelling alone, its Windows prefix kept ahead of the root. Not
// `fs::canonicalize`, which needs the folder to exist and follows links: a tool
// variable can name a folder that is not there.
fn normalize(path: &Path) -> PathBuf {
    let mut parts: Vec<OsString> = Vec::new();
    let mut prefix = None;
    let mut root = false;
    for part in path.components() {
        match part {
            Component::Prefix(found) => prefix = Some(found.as_os_str().to_os_string()),
            Component::RootDir => root = true,
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(last) if last != ".." => {
                    parts.pop();
                }
                _ => parts.push(OsString::from("..")),
            },
            Component::Normal(name) => parts.push(name.to_os_string()),
        }
    }
    let mut out = prefix.map(PathBuf::from).unwrap_or_default();
    if root {
        out.push(MAIN_SEPARATOR_STR);
    }
    for part in parts {
        out.push(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[cfg(windows)]
    #[test]
    fn normalize_keeps_a_windows_paths_prefix_ahead_of_its_root() {
        for (spelled, kept) in [
            (r"C:\Users\R\.cargo", r"C:\Users\R\.cargo"),
            (r"C:\Users\R\a\..\.cargo", r"C:\Users\R\.cargo"),
            (r"C:\Users\R\.cargo\", r"C:\Users\R\.cargo"),
            (r"C:\..\..\x", r"C:\..\..\x"),
            (r"\\?\C:\Users\R\.cargo", r"\\?\C:\Users\R\.cargo"),
            (r"\\?\C:\Users\R\a\..\.cargo", r"\\?\C:\Users\R\.cargo"),
        ] {
            assert_eq!(normalize(Path::new(spelled)), Path::new(kept), "{spelled}");
        }
    }

    fn set(name: &'static str, folder: &Path) -> (&'static str, String) {
        (name, folder.to_string_lossy().into_owned())
    }

    // A file at `rel` under `root`, with its folders made
    fn file_at(root: &Path, rel: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x\n").unwrap();
    }

    #[test]
    fn the_cargo_home_named_by_its_variable_is_read_with_the_default_one() {
        let dir = tempfile::tempdir().unwrap();
        let (home, named) = (dir.path(), dir.path().join("builds/7/cargo"));
        file_at(&named, "registry/src/serde-1.0.2/src/lib.rs");
        file_at(&home.join(".cargo"), "registry/src/rand-0.9.0/src/lib.rs");

        let homes = toolchain_homes(home, &BTreeMap::from([set("CARGO_HOME", &named)]));

        assert_eq!(homes, [named, home.join(".cargo")]);
    }

    #[test]
    fn a_relative_cargo_home_names_no_folder_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        file_at(&home.join(".cargo"), "registry/src/rand-0.9.0/src/lib.rs");
        file_at(
            &home.join("elsewhere"),
            "registry/src/serde-1.0.2/src/lib.rs",
        );

        // A relative tool variable means a folder under wherever the tool was
        // run, which is a different one for every call, so it names nothing.
        let homes = toolchain_homes(
            home,
            &BTreeMap::from([("CARGO_HOME", "elsewhere".to_owned())]),
        );

        assert_eq!(homes, [home.join(".cargo")]);
    }

    #[test]
    fn a_home_named_twice_over_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        file_at(&home.join(".cargo"), "registry/src/rand-0.9.0/src/lib.rs");
        // The named home is the default one, spelled through a folder that is
        // there, so both spellings reach the same crates.
        file_at(home, "tools/keep.txt");
        let named = home.join("tools/../.cargo/");

        let homes = toolchain_homes(home, &BTreeMap::from([set("CARGO_HOME", &named)]));

        assert_eq!(homes, [home.join(".cargo")]);
    }

    #[test]
    fn a_cargo_home_moved_by_its_variable_keeps_its_credentials_named() {
        let dir = tempfile::tempdir().unwrap();
        let (home, moved) = (dir.path(), dir.path().join("builds/cargo"));
        file_at(&moved, "registry/src/serde-1.0.2/src/lib.rs");
        file_at(&moved, "credentials.toml");

        let homes = toolchain_homes(home, &BTreeMap::from([set("CARGO_HOME", &moved)]));
        assert!(homes.contains(&moved), "{homes:?}");

        // The home whole is what a call reads back, so each credential file it
        // could hold is named beside it, the one not on disk included.
        let named = cargo_credentials(&homes);
        for name in ["credentials", "credentials.toml"] {
            let own = moved.join(name).to_string_lossy().into_owned();
            assert!(named.contains(&own), "{own}: {named:?}");
        }
    }

    #[test]
    fn rustups_home_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        file_at(&home.join(".rustup"), "toolchains/stable/bin/rustc");

        assert_eq!(
            toolchain_homes(home, &BTreeMap::new()),
            [home.join(".rustup")]
        );
    }

    #[test]
    fn gos_bin_and_module_cache_and_buns_home_are_read_back_and_no_other_folder() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        for rel in [
            "go/bin/golangci-lint",
            "go/pkg/mod/x/go.mod",
            "go/pkg/sumdb/sum.golang.org/latest",
            "go/src/other/main.go",
            ".bun/bin/bun",
            "other-repo/.env",
        ] {
            file_at(home, rel);
        }

        let homes = toolchain_homes(home, &BTreeMap::new());

        assert_eq!(
            homes,
            [
                home.join("go/bin"),
                home.join("go/pkg/mod"),
                home.join(".bun")
            ]
        );
    }

    #[test]
    fn a_gopath_and_a_module_cache_set_elsewhere_are_the_ones_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let (gopath, modcache) = (home.join("work/gopath"), home.join("caches/gomod"));
        file_at(&gopath, "bin/golangci-lint");
        file_at(&gopath, "pkg/mod/y/go.mod");
        file_at(&modcache, "z/go.mod");
        let env = BTreeMap::from([set("GOPATH", &gopath), set("GOMODCACHE", &modcache)]);

        assert_eq!(
            toolchain_homes(home, &env),
            [gopath.join("bin"), modcache, gopath.join("pkg/mod")]
        );
    }

    #[test]
    fn a_toolchain_not_on_the_machine_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        file_at(dir.path(), "other-repo/src/main.go");
        let env = BTreeMap::from([
            set("GOPATH", &dir.path().join("nowhere")),
            set("BUN_INSTALL", &dir.path().join("none")),
        ]);

        assert!(toolchain_homes(dir.path(), &env).is_empty());
    }

    // A tool home read back whole that holds the home would undo its deny.
    #[test]
    fn a_tool_variable_naming_the_home_or_above_it_reads_none_of_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("me");
        file_at(&home, "bin/tool");
        file_at(&home, "other-repo/.env");
        let env = BTreeMap::from([
            set("BUN_INSTALL", &home),
            set("CARGO_HOME", dir.path()),
            set("GOPATH", &home),
        ]);

        assert_eq!(toolchain_homes(&home, &env), [home.join("bin")]);
    }
}
