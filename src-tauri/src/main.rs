//! kelpie-app: the binary that opens kelpie's window.
//!
//! The wiring lives in `kelpie_app`, so this stays a thin entry point.

// No console window beside the app on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sys;

fn main() {
    if let Err(error) = kelpie_app::run() {
        sys::failure(&error.to_string());
        std::process::exit(1);
    }
}
