//! Kelpie's window: the Tauri 2 app crate that hosts the engine and its IPC
//!
//! The crate is thin on purpose (ADR 0002): supervision and sessions live in
//! `kelpie-engine`, and nothing here spawns a process, registers a shell plugin
//! or carries a sidecar (ADR 0001). [`run`] hosts the engine's process tree for
//! the run, and commands reach that tree to start anything.
//!
//! # Quick start
//!
//! ```no_run
//! kelpie_app::run().ok();
//! ```
//!
//! [`builder`] holds the app wiring, and [`run`] opens the window
//! `src-tauri/tauri.conf.json` describes.

#![doc(test(attr(deny(warnings))))]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kelpie_engine::login::Store;
use kelpie_engine::run::RunId;
use kelpie_engine::tree::{ProcessTree, sweep_leftovers};
use tauri::Manager as _;

pub mod settings;

/// The file in the data folder that lists this run's process groups
const RUN_FILE: &str = "run.json";

/// How long the children are given to stop before the tree kills them
const STOP_GRACE: Duration = Duration::from_secs(10);

/// The engine the window hosts: this run's process tree, and the data folder its
/// bridges, session state and harness answers live in
///
/// Commands take it as state. A test builds one of its own with [`Host::start`].
#[derive(Debug)]
pub struct Host {
    /// The tree every process of this run starts through (ADR 0001)
    pub tree: Arc<ProcessTree>,
    /// The folder this run's data lives in (ADR 0008)
    pub data: PathBuf,
}

impl Host {
    /// Make `data` and start this run's tree in it
    ///
    /// Whatever a run that died without quitting left behind goes first, before
    /// this run can start anything.
    ///
    /// # Errors
    ///
    /// The data folder could not be made, or the sweep or the run file failed.
    pub fn start(data: &Path) -> io::Result<Self> {
        fs::create_dir_all(data)?;
        let run_file = data.join(RUN_FILE);
        sweep_leftovers(&run_file)?;
        let tree = ProcessTree::new(RunId::new()?, run_file)?;
        Ok(Self {
            tree: Arc::new(tree),
            data: data.to_path_buf(),
        })
    }

    /// Stop every process this run started
    ///
    /// Returns whether any process could not be stopped.
    pub fn stop(&self) -> io::Result<()> {
        self.tree.stop_all(STOP_GRACE).map(|_| ())
    }
}

/// Wire kelpie's app onto a builder, whatever runtime it carries.
///
/// It takes the builder rather than returning one so a test can wire
/// `tauri::test::mock_builder()` and build it on a mock context, while [`run`]
/// wires the real `tauri::Wry` runtime.
pub fn builder<R: tauri::Runtime>(app: tauri::Builder<R>) -> tauri::Builder<R> {
    builder_with(app, Store::keychain())
}

/// [`builder`], keeping logins in `logins` instead of the OS keychain
///
/// A test passes `Store::memory()`, so it never touches the person's keychain.
/// The process tree is not wired here: [`run`] manages it once the data folder
/// is known, and a test that needs one manages its own.
pub fn builder_with<R: tauri::Runtime>(app: tauri::Builder<R>, logins: Store) -> tauri::Builder<R> {
    app.manage(logins).invoke_handler(tauri::generate_handler![
        settings::login_set,
        settings::login_status,
        settings::login_clear,
        settings::harness_options,
    ])
}

/// Build kelpie's window and run its event loop.
///
/// A running app never returns: Tauri ends the process when its loop ends.
///
/// # Errors
///
/// The app could not be built: `tauri.conf.json` did not load, or the webview
/// runtime could not start.
pub fn run() -> Result<(), tauri::Error> {
    let app = builder::<tauri::Wry>(tauri::Builder::default())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            app.manage(Host::start(&data)?);
            Ok(())
        })
        .build(tauri::generate_context!())?;
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            let _ = app.state::<Host>().stop();
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::builder;
    use tauri::Manager as _;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    #[test]
    fn the_app_builds_on_a_mock_runtime() {
        let app = builder(mock_builder())
            .build(mock_context(noop_assets()))
            .expect("kelpie's app should build on the mock runtime");
        assert!(
            app.webview_windows().is_empty(),
            "the builder creates no window of its own"
        );
    }
}
