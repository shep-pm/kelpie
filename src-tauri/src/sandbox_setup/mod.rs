//! What the PM's last start found about its sandbox
//!
//! Some machines cannot start a sandbox until a one-time change is made on
//! them: on Ubuntu 24.04 and later, AppArmor strips the capabilities from
//! bwrap's user namespace (`docs/design/sandbox-setup.md`). [`pm::open`]
//! probes the PM's sandbox before it builds the session and records here what
//! it found, and [`pm_sandbox`] hands that to the chat, which says why the
//! first message failed in the note's words instead of showing bwrap's.
//!
//! Only a [`Refusal`] is kept. The home-folder refusal and every other error
//! are no setup a person can make, so the chat shows them as it always did.
//! When the project lets the PM run without a sandbox, the refusal stays
//! recorded and the view says the session is `unsandboxed`.
//!
//! The view also says whether Install is on offer: only where the package put
//! kelpie's profile on disk, so an AppImage, which runs from a new mount path
//! each launch, and a dev build are offered Run without a sandbox alone
//! (`docs/design/sandbox-setup.md`, The AppImage). [`sandbox_install`] runs
//! Install and, once the profile is loaded, ends the PM's session so the next
//! send starts it sandboxed.
//!
//! [`pm::open`]: crate::pm::open

use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

use kelpie_engine::sandbox::Refusal;
use serde::Serialize;
use tauri::State;

use crate::state::AppState;

pub mod install;

/// Why a sandbox cannot start, as the webview tells the two apart
// wire format: the bindings hold these two strings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RefusalView {
    /// The namespace has no capabilities in it, which kelpie's AppArmor profile fixes
    NoCapabilities,

    /// The kernel gives no namespace at all, which no profile fixes
    NoNamespaces,
}

impl RefusalView {
    /// The view of `refusal`, `None` for one the webview has no words for yet
    ///
    /// [`Refusal`] grows with what bwrap says, so a refusal added to the engine
    /// first reads as no refusal here, and the chat shows its error as it does
    /// any other.
    fn of(refusal: Refusal) -> Option<Self> {
        match refusal {
            Refusal::NoCapabilities => Some(Self::NoCapabilities),
            Refusal::NoNamespaces => Some(Self::NoNamespaces),
            _ => None,
        }
    }
}

/// Whether kelpie can load its AppArmor profile on this machine
// wire format: the bindings hold these two strings
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Install {
    /// The package put the profile on disk, so Install can load it
    Available,

    /// Only the `.deb` or `.rpm` can install the profile: kelpie runs as an
    /// AppImage, or the packaged profile is not on this machine
    #[default]
    PackageOnly,
}

impl Install {
    /// What a start offers, given the `APPIMAGE` variable and where the profile should be
    ///
    /// An AppImage sets `APPIMAGE` to its own path. A dev build has no
    /// packaged profile, so it reads as an AppImage does.
    fn detect(appimage: Option<&OsStr>, profile: &Path) -> Self {
        if appimage.is_some_and(|path| !path.is_empty()) || !profile.is_file() {
            Self::PackageOnly
        } else {
            Self::Available
        }
    }

    /// What this machine offers
    fn here() -> Self {
        Self::detect(
            std::env::var_os("APPIMAGE").as_deref(),
            Path::new(install::PROFILE),
        )
    }
}

/// What the chat is told about the PM's sandbox
// wire format: the bindings hold this object
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, specta::Type)]
pub struct SandboxView {
    /// Why the PM's last start could not make its sandbox, `None` when it
    /// found no refusal
    pub refusal: Option<RefusalView>,

    /// Whether the PM's session runs without a sandbox, because the project
    /// allows it and the sandbox could not start. `refusal` stays set then.
    pub unsandboxed: bool,

    /// Whether Install can load the profile here
    pub install: Install,
}

/// What the PM's last start found, held by the app for the whole run
#[derive(Debug, Default)]
pub struct SandboxSetup {
    found: Mutex<SandboxView>,
    install: Install,
}

impl SandboxSetup {
    /// A record for this machine: Install is on offer where the package put the profile
    ///
    /// [`SandboxSetup::default`] offers none, which is what a test wants.
    #[must_use]
    pub fn detected() -> Self {
        Self::offering(Install::here())
    }

    /// A record that offers `install`
    #[must_use]
    pub fn offering(install: Install) -> Self {
        Self {
            found: Mutex::new(SandboxView {
                install,
                ..SandboxView::default()
            }),
            install,
        }
    }

    /// Keep what the PM's start found, replacing what the one before found
    ///
    /// `None` is a start that found no refusal, so a refusal stays only until
    /// the next start that works.
    pub fn record(&self, found: Option<Refusal>) {
        self.keep(SandboxView {
            refusal: found.and_then(RefusalView::of),
            unsandboxed: false,
            install: self.install,
        });
    }

    /// Keep a start that ran the PM without a sandbox because of `found`
    ///
    /// The refusal stays recorded while the session runs, so the chat still
    /// has something to offer an install on.
    pub fn record_unsandboxed(&self, found: Refusal) {
        self.keep(SandboxView {
            refusal: RefusalView::of(found),
            unsandboxed: true,
            install: self.install,
        });
    }

    fn keep(&self, view: SandboxView) {
        *self.found.lock().unwrap_or_else(PoisonError::into_inner) = view;
    }

    /// What the last start found
    #[must_use]
    pub fn view(&self) -> SandboxView {
        *self.found.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the PM's last start found about its sandbox
///
/// The chat asks when a turn ends failed. Reads what the PM's last start kept,
/// and starts nothing.
#[tauri::command]
#[specta::specta]
#[must_use]
pub fn pm_sandbox(state: State<'_, AppState>) -> SandboxView {
    state.sandbox_setup().view()
}

/// Load kelpie's AppArmor profile through `pkexec`, and end the PM's session once it is loaded
///
/// Waits on the password prompt, so it runs off the main thread. A loaded
/// profile ends the held session, even an unsandboxed one, and the next send
/// opens one that probes the sandbox again and starts it sandboxed. The
/// command is fixed: nothing from the webview reaches it
/// (`docs/design/sandbox-setup.md`, Install).
#[tauri::command(async)]
#[specta::specta]
#[must_use]
pub fn sandbox_install(state: State<'_, AppState>) -> install::InstallOutcome {
    let outcome = install::install(&state.tree());
    if outcome == install::InstallOutcome::Installed {
        state.end_pm_session();
    }
    outcome
}

#[cfg(test)]
mod tests;
