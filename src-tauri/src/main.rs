//! kelpie-app: the binary that opens kelpie's window.
//!
//! The wiring lives in `kelpie_app`, so this stays a thin entry point.

// No console window beside the app on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod sys;

fn main() {
    // First, before Tauri: crabpot runs this binary again as its helper on
    // Linux and Windows (ADR 0005), and a `main` that skipped this would run
    // there in place of the command.
    kelpie_engine::sandbox::run_from_args();
    if let Err(error) = kelpie_app::run() {
        sys::failure(&error.to_string());
        std::process::exit(1);
    }
}
