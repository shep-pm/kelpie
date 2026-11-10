//! The login a worker holds on the project's repo
//!
//! The token is in a `gh` config folder of the worker's own, in its state
//! folder, in the `hosts.yml` that `gh` reads and that git's credential helper,
//! `gh auth git-credential`, answers from. No variable carries it: Codex drops
//! any whose name holds `KEY`, `SECRET` or `TOKEN` from the commands it runs,
//! and a variable shows in every process's environment. Git's own helpers are
//! emptied, which drops the keychain's, and github.com's is `gh`'s, so nothing
//! the maintainer set up is asked. That helper is a program on the machine, and
//! [`helper_on_path`] is how kelpie finds out before a turn that it is not there.

use std::path::{Path, PathBuf};

use crate::github::InstallationToken;
use crate::worker_folder;

// The account the token is kept under, and the user git sends with it. `gh`
// reads the older shape, a token directly under the host, only after asking
// GitHub whose it is to migrate it, which a worker's fence may not allow.
const USER: &str = "x-access-token";

// Its name in the state folder
const CONFIG_DIR: &str = "gh";

/// The program git runs to answer github.com, which [`GIT_CONFIG`] names
pub(crate) const HELPER: &str = "gh";

/// The `gh` a worker's git answers github.com with, from this process's `PATH`
///
/// The worker's harness carries this process's `PATH` to the git it runs, so this
/// is the `PATH` its credential helper is looked up on. `None` is where a worker
/// has no way to push with the token kelpie mints, which a worker's start says
/// before the turn rather than leaving the push to fail inside it.
#[must_use]
pub fn helper_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    helper_on(&path)
}

/// [`helper_on_path`], searching `path` instead of this process's `PATH`
#[must_use]
pub fn helper_on(path: &std::ffi::OsStr) -> Option<PathBuf> {
    crate::harness::on_path(HELPER, path)
}

/// The folder `gh` reads its config from, in the worker's state folder
#[must_use]
pub fn config_dir(state_dir: &Path) -> PathBuf {
    state_dir.join(CONFIG_DIR)
}

/// Writes `token` where the worker's `gh` finds it, in `state_dir`, readable by
/// the user alone and replacing the last turn's
///
/// # Errors
///
/// [`worker_folder::Error`] for a `gh` folder or file that is a link, which is
/// left along with whatever it points at.
pub fn write(state_dir: &Path, token: &InstallationToken) -> Result<(), worker_folder::Error> {
    let text = format!(
        "github.com:\n    git_protocol: https\n    user: {USER}\n    oauth_token: {token}\n    \
         users:\n        {USER}:\n            oauth_token: {token}\n",
        token = token.expose()
    );
    // The version says the config is already in the shape `gh` migrates to. Each
    // is for the user alone, whatever mode a file already there had, and renamed
    // into place so a `gh` reading it never finds half of it: with no token in
    // the file it would look in the keychain.
    let dir = Path::new(CONFIG_DIR);
    worker_folder::write_private(state_dir, &dir.join("config.yml"), "version: \"1\"\n")?;
    worker_folder::write_private(state_dir, &dir.join("hosts.yml"), &text)?;
    Ok(())
}

/// What a worker's `GIT_CONFIG_PARAMETERS` carries to point git at that login
///
/// `GIT_CONFIG_PARAMETERS` is what `git -c` sets for the commands it runs. Git
/// reads it after every config file, so the helper list is emptied after whatever
/// the maintainer's own files named, and github.com's is the only one left. None
/// of the names holds `KEY`, `SECRET` or `TOKEN`.
pub(crate) const GIT_CONFIG: &str =
    "'credential.helper'='' 'credential.https://github.com.helper'='!gh auth git-credential'";

// A worker with no token still gets its helpers emptied, so git asks nothing the
// maintainer set up, and github.com is answered by nothing.
pub(crate) const NO_HELPER: &str = "'credential.helper'=''";

/// The variables that point `gh` at the login in `state_dir`, and keep git from
/// prompting
pub(crate) fn env(state_dir: &Path) -> Vec<(String, String)> {
    let config = (
        "GH_CONFIG_DIR".to_owned(),
        config_dir(state_dir).to_string_lossy().into_owned(),
    );
    std::iter::once(config).chain(no_prompt()).collect()
}

/// The variables that keep `gh` and git from prompting, for a login that needs
/// no `gh` folder
pub(crate) fn no_prompt() -> [(String, String); 2] {
    let pair = |name: &str, value: &str| (name.to_owned(), value.to_owned());
    [
        pair("GH_PROMPT_DISABLED", "1"),
        pair("GIT_TERMINAL_PROMPT", "0"),
    ]
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::Command;

    use super::*;
    use crate::worktree::tests::{Dir, process_tree};

    fn token(text: &str) -> InstallationToken {
        InstallationToken::new(text.to_owned())
    }

    // The helper is a program on the machine, and a machine with no `gh` has no way
    // for a worker's git to answer github.com at all. Read off a `PATH` of the
    // test's own, so it says the same whatever the machine running the tests has.
    #[cfg(unix)]
    #[test]
    fn a_path_holds_a_helper_only_where_one_can_be_run() {
        let (bin, empty) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let gh = bin.path().join(HELPER);
        fs::write(&gh, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();

        let found = helper_on(bin.path().as_os_str());
        assert_eq!(found.as_deref(), Some(gh.as_path()), "{found:?}");

        fs::set_permissions(&gh, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            helper_on(bin.path().as_os_str()).is_none(),
            "a file no one may run is no helper"
        );

        assert!(
            helper_on(empty.path().as_os_str()).is_none(),
            "no `{HELPER}` on that PATH"
        );
    }

    // The helper `GIT_CONFIG` names as text and the one `helper_on` looks for are
    // the same program, or kelpie would check for a helper git never runs.
    #[test]
    fn git_names_the_helper_kelpie_looks_for() {
        assert!(
            GIT_CONFIG.contains(&format!("!{HELPER} auth git-credential")),
            "{GIT_CONFIG}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_token_is_written_for_the_user_alone_and_replaced_by_the_next_calls() {
        let state = tempfile::tempdir().unwrap();

        write(state.path(), &token("ghs_first")).unwrap();
        write(state.path(), &token("ghs_second")).unwrap();

        let file = config_dir(state.path()).join("hosts.yml");
        let text = fs::read_to_string(&file).unwrap();
        assert!(text.contains("oauth_token: ghs_second"), "{text}");
        assert!(!text.contains("ghs_first"), "{text}");
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    // A folder an earlier kelpie, or the worker, left with looser files.
    #[cfg(unix)]
    #[test]
    fn both_files_are_for_the_user_alone_even_where_older_ones_were_not() {
        let state = tempfile::tempdir().unwrap();
        let dir = config_dir(state.path());
        fs::create_dir_all(&dir).unwrap();
        for name in ["config.yml", "hosts.yml"] {
            fs::write(dir.join(name), "old\n").unwrap();
            fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o644)).unwrap();
        }

        write(state.path(), &token("ghs_scoped")).unwrap();

        for name in ["config.yml", "hosts.yml"] {
            let mode = fs::metadata(dir.join(name)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "{name}");
        }
    }

    // The worker can write its state folder, and kelpie writes as the maintainer.
    #[cfg(unix)]
    #[test]
    fn a_link_where_the_file_is_staged_leaves_what_it_points_to_alone() {
        let (state, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (dir, hosts) = (config_dir(state.path()), theirs.path().join("hosts.yml"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(&hosts, "their login\n").unwrap();
        symlink(&hosts, dir.join("hosts.yml.tmp")).unwrap();

        write(state.path(), &token("ghs_scoped")).unwrap();

        assert_eq!(fs::read_to_string(&hosts).unwrap(), "their login\n");
        let text = fs::read_to_string(dir.join("hosts.yml")).unwrap();
        assert!(text.contains("oauth_token: ghs_scoped"), "{text}");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_for_the_folder_is_refused_and_the_maintainers_own_login_is_not_replaced() {
        let (state, theirs) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        fs::write(theirs.path().join("hosts.yml"), "their login\n").unwrap();
        symlink(theirs.path(), config_dir(state.path())).unwrap();

        let refused = write(state.path(), &token("ghs_scoped"));

        assert!(refused.is_err());
        assert!(!refused.unwrap_err().to_string().contains("ghs_scoped"));
        let theirs: Vec<_> = fs::read_dir(theirs.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(theirs, ["hosts.yml"]);
        let kept = fs::read_to_string(config_dir(state.path()).join("hosts.yml")).unwrap();
        assert_eq!(kept, "their login\n");
    }

    /// The `gh` of the test's own, standing in for the one CI's container installs
    /// no copy of
    ///
    /// It answers from the folder `GH_CONFIG_DIR` names, as `gh` does, and logs what
    /// it was asked, the folder it read, and whether the token it handed git was
    /// sitting in a variable of the caller's.
    fn gh_fixture(bin: &Path, log: &Path) {
        let script = format!(
            "#!/bin/sh\n\
             hosts=\"$GH_CONFIG_DIR/hosts.yml\"\n\
             token=$(sed -n 's/^[[:space:]]*oauth_token: //p' \"$hosts\" | head -n 1)\n\
             user=$(sed -n 's/^[[:space:]]*user: //p' \"$hosts\" | head -n 1)\n\
             in_env=no\n\
             env | grep -qF \"$token\" && in_env=yes\n\
             printf 'gh %s config=%s in_env=%s\\n' \"$*\" \"$GH_CONFIG_DIR\" \"$in_env\" >> '{}'\n\
             printf 'username=%s\\npassword=%s\\n' \"$user\" \"$token\"\n",
            log.display()
        );
        write_helper(&bin.join("gh"), &script);
    }

    /// A credential helper of the maintainer's, of the shape `osxkeychain` and
    /// `manager` are: git finds `git-credential-their` on `PATH` by its name
    ///
    /// It logs that it was asked at all, which is what says whether git asked it.
    fn their_helper(bin: &Path, log: &Path) {
        let script = format!(
            "#!/bin/sh\n\
             printf 'their %s\\n' \"$*\" >> '{}'\n\
             printf 'username=%s\\npassword=%s\\n' maintainer their-login\n",
            log.display()
        );
        write_helper(&bin.join("git-credential-their"), &script);
    }

    fn write_helper(path: &Path, text: &str) {
        fs::write(path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// git's credential chain, measured with git itself and a `gh` of the test's own
    ///
    /// What is proven is kelpie's own half: the variables it sets, the file it
    /// writes, and that git asks that one helper and none of the maintainer's. That
    /// `gh` reads the folder `GH_CONFIG_DIR` names is `gh`'s own doing, and the
    /// ignored test below measures that with the maintainer's real `gh`.
    #[test]
    fn git_pushes_with_the_token_and_asks_none_of_the_helpers_the_maintainer_set_up() {
        let (dir, state, home, bin) = (
            Dir::new("login-git"),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let tree = process_tree(dir.path());
        write(state.path(), &token("ghs_scoped")).unwrap();
        let log = home.path().join("asked");
        gh_fixture(bin.path(), &log);
        their_helper(bin.path(), &log);

        let global = home.path().join("gitconfig");
        fs::write(&global, "[credential]\n\thelper = their\n").unwrap();

        let mut git = Command::new("git");
        git.args(["credential", "fill"])
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env(state.path()))
            .env("GIT_CONFIG_PARAMETERS", GIT_CONFIG)
            .env("PATH", on_path(bin.path()));

        let out = tree
            .output_with_input(&mut git, b"protocol=https\nhost=github.com\n\n")
            .unwrap();
        let answer = String::from_utf8_lossy(&out.stdout);

        assert!(
            answer.contains("password=ghs_scoped\n"),
            "{answer} {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!answer.contains("their-login"), "{answer}");
        assert_eq!(
            fs::read_to_string(&log).unwrap_or_default(),
            format!(
                "gh auth git-credential get config={} in_env=no\n",
                config_dir(state.path()).display()
            ),
            "every helper git asked, in the order it asked them"
        );
    }

    /// `bin` first on `PATH`, so git finds the test's helpers before any real ones
    fn on_path(bin: &Path) -> String {
        let path = std::env::var_os("PATH").unwrap_or_default();
        format!("{}:{}", bin.display(), path.to_string_lossy())
    }

    // The same chain with the maintainer's own `gh`, which CI's container installs
    // no copy of. Run it by hand where `gh` is installed:
    // `cargo test -p kelpie-engine --lib -- --ignored`
    #[ignore = "needs the maintainer's own `gh` on PATH, which CI does not install"]
    #[test]
    fn the_maintainers_own_gh_answers_from_the_folder_of_the_workers() {
        let (dir, state, home, bin) = (
            Dir::new("login-gh"),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let tree = process_tree(dir.path());
        write(state.path(), &token("ghs_scoped")).unwrap();
        let log = home.path().join("asked");
        their_helper(bin.path(), &log);

        let global = home.path().join("gitconfig");
        fs::write(&global, "[credential]\n\thelper = their\n").unwrap();

        let mut git = Command::new("git");
        git.args(["credential", "fill"])
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env(state.path()))
            .env("GIT_CONFIG_PARAMETERS", GIT_CONFIG)
            .env("PATH", on_path(bin.path()));

        let out = tree
            .output_with_input(&mut git, b"protocol=https\nhost=github.com\n\n")
            .unwrap();
        let answer = String::from_utf8_lossy(&out.stdout);

        // No `{answer}` here: where this fails, the answer is the maintainer's own
        // token, which no log should carry (IR-41).
        assert!(
            answer.contains("password=ghs_scoped\n"),
            "the maintainer's `gh` answered from somewhere other than the worker's folder"
        );
        assert!(!answer.contains("their-login"), "{answer}");
        assert_eq!(
            fs::read_to_string(&log).unwrap_or_default(),
            "",
            "git asked a helper of the maintainer's"
        );
    }
}
