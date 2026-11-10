//! The login a worker holds on the project's repo
//!
//! The token is in a `gh` config folder of the worker's own, in its state
//! folder, in the `hosts.yml` that `gh` reads and that git's credential helper,
//! `gh auth git-credential`, answers from. No variable carries it: Codex drops
//! any whose name holds `KEY`, `SECRET` or `TOKEN` from the commands it runs,
//! and a variable shows in every process's environment. Git's own helpers are
//! emptied, which drops the keychain's, and github.com's is `gh`'s, so nothing
//! the maintainer set up is asked.

use std::path::{Path, PathBuf};

use crate::github::InstallationToken;
use crate::worker_folder;

// The account the token is kept under, and the user git sends with it. `gh`
// reads the older shape, a token directly under the host, only after asking
// GitHub whose it is to migrate it, which a worker's fence may not allow.
const USER: &str = "x-access-token";

// Its name in the state folder
const CONFIG_DIR: &str = "gh";

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

    // Measured with git and `gh` themselves: a helper the maintainer set up, the
    // keychain's among them, is not asked, and github.com is answered with the
    // token from the folder the worker's `gh` reads, from nothing in a variable.
    #[test]
    fn git_pushes_with_the_token_and_asks_none_of_the_helpers_the_maintainer_set_up() {
        let (dir, state, home) = (
            Dir::new("login-git"),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let tree = process_tree(dir.path());
        write(state.path(), &token("ghs_scoped")).unwrap();
        let global = home.path().join("gitconfig");
        fs::write(
            &global,
            "[credential]\n\thelper = !f() { echo username=maintainer; echo password=their-login; }; f\n",
        )
        .unwrap();
        let mut git = Command::new("git");
        git.args(["credential", "fill"])
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .envs(env(state.path()))
            .env("GIT_CONFIG_PARAMETERS", GIT_CONFIG);

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
    }
}
