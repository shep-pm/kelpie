//! Opening a ticket's page on GitHub, through the OS's own broker
//!
//! [`open_link`] hands a URL to the OS and never starts a browser itself: a
//! browser started by kelpie would descend from kelpie and sit in its process
//! group, which ADR 0001 forbids. So the one process kelpie starts is a
//! broker that passes the URL on and exits, started through the process tree
//! like every other.
//!
//! - macOS: `open`, which gives the URL to LaunchServices.
//! - Linux: the XDG desktop portal's `org.freedesktop.portal.OpenURI.OpenURI`
//!   over the session bus, called with `gdbus call`, so no dependency comes in.
//! - Windows: nothing yet. Opening links waits on Windows being proven
//!   (`docs/design/ux.md`, Sandbox, item 6).
//!
//! Where the broker is missing or does not answer, the window is told to copy
//! the link, with a line saying why. Tauri's opener plugin is out because it
//! calls `open::that_detached`, which calls `setsid` on unix, and `xdg-open` is
//! out because it can run the browser as its own child, inside kelpie's group.
//!
//! Only the project's own issues and pull requests open, so a compromised
//! window cannot make kelpie open anything else.

use std::io;
use std::path::PathBuf;
use std::process::{Command, Output};

use kelpie_engine::github::ForgeSlug;
use kelpie_engine::tree::ProcessTree;
use serde::Serialize;
use specta::Type;
use tauri::State;

use crate::state::AppState;

/// What the window does after an open
// wire format: the Board panel switches on `kind`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkView {
    /// The OS took the link
    Opened,
    /// Nothing opened it: the window copies `url` and says `message`
    Copy {
        /// The link to copy
        url: String,
        /// Why it was not opened
        message: String,
    },
}

/// The operating system, as far as opening a link tells them apart
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// macOS
    MacOs,
    /// Linux
    Linux,
    /// Windows
    Windows,
}

impl Os {
    /// The system kelpie was built for
    #[must_use]
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// The broker programs, by name or path
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Programs {
    /// macOS's `open`
    pub open: PathBuf,
    /// The GLib bus tool that calls the portal
    pub gdbus: PathBuf,
}

impl Default for Programs {
    fn default() -> Self {
        Self {
            open: "open".into(),
            gdbus: "gdbus".into(),
        }
    }
}

/// Where a broker's command is started: the process tree, or a test's stand-in
pub trait Run {
    /// Start `command` and wait for it to exit
    ///
    /// # Errors
    ///
    /// The command could not be started.
    fn run(&self, command: &mut Command) -> io::Result<Output>;
}

impl Run for ProcessTree {
    fn run(&self, command: &mut Command) -> io::Result<Output> {
        self.output(command)
    }
}

/// Whether `url` is an issue or a pull request of `repo` on github.com
///
/// The whole URL is matched, with no query, fragment or other path, so what
/// reaches a broker is made of letters, digits and `-_.:/` alone.
///
/// # Errors
///
/// The URL is anything else: the text says what opens.
pub fn check(url: &str, repo: &ForgeSlug) -> Result<(), String> {
    let refused = || {
        format!("Kelpie opens only the issues and pull requests of {repo} on github.com from here.")
    };
    let rest = url
        .strip_prefix("https://github.com/")
        .ok_or_else(refused)?;
    let mut parts = rest.split('/');
    let (Some(owner), Some(name), Some(kind), Some(number), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(refused());
    };
    let own = format!("{owner}/{name}").eq_ignore_ascii_case(repo.as_str());
    let counted = number.len() <= 18
        && number.bytes().all(|byte| byte.is_ascii_digit())
        && number.parse::<u64>().is_ok_and(|number| number > 0);
    if own && counted && matches!(kind, "issues" | "pull") {
        Ok(())
    } else {
        Err(refused())
    }
}

/// How long `gdbus` waits for the portal, in seconds
const PORTAL_WAIT: &str = "10";

/// The portal's call for `url`, which `check` has already limited to plain characters
fn portal(gdbus: &PathBuf, url: &str) -> Command {
    let mut command = Command::new(gdbus);
    command.args([
        "call",
        "--session",
        "--timeout",
        PORTAL_WAIT,
        "--dest",
        "org.freedesktop.portal.Desktop",
        "--object-path",
        "/org/freedesktop/portal/desktop",
        "--method",
        "org.freedesktop.portal.OpenURI.OpenURI",
        // no parent window, the URL as a GVariant string, and no options
        "",
        &format!("'{url}'"),
        "{}",
    ]);
    command
}

/// Hand `url` to the OS's broker, through `run`
///
/// # Errors
///
/// `url` is not an issue or a pull request of `repo` (see [`check`]). A broker
/// that is missing or says no is not an error: the answer is [`LinkView::Copy`].
pub fn open_with(
    run: &dyn Run,
    os: Os,
    programs: &Programs,
    repo: &ForgeSlug,
    url: &str,
) -> Result<LinkView, String> {
    check(url, repo)?;
    let (mut command, said) = match os {
        Os::MacOs => (
            Command::new(&programs.open),
            "macOS would not open the link, so it is copied.",
        ),
        Os::Linux => (
            portal(&programs.gdbus, url),
            "No desktop portal answered, so the link is copied.",
        ),
        Os::Windows => {
            return Ok(copy(
                url,
                "Opening links waits on Windows being proven, so the link is copied.",
            ));
        }
    };
    if os == Os::MacOs {
        command.arg(url);
    }
    match run.run(&mut command) {
        Ok(output) if output.status.success() => Ok(LinkView::Opened),
        _ => Ok(copy(url, said)),
    }
}

fn copy(url: &str, message: &str) -> LinkView {
    LinkView::Copy {
        url: url.to_owned(),
        message: message.to_owned(),
    }
}

/// Open a ticket's issue or pull request page on GitHub
///
/// Answers [`LinkView::Copy`] where the OS has no broker to give it to: the
/// window copies the link and says why.
///
/// # Errors
///
/// No project is picked or it is not on GitHub, or the URL is not one of the
/// project's own issues or pull requests.
#[tauri::command(async)]
#[specta::specta]
pub fn open_link(state: State<'_, AppState>, url: String) -> Result<LinkView, String> {
    let project = state.project_id(None)?;
    let source = state.source(&project)?;
    let tree = state.tree();
    open_with(
        &*tree,
        Os::current(),
        &Programs::default(),
        source.repo(),
        &url,
    )
}
