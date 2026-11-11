//! Install: the one command that loads kelpie's AppArmor profile
//!
//! The `.deb` and `.rpm` ship the profile root-owned at [`PROFILE`]. Install
//! asks `pkexec` to copy it to `/etc/apparmor.d/kelpie` and load it with
//! `apparmor_parser -r`, one password prompt for both
//! (`docs/design/sandbox-setup.md`, Install). Nothing is copied from a folder
//! the person can write, so the prompt cannot be turned into root for another
//! file.
//!
//! `pkexec` runs its program with a fixed `PATH`, so the script and every
//! program in it are constants with absolute paths. No value from the
//! environment, a setting or the webview reaches the command line, and the
//! command carries no environment of its own.
//!
//! The command runs through the process tree (ADR 0001). [`outcome`] reads how
//! it ended, in `pkexec`'s own terms (`pkexec(1)`): 126 is a prompt the person
//! dismissed, and 127 with "No authentication agent found" is a desktop with no
//! polkit agent, which leaves the person the two commands to run in a terminal.

use std::io;
use std::process::Command;

use kelpie_engine::tree::ProcessTree;
use serde::Serialize;

/// Where the package ships the profile
///
/// `src-tauri/tauri.conf.json` maps it for the `.deb` and the `.rpm`.
pub const PROFILE: &str = "/usr/share/kelpie-app/kelpie.apparmor";

/// Where Install puts the profile, and the file `apparmor_parser` loads
const INSTALLED: &str = "/etc/apparmor.d/kelpie";

/// `pkexec`, by the path polkit installs it at
const PKEXEC: &str = "/usr/bin/pkexec";

/// The shell `pkexec` runs the script with
const SH: &str = "/bin/sh";

/// Copies the profile with the mode a profile has
const INSTALL: &str = "/usr/bin/install";

/// Loads a profile into the kernel, replacing the one of the same name
const APPARMOR_PARSER: &str = "/usr/sbin/apparmor_parser";

/// What `pkexec` prints when no polkit agent runs to ask for the password
const NO_AGENT: &str = "No authentication agent found";

/// What `pkexec` exits with when the person dismissed the prompt
const DISMISSED: i32 = 126;

/// What `pkexec` exits with when it could not ask, or the person was not authorized
const NOT_RUN: i32 = 127;

/// The script `pkexec` runs: copy the profile, then load it
fn script() -> String {
    format!("{INSTALL} -m 0644 {PROFILE} {INSTALLED} && {APPARMOR_PARSER} -r {INSTALLED}")
}

/// The one `pkexec` run that installs the profile, not yet started
///
/// `/bin/sh -c` with a script that is the same on every call.
#[must_use]
pub fn install_command() -> Command {
    let mut command = Command::new(PKEXEC);
    command.arg(SH).arg("-c").arg(script());
    command
}

/// The two commands a person runs in a terminal when no agent can ask for their password
#[must_use]
pub fn terminal_commands() -> Vec<String> {
    vec![
        format!("sudo install -m 0644 {PROFILE} {INSTALLED}"),
        format!("sudo apparmor_parser -r {INSTALLED}"),
    ]
}

/// How an Install ended, as the webview reads it
// wire format: the bindings hold this object
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallOutcome {
    /// The profile is copied and loaded
    Installed,

    /// The person dismissed the password prompt
    Dismissed,

    /// No polkit agent could ask for the password, or `pkexec` is not there
    NoAgent {
        /// The two commands to run in a terminal instead
        commands: Vec<String>,
    },

    /// `pkexec` or `apparmor_parser` failed
    Failed {
        /// The tool's own words
        message: String,
    },
}

impl InstallOutcome {
    fn no_agent() -> Self {
        Self::NoAgent {
            commands: terminal_commands(),
        }
    }
}

/// How an Install that exited with `code` and printed `stderr` ended
///
/// `code` is `None` for a command a signal ended.
#[must_use]
pub fn outcome(code: Option<i32>, stderr: &str) -> InstallOutcome {
    let said = stderr.trim();
    match code {
        Some(0) => InstallOutcome::Installed,
        Some(DISMISSED) => InstallOutcome::Dismissed,
        Some(NOT_RUN) if said.contains(NO_AGENT) => InstallOutcome::no_agent(),
        Some(code) => InstallOutcome::Failed {
            message: if said.is_empty() {
                format!("pkexec exited with code {code}")
            } else {
                said.to_owned()
            },
        },
        None => InstallOutcome::Failed {
            message: if said.is_empty() {
                "pkexec was ended by a signal".to_owned()
            } else {
                said.to_owned()
            },
        },
    }
}

/// Run `command` in `tree`, and say how it ended
///
/// A program that is not there is no agent to ask: a host without polkit has
/// no `pkexec` either.
fn run(tree: &ProcessTree, command: &mut Command) -> InstallOutcome {
    match tree.output(command) {
        Ok(output) => outcome(
            output.status.code(),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => InstallOutcome::no_agent(),
        Err(error) => InstallOutcome::Failed {
            message: format!("kelpie could not run pkexec: {error}"),
        },
    }
}

/// Install the profile through `pkexec`, in `tree`
///
/// Waits on the password prompt, so it runs off the main thread.
#[must_use]
pub fn install(tree: &ProcessTree) -> InstallOutcome {
    run(tree, &mut install_command())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::PathBuf;

    use kelpie_engine::run::RunId;

    use super::*;

    #[test]
    fn the_pkexec_command_runs_one_fixed_script_on_the_packaged_profile() {
        let command = install_command();

        assert_eq!(command.get_program(), "/usr/bin/pkexec");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [
                OsStr::new("/bin/sh"),
                OsStr::new("-c"),
                OsStr::new(
                    "/usr/bin/install -m 0644 /usr/share/kelpie-app/kelpie.apparmor \
                     /etc/apparmor.d/kelpie && \
                     /usr/sbin/apparmor_parser -r /etc/apparmor.d/kelpie"
                ),
            ]
        );
        assert_eq!(
            command.get_envs().count(),
            0,
            "nothing from the environment reaches pkexec"
        );
    }

    #[test]
    fn exit_126_is_a_dismissed_prompt() {
        assert_eq!(outcome(Some(126), ""), InstallOutcome::Dismissed);
    }

    #[test]
    fn no_agent_found_shows_the_two_terminal_commands() {
        let found = outcome(
            Some(127),
            "Error getting authority: No authentication agent found.\n",
        );

        assert_eq!(
            found,
            InstallOutcome::NoAgent {
                commands: vec![
                    "sudo install -m 0644 /usr/share/kelpie-app/kelpie.apparmor /etc/apparmor.d/kelpie"
                        .to_owned(),
                    "sudo apparmor_parser -r /etc/apparmor.d/kelpie".to_owned(),
                ]
            }
        );
        // The webview reads this shape
        assert_eq!(
            serde_json::to_value(&found).unwrap()["kind"],
            "no_agent",
            "the outcome is tagged by kind"
        );
    }

    #[test]
    fn a_missing_pkexec_shows_the_two_terminal_commands() {
        let dir = std::env::temp_dir().join(format!("kelpie-install-{}", RunId::new().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let tree = ProcessTree::new(RunId::new().unwrap(), dir.join("run.json")).unwrap();
        let missing = PathBuf::from("/kelpie-test/no/such/pkexec");

        let found = run(&tree, &mut Command::new(&missing));

        assert_eq!(found, InstallOutcome::no_agent());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_install_names_the_tools_own_words() {
        let words = "apparmor_parser: Unable to replace \"kelpie\".  Permission denied";

        assert_eq!(
            outcome(Some(1), &format!("{words}\n")),
            InstallOutcome::Failed {
                message: words.to_owned()
            }
        );
        // pkexec's own refusal is also exit 127, and is not a missing agent
        assert_eq!(
            outcome(
                Some(127),
                "Error executing command as another user: Not authorized"
            ),
            InstallOutcome::Failed {
                message: "Error executing command as another user: Not authorized".to_owned()
            }
        );
        // A tool that said nothing still says what happened
        assert_eq!(
            outcome(Some(2), ""),
            InstallOutcome::Failed {
                message: "pkexec exited with code 2".to_owned()
            }
        );
    }

    #[test]
    fn a_working_install_is_installed() {
        assert_eq!(outcome(Some(0), ""), InstallOutcome::Installed);
    }
}
