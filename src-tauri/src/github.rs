//! The GitHub row on the settings page
//!
//! Kelpie holds no GitHub token: every read and every draft pull request goes
//! out as the login the system's `gh` holds (`docs/design/tickets.md`, GitHub).
//! [`github_status`] asks `gh` whose login that is and says only the login back,
//! so no command here returns a token or a login's value. No sign-in runs inside
//! kelpie: a maintainer without a login runs `gh auth login` in a terminal and
//! presses Refresh.

use kelpie_engine::github::gh::Gh;
use kelpie_engine::ticket::ForgeError;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// Whose `gh` login kelpie acts as
// wire format: changing this is a breaking change
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GithubStatus {
    /// `gh` holds a login, and `gh api user` names it
    SignedIn {
        /// The login `gh api user` printed
        login: String,
    },

    /// `gh` ran and said it has no login, carrying `gh`'s own line
    NotSignedIn {
        /// What `gh` said, which is where the row's advice comes from
        line: String,
    },

    /// There is no `gh` on this machine's `PATH` to ask
    NoGh,
}

/// What the row says, from `gh`'s answer to `gh api user`
///
/// # Errors
///
/// `gh` failed for a reason that says nothing about a login, with `gh`'s own
/// words, or `gh` could not be asked at all.
fn status_of(gh: Option<&Gh>) -> Result<GithubStatus, String> {
    let Some(gh) = gh else {
        return Ok(GithubStatus::NoGh);
    };
    match gh.viewer() {
        Ok(login) => Ok(GithubStatus::SignedIn { login }),
        // gh's own line, not ForgeError's text for it: that one is worded for a
        // ticket read, and this row is about who kelpie acts as
        Err(ForgeError::Failed(line)) if says_no_login(&line) => {
            Ok(GithubStatus::NotSignedIn { line })
        }
        // a network fault or a GitHub outage is no answer about a login
        Err(ForgeError::Failed(line)) => Err(line),
        Err(ForgeError::Spawn(why)) => Err(format!("kelpie could not start gh: {why}")),
        Err(ForgeError::Unreadable(what)) => {
            let said = what.trim();
            Err(format!("gh answered what kelpie cannot read: {said}"))
        }
        Err(error) => Err(error.to_string()),
    }
}

// Whether `gh`'s line is about its login: the advice to sign in, or a 401 for a
// token GitHub will not take
fn says_no_login(line: &str) -> bool {
    line.to_lowercase().contains("auth login") || line.contains("HTTP 401")
}

/// Whose `gh` login every read and draft pull request goes out as
///
/// The one `gh` call is `gh api user`, through the app's process tree (ADR 0001).
/// Nothing here signs anyone in or reads a token.
///
/// # Errors
///
/// `gh` failed for a reason that says nothing about a login, with `gh`'s own
/// words, or `gh` could not be asked at all.
#[tauri::command(async)]
#[specta::specta]
pub fn github_status(state: State<'_, AppState>) -> Result<GithubStatus, String> {
    status_of(Gh::on_path(&state.tree()).as_ref())
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::process::{ExitStatus, Output};

    use kelpie_engine::github::gh::{Gh, Runner};

    use super::{GithubStatus, status_of};

    #[cfg(unix)]
    fn status(code: i32) -> ExitStatus {
        std::os::unix::process::ExitStatusExt::from_raw(code << 8)
    }

    #[cfg(windows)]
    fn status(code: i32) -> ExitStatus {
        std::os::windows::process::ExitStatusExt::from_raw(code.cast_unsigned())
    }

    // The `gh` child, scripted: each call is answered with `answer`, so no test
    // needs `gh`, a login or the network.
    #[derive(Debug)]
    struct Scripted {
        answer: io::Result<Output>,
    }

    impl Scripted {
        fn answering(stdout: &str) -> Self {
            Self {
                answer: Ok(Output {
                    status: status(0),
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: Vec::new(),
                }),
            }
        }

        fn refusing(stderr: &str) -> Self {
            Self {
                answer: Ok(Output {
                    status: status(1),
                    stdout: Vec::new(),
                    stderr: stderr.as_bytes().to_vec(),
                }),
            }
        }
    }

    impl Runner for Scripted {
        fn run(&self, _args: &[String]) -> io::Result<Output> {
            match &self.answer {
                Ok(output) => Ok(Output {
                    status: output.status,
                    stdout: output.stdout.clone(),
                    stderr: output.stderr.clone(),
                }),
                Err(why) => Err(io::Error::new(why.kind(), why.to_string())),
            }
        }
    }

    #[test]
    fn a_login_gh_names_is_signed_in_as_that_login() {
        let gh = Gh::with_runner(Box::new(Scripted::answering("octocat\n")));

        assert_eq!(
            status_of(Some(&gh)).unwrap(),
            GithubStatus::SignedIn {
                login: "octocat".to_owned(),
            }
        );
    }

    #[test]
    fn a_gh_with_no_login_is_not_signed_in_and_carries_its_line() {
        let gh = Gh::with_runner(Box::new(Scripted::refusing(
            "gh: To get started with GitHub CLI, please run:  gh auth login\n",
        )));

        assert_eq!(
            status_of(Some(&gh)).unwrap(),
            GithubStatus::NotSignedIn {
                line: "gh: To get started with GitHub CLI, please run:  gh auth login".to_owned(),
            }
        );
    }

    #[test]
    fn a_missing_gh_is_no_gh() {
        assert_eq!(status_of(None), Ok(GithubStatus::NoGh));
    }

    #[test]
    fn a_failure_that_is_not_about_the_login_says_nothing_about_who_is_signed_in() {
        // a token GitHub will not take is still a login problem
        let bad = Gh::with_runner(Box::new(Scripted::refusing(
            "gh: Bad credentials (HTTP 401)\n",
        )));
        assert_eq!(
            status_of(Some(&bad)).unwrap(),
            GithubStatus::NotSignedIn {
                line: "gh: Bad credentials (HTTP 401)".to_owned(),
            }
        );

        // a GitHub outage and a host no one can reach answer nothing about a login
        for printed in [
            "gh: Server Error (HTTP 500)\n",
            "Get \"https://api.github.com/user\": dial tcp: lookup api.github.com: no such host\n",
        ] {
            let gh = Gh::with_runner(Box::new(Scripted::refusing(printed)));

            assert_eq!(status_of(Some(&gh)).unwrap_err(), printed.trim_end());
        }
    }

    #[test]
    fn the_answer_never_holds_a_token() {
        // What GH_DEBUG=api prints around the failure: the exchange, with the
        // login's token in its request headers, then gh's own line
        let gh = Gh::with_runner(Box::new(Scripted::refusing(
            "\
* Request at 2026-11-10 05:31:54.1 +0000 UTC
> GET /user HTTP/1.1
> Authorization: Bearer ghp_secrettokenvalue
< HTTP/2.0 401 Unauthorized
<
gh: To get started with GitHub CLI, please run:  gh auth login
",
        )));

        let seen = status_of(Some(&gh)).unwrap();

        assert_eq!(
            seen,
            GithubStatus::NotSignedIn {
                line: "gh: To get started with GitHub CLI, please run:  gh auth login".to_owned(),
            }
        );
        let json = serde_json::to_string(&seen).unwrap();
        assert!(!json.contains("ghp_"), "{json}");
        assert!(!json.contains("Authorization"), "{json}");
        assert_eq!(
            serde_json::to_string(&GithubStatus::SignedIn {
                login: "octocat".to_owned(),
            })
            .unwrap(),
            r#"{"kind":"signed_in","login":"octocat"}"#
        );
    }
}
