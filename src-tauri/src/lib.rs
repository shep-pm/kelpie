//! Kelpie's window: the Tauri 2 app crate that hosts the engine and its IPC
//!
//! The crate is thin on purpose (ADR 0002): supervision and sessions live in
//! `kelpie-engine`, and nothing here spawns a process, registers a shell plugin
//! or carries a sidecar (ADR 0001).
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

/// Wire kelpie's app onto a builder, whatever runtime it carries.
///
/// It takes the builder rather than returning one so a test can wire
/// `tauri::test::mock_builder()` and build it on a mock context, while [`run`]
/// wires the real `tauri::Wry` runtime.
pub fn builder<R: tauri::Runtime>(app: tauri::Builder<R>) -> tauri::Builder<R> {
    app
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
    let app = builder::<tauri::Wry>(tauri::Builder::default()).build(tauri::generate_context!())?;
    app.run(|_app, _event| {});
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
