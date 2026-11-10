//! Kelpie's window: the Tauri 2 app crate that hosts the engine and its IPC
//!
//! The crate is thin on purpose (ADR 0002): supervision and sessions live in
//! `kelpie-engine`, and nothing here spawns a process or registers a shell
//! plugin (ADR 0001). The one process it bundles is the reaper sidecar, which
//! the engine's tree starts, beside the app's own executable.
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

use std::path::{Path, PathBuf};

use kelpie_engine::login::Store;

use tauri::{Manager as _, RunEvent, Runtime};
use tauri_plugin_dialog::DialogExt as _;
use tauri_specta::collect_commands;

pub mod pm;
pub mod settings;
pub mod state;

use state::AppState;

/// Where the bindings are written, from the app crate's folder
const BINDINGS: &str = "../src/bindings.ts";

/// The commands the webview may call, typed for the bindings
fn commands<R: Runtime>() -> tauri_specta::Builder<R> {
    tauri_specta::Builder::<R>::new()
        .commands(collect_commands![
            pm::project_get,
            pm::project_pick,
            pm::pm_send,
            pm::pm_cancel,
            pm::pm_history,
            pm::pm_retry,
            settings::login_set,
            settings::login_status,
            settings::login_clear,
            settings::harness_options,
        ])
        // Token counts are far below 2^53, which a JS number holds exactly
        .dangerously_cast_bigints_to_number()
        // One type per Rust type: the events only ever go out, and `Usage` is written in full
        .disable_serde_phases()
}

/// Write `src/bindings.ts`: the commands and the types they carry, in TypeScript
///
/// `pnpm tauri dev` does it on every start of a debug build, and the checked-in
/// file is the one it wrote.
///
/// # Errors
///
/// The file cannot be written.
pub fn export_bindings(path: impl AsRef<Path>) -> Result<(), specta_typescript::Error> {
    commands::<tauri::Wry>().export(specta_typescript::Typescript::default(), path)
}

/// Wire kelpie's app onto a builder, whatever runtime it carries.
///
/// It takes the builder rather than returning one so a test can wire
/// `tauri::test::mock_builder()` and build it on a mock context, while [`run`]
/// wires the real `tauri::Wry` runtime.
///
/// On setup the app opens its state in the app data folder, sweeping what the
/// previous run left running before anything starts (ADR 0001), and hands the
/// tree the reaper beside the app's executable, which the engine starts before
/// any session can. Logins are kept in the OS keychain.
pub fn builder<R: Runtime>(app: tauri::Builder<R>) -> tauri::Builder<R> {
    builder_with(app, Store::keychain(), Some(state::reaper_path()))
}

/// [`builder`], keeping logins in `logins` instead of the OS keychain and
/// starting the reaper at `reaper`
///
/// A test passes `Store::memory()` and `None`, so it never touches the person's
/// keychain or needs the sidecar `scripts/build-reaper.sh` builds.
pub fn builder_with<R: Runtime>(
    app: tauri::Builder<R>,
    logins: Store,
    reaper: Option<PathBuf>,
) -> tauri::Builder<R> {
    app.manage(logins.clone())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(commands::<R>().invoke_handler())
        .setup(move |app| {
            let data_dir = app.path().app_data_dir()?;
            let handle = app.handle().clone();
            let picker = Box::new(move || {
                handle
                    .dialog()
                    .file()
                    .blocking_pick_folder()
                    .map(|folder| folder.into_path().map_err(|error| error.to_string()))
                    .transpose()
            });
            app.manage(AppState::open(
                data_dir,
                logins,
                Box::new(pm::open),
                picker,
                reaper.as_deref(),
            )?);
            Ok(())
        })
}

/// Stop every process of this run, when the app is asked to quit
fn on_run_event<R: Runtime>(app: &tauri::AppHandle<R>, event: &RunEvent) {
    if let RunEvent::ExitRequested { .. } = event
        && let Some(state) = app.try_state::<AppState>()
        && let Err(error) = state.stop_all()
    {
        eprintln!("kelpie: could not stop every process: {error}");
    }
}

/// Build kelpie's window and run its event loop.
///
/// A running app never returns: Tauri ends the process when its loop ends.
/// A debug build writes `src/bindings.ts` first.
///
/// # Errors
///
/// The app could not be built: `tauri.conf.json` did not load, or the webview
/// runtime could not start.
pub fn run() -> Result<(), tauri::Error> {
    #[cfg(debug_assertions)]
    if let Err(error) = export_bindings(Path::new(env!("CARGO_MANIFEST_DIR")).join(BINDINGS)) {
        eprintln!("kelpie: could not write {BINDINGS}: {error}");
    }
    let app = builder::<tauri::Wry>(tauri::Builder::default()).build(tauri::generate_context!())?;
    app.run(|app, event| on_run_event(app, &event));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BINDINGS, builder_with, export_bindings};
    use kelpie_engine::login::Store;
    use tauri::Manager as _;
    use tauri::test::{mock_builder, mock_context, noop_assets};

    #[test]
    fn the_app_builds_on_a_mock_runtime() {
        let app = builder_with(mock_builder(), Store::memory(), None)
            .build(mock_context(noop_assets()))
            .expect("kelpie's app should build on the mock runtime");
        assert!(
            app.webview_windows().is_empty(),
            "the builder creates no window of its own"
        );
    }

    #[test]
    fn the_checked_in_bindings_are_the_ones_the_commands_generate() {
        let checked_in = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(BINDINGS);
        let fresh = std::env::temp_dir().join(format!(
            "kelpie-bindings-{}.ts",
            kelpie_engine::run::RunId::new().unwrap()
        ));
        export_bindings(&fresh).unwrap();
        let generated = std::fs::read_to_string(&fresh).unwrap();
        let _ = std::fs::remove_file(&fresh);
        let held = std::fs::read_to_string(&checked_in).unwrap_or_default();
        assert!(
            held == generated,
            "src/bindings.ts is out of date: run `pnpm tauri dev`, which writes it, and commit it"
        );
    }
}
