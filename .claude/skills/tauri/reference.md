# Tauri 2 reference

Read at tauri 2.12.5 (`2e23492`), tauri-docs `5e49cc6`, plugins-workspace `92e47a1`, tauri-specta `15a80c3`, create-tauri-app `c6f5cf6`, tauri-action `47f975c`, on 2026-11-08. "(source)" marks a fact read from code rather than docs, which can change without notice. Rust MSRV for tauri is 1.90.

## Commands

From `develop/calling-rust.mdx` and `develop/state-management.mdx`.

- A command defined in `lib.rs` must not be `pub`; one defined in another module must be. Names are unique across modules.
- Sync commands run on the main thread unless marked `#[tauri::command(async)]`. An `async fn` command runs on Tauri's runtime, so its future must be `Send + 'static` (source).
- An async command taking a borrowed argument (`&str`, `State<'_, T>`) must return `Result`.
- `#[tauri::command(rename_all = "snake_case")]` switches JS argument names from camelCase.
- Errors must implement `Serialize`; the JS promise rejects with that value.
- Tauri's tokio runtime is a singleton: use `tauri::async_runtime::spawn`.
- The `AppHandle` example in `calling-rust.mdx` is stale v1 code (`global_shortcut_manager`).

```rust
#[derive(Debug, thiserror::Error)]
enum Error { #[error("{0}")] Msg(String) }
impl serde::Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

#[tauri::command]
async fn start(state: tauri::State<'_, tokio::sync::Mutex<Engine>>, job_id: String) -> Result<u32, Error> {
    let mut engine = state.lock().await;
    engine.start(&job_id).map_err(|e| Error::Msg(e.to_string()))
}
// JS: await invoke<number>('start', { jobId })
```

## Channels

From `develop/calling-frontend.mdx`, `crates/tauri/src/ipc/channel.rs` and `packages/api/src/core.ts`.

- Each message carries an index, and the JS `Channel` reorders, so `onmessage` sees messages in `send()` order (source).
- Dropping the last Rust clone ends the channel and the JS side unregisters. A `Channel` is cheap to clone and can be kept in engine state.
- Bytes: `Channel<InvokeResponseBody>` with `InvokeResponseBody::Raw(vec)` arrives as an `ArrayBuffer`. `Channel<Vec<u8>>` sends a JSON array of numbers, which is slow for pty output.
- Capabilities don't apply to events or channels.

```rust
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "event", content = "data")]
enum Out { Line { stream: String, text: String }, Exit { code: Option<i32> } }

#[tauri::command]
async fn run(on_event: tauri::ipc::Channel<Out>) {
    on_event.send(Out::Line { stream: "out".into(), text: "hi".into() }).ok();
}
```

```ts
import { invoke, Channel } from '@tauri-apps/api/core';
const ch = new Channel<Out>();
ch.onmessage = (m) => { /* ordered */ };
await invoke('run', { onEvent: ch });
```

For an event, `listen()` returns a promise. Await it and call the `unlisten` it gives in the effect's cleanup.

## Capabilities and permissions

From `security/capabilities.mdx`, `security/permissions.mdx`, and `crates/tauri/src/webview/mod.rs` (source).

- Every file in `src-tauri/capabilities/` is enabled unless `app.security.capabilities` lists identifiers. Capabilities match window labels, not titles.
- Plugin permissions look like `<plugin>:allow-<command>`; `<plugin>:default` is the plugin's set. Core is `core:default`.
- App commands are allowed for every local window by default. Listing commands in `AppManifest::commands` in `build.rs`, or adding `src-tauri/permissions/*.toml`, turns on enforcement for every app command at once. Each then needs `allow-<command>` with `_` written as `-`.
- Remote origins are denied unless a capability has `remote.urls`.

```rust
// build.rs, only if kelpie opts into per-command permissions
fn main() {
    tauri_build::try_build(tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(&["start_engine"])))
        .unwrap();
}
```

## Lifecycle

From `crates/tauri/src/app.rs` (source).

- `Builder::setup` runs at `Ready`; manage state there.
- `RunEvent` is `#[non_exhaustive]`. `ExitRequested { code: None, api }` fires when the last window closes; `code: Some(n)` means `app.exit(n)`. `api.prevent_exit()` keeps the app alive, e.g. to live in the tray.
- `Exit` is the last event. `App::run` never returns, and calls `std::process::exit` after it. Use `run_return` to run code after the loop.
- `on_window_event`: `WindowEvent::CloseRequested { api }` with `api.prevent_close()`. A JS `onCloseRequested` listener prevents the close on its own.
- Single instance: `tauri-plugin-single-instance`, registered first in the builder, under `#[cfg(desktop)]`. It uses DBus on Linux.

## Sidecars

From `develop/sidecar.mdx`.

- `bundle.externalBin: ["binaries/reaper"]` is relative to `tauri.conf.json`, and the file must be `binaries/reaper-<target triple>[.exe]`. `rustc --print host-tuple` gives the triple.
- Kelpie starts the reaper from the engine with `std::process::Command`, so it resolves the path itself. Where the bundler puts the sidecar inside each OS bundle is not recorded here yet; the Phase 1 slice that adds the reaper confirms it and adds it to this file.

## Dev, build and CI

- `tauri dev` runs `build.beforeDevCommand` from the folder holding `package.json`, then polls `devUrl` for about three minutes before giving up (source).
- `create-tauri-app`'s React template sets `clearScreen: false`, `server.watch.ignored: ["**/src-tauri/**"]`, and takes the HMR host from `TAURI_DEV_HOST`.
- Bundles land under `target/release/bundle/`: `deb/`, `rpm/`, `appimage/` on Linux, `macos/` and `dmg/` on macOS, `msi/` and `nsis/` on Windows.
- `generate_context!` reads `tauri.conf.json` relative to the app crate. `frontendDist` and `externalBin` are relative to that file.
- Linux build packages (`start/prerequisites.mdx`): `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev`. Kelpie's CI runs on the self-hosted pool, whose image has none of them, so the job installs them with apt.
- A pool job without an arch label may land on an arm64 box. WebKitGTK builds there, but slowly; `[self-hosted, X64]` pins a heavy job to the x86 box.

## Testing

From `crates/tauri/src/test/mod.rs` and `develop/Tests/`.

- Keep logic in the engine crates and unit test it there. Test the command wiring thinly.
- `tauri::test` needs the `test` feature and is marked unstable: `mock_builder()`, `mock_context(noop_assets())`, `get_ipc_response`. Commands must be generic over `R: Runtime` to run under `MockRuntime`. No real webview runs.
- Frontend: `@tauri-apps/api/mocks` has `mockIPC`, `mockWindows` and `clearMocks`. Event mocking needs `{ shouldMockEvents: true }`.

## tauri-specta

- `2.0.0-rc.26` is the newest on crates.io (2026-10-21); git HEAD is rc.27. Pin with `=` and match the `specta` and `specta-typescript` versions that rc.26 names on docs.rs.
- `#[tauri::command] #[specta::specta]`, then `Builder::<Wry>::new().commands(collect_commands![..])`, export TS under `#[cfg(debug_assertions)]`, and pass `builder.invoke_handler()`.
- `ErrorHandlingMode::Result` returns `{ status: "ok", data }` or `{ status: "error", error }` instead of throwing.
