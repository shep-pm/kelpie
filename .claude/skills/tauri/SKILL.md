---
name: tauri
description: Use before touching src-tauri/, tauri.conf.json, capabilities, a #[tauri::command], an invoke call, or the app's CI build. Tauri 2 facts and kelpie's choices, for agents whose training mixes in Tauri 1.
---

# Tauri 2 in kelpie

Kelpie runs Tauri 2.12 with React 19, Vite and TypeScript. Most of what a model remembers about Tauri is version 1, so check every API against the table below before writing it. [reference.md](reference.md) has the detail and snippets for commands, channels, capabilities, testing and CI, with a source for each fact.

## Tauri 1 forms that no longer exist

| Tauri 1 | Tauri 2 |
|---|---|
| `tauri.allowlist` | `src-tauri/capabilities/*.json` and permissions |
| `@tauri-apps/api/tauri` | `@tauri-apps/api/core` (`invoke`, `Channel`) |
| `@tauri-apps/api/{fs,dialog,shell,process,notification,updater,...}` | `@tauri-apps/plugin-<name>` plus the `tauri-plugin-<name>` crate |
| `Window`, `WindowBuilder`, `get_window` | `WebviewWindow`, `WebviewWindowBuilder`, `get_webview_window` |
| `tauri` key in `tauri.conf.json` | `app`; `bundle`, `productName`, `version`, `identifier` move to the top level |
| `build.devPath`, `build.distDir` | `build.devUrl`, `build.frontendDist` |
| `systemTray`, `SystemTray` | `app.trayIcon`, `tauri::tray::TrayIconBuilder` |
| logic in `main.rs` | `lib.rs` with `pub fn run()`; `main.rs` only calls it |

## Kelpie's choices

1. **The engine has no Tauri dependency.** Process supervision, sessions and the protocol live in plain crates and are tested as plain Rust. The app crate (`src-tauri`) is thin: commands that call the engine, and lifecycle wiring.
2. **Process output and session streams go over `tauri::ipc::Channel<T>`.** It is ordered and built for throughput. Events are for rare, small notices only, since they are JSON, untyped, and may arrive out of order.
3. **Nothing is spawned through Tauri.** The shell plugin and `Command.sidecar` from JS break ADR 0001: every process goes through the engine's tree. The reaper is a sidecar for bundling only (`bundle.externalBin`, file named `<name>-<target triple>`), and the engine starts it.
4. **Tauri kills nothing on exit, and `RunEvent::Exit` never fires on SIGKILL, abort or `process::exit`.** The engine's stop ladder runs on `ExitRequested`, and the reaper covers the rest. Exit hooks don't count as cleanup.
5. **Build with `tauri build`.** A plain `cargo build --release` produces a dev-mode binary that loads `devUrl`.
6. **Vite runs on port 1520** (Tauri's template default 1420, plus 100, to stay off a dev server already running on the default). Set `server.port: 1520` with `strictPort: true` and `build.devUrl` to match. `--port` and `TAURI_CLI_PORT` don't move Vite.
7. **`src-tauri` is a member of the root Cargo workspace.** Artifacts and bundles land in the root `target/`, not `src-tauri/target`.
8. **tauri-specta is pinned exact**, `=2.0.0-rc.26` (still a release candidate), with the `specta` rc it requires. It types `Channel<T>` arguments directly; a channel nested inside a struct needs `JavaScriptChannelId`.

## Mistakes that compile

- Two `invoke_handler` calls: only the last one counts. Put every command in one `generate_handler!`.
- `State<'_, T>` with a type that was never `.manage()`d: no compile error. `invoke` rejects at runtime.
- A `std::sync::MutexGuard` held across `.await`: the future stops being `Send`. Use `tokio::sync::Mutex` there.
- Argument names: JS sends camelCase (`{ jobId }`) for Rust `job_id`.
- A missing permission reads `Command X not allowed by ACL` in a release build. Debug builds name the permission.
