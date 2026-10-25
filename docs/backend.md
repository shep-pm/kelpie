# Backend and stack

From a session on 2026-10-23. Decided unless marked open.

## Why kelpie exists

1. Claude Desktop routinely left zombie processes on the maintainer's laptop.
2. shep was already a good process manager, so SK was built on it as a harness of its own.
3. SK now builds every shep-pm project, and its token efficiency is far ahead of the weeks before it.
4. Kelpie is meant for other people eventually, not only the maintainer.
5. The long-run shape is like Claude Code and Claude Desktop: one backend, driven from a CLI and from an app.

## Names

1. `kelpie` is this app. `SK` is shep-kelpie.
2. SK keeps its name until it stops being a dog. It gets a new one then, if it needs one.

## Repos

1. The app is built here. SK stays in shep-pm/shep-kelpie and becomes the app's backend.
2. Not a fresh repo for everything: SK is 304k lines with its history, issues and CI, and it keeps building itself while the app grows.
3. Not the app inside SK: a Node and Tauri toolchain in SK's workspace, and a broken app change could stop the tool that builds everything.
4. Building the app here is also goal 2 in `goals.md`: SK starting a project from nothing (SK#897).
5. Open: whether SK's backend moves here when kelpie goes public.

## Shape

```
  kelpie (Tauri 2)          kelpie CLI (today: shep kelpie)
  React + TS webview
  thin Rust side
        |                          |
        +---- SK's client crate ---+    typed requests and events
                     |  local socket (Unix socket, named pipe on Windows)
                     v
               SK, headless daemon
               runner, board, rulings, leases, PM
               adapters: claude, codex, pi, gh
                     |  shep-client
                     v
               shepherd: workers and their children, no zombies
```

1. The app and the CLI are equal clients of SK. Closing the app never stops work.
2. shep stays the process layer only. SK owns the domain.
3. Going through shep for the domain would be two layers, with every product decision waiting on shep.

## What SK looks like today

1. Its verbs (`Rule`, `Board`, `Triage`, `Drain`, ...) are SK's own `Request` enum in `crates/runner/src/runner/trigger/mod.rs`.
2. They travel inside shep's generic `Trigger` request: the shepherd, then shep-channel, then a string parsed back out. shep never knows what a ruling is.
3. The domain crates (`core`, `runner`, `adapters`, `process`, `tree`, `panel`, `facts`) don't depend on shep beyond `core`'s client port.
4. What makes SK a dog is thin: the verbs, its settings in shep's `overrides.json`, and supervision.

## Stack

| Layer | Pick | Why |
|---|---|---|
| UI | Tauri 2, React + TS, Vite, Tailwind or shadcn, zustand, TanStack Query | The maintainer's strongest stack; Tauri is Rust-native and small |
| Types across | `tauri-specta` (still rc.25), TS types generated from Rust | One source of truth |
| Tauri's Rust side | A client only | The app is a view, not the engine |
| API | A `protocol` crate on the transport pattern of `shep_core::transport` | Borrows shep's crates; Windows already works there |
| Domain | SK's crates, minus the dog wrapper | Kept as they are |
| State | Files, as today; maybe SQLite for the board and journal later | Not a day-one change |
| Processes | shep: SK runs workers as sheep | Why kelpie exists |
| Install | Tauri bundles SK and shep as sidecars (`externalBin`) with their own `SHEP_HOME` | Nobody installs shep first |

The earlier view picks still stand (`gui-idea.md`, Tech), with one change: React Flow's docs now recommend dagre over ELK, and the layout for big graphs moves into SK (see Visualizers).

## Decided on 2026-10-24

From a survey of the framework landscape as of October 2026. Sources are in the session's three research reports; the key facts are below.

### Visualizers: webview, with the heavy work in Rust

1. Native Rust wins only when the canvas is the whole app, with thousands of live nodes at 120 fps. Rerun (egui and wgpu) shows it can be done.
2. Kelpie's views answer one question with 5 to 50 nodes, so the renderer is not the bottleneck.
3. Indexing and graph layout run in Rust, in SK, and send coordinates over the protocol. The webview only draws.
4. A big graph draws on the GPU in the webview: cosmos.gl (GPU force layout, MIT) or Sigma.js.
5. If one view ever outgrows that, a Rust to WASM and WebGPU canvas fits inside the same webview.
6. No Rust-native framework fits today: none has an ELK-style layout, a mature graph widget, a diff view and a terminal together, and none shares a desktop UI with mobile. GPUI with gpui-component came closest on desktop.

### Platforms: one desktop app, native mobile, one Rust client

| Target | Approach | Why |
|---|---|---|
| macOS, Linux, Windows | One Tauri 2 app | Desktop is UI-heavy and shared |
| Android | Native Kotlin, small | Push and background behaviour are where cross-platform hurts most, and the app is mostly one screen |
| iOS, later | Native Swift | Same |
| All | One Rust client crate for SK's protocol, bound to Kotlin and Swift with UniFFI | Protocol logic written once |

1. Agents make per-platform code cheap to write, not to review, test and release. The maintainer's attention is the scarce part.
2. Tauri 3 is alpha (3.0.0-alpha.5, 2026-10-14). Its bundled Chromium (CEF) runtime is the way out if WebKit differences bite on desktop, without leaving Tauri. OpenCode left Tauri for Electron in 2026, reportedly over WebKit rendering.
3. Electron is the industry default (Claude Desktop, Codex, Cursor, Goose, OpenCode) and stays the fallback. The React UI moves with little change.

### Notifications: kelpie's own connection, no Google needed

1. Tauri 2's notification plugin shows native notifications with action buttons on Android and iOS, but has no remote push. Its push PR has been open since 2024-11.
2. Android allows a foreground service holding one connection, which keeps working in doze mode. ntfy's instant delivery works this way.
3. So the Android app keeps one connection to SK, written fresh or borrowing ntfy-android's connection service (Apache-2.0). No FCM.
4. It fixes what made ntfy painful: rulings grouped by project with a screen each, and pairing once by QR instead of re-entering a token.
5. The phone reaches SK over Tailscale, or a small relay for people without it.
6. Some phone makers' battery managers kill background connections; the app asks to be exempted.
7. iOS allows no background connection, so it needs APNs. SK, or a small server it runs, calls APNs directly with the maintainer's own key. Nobody else sits in between.

## What changes in SK

| Today | Later |
|---|---|
| Domain crates | Unchanged |
| `Request` inside shep's `Trigger` | A `protocol` crate on SK's own socket |
| `shep kelpie <verb>` | `kelpie <verb>`, with `shep kelpie` kept as an alias for a while |
| Settings in `overrides.json` | SK's own settings file |
| Lookout panes | Not grown further; the app replaces them |

Recommended, not decided: no new CLI or lookout surface in SK beyond what the app also needs.

## For other people

1. Not everyone uses Claude. SK's shipped agents name models and a local script only the maintainer has.
2. SK#1232 (parked until shep#1079): ship role baselines with no model, and a generator that binds a harness and model per role.
3. Requiring shep: bundling it as a sidecar turns it from an install burden into something users never see.

## Order

1. SK#1236: SK answers on its own socket, with a typed protocol and status events. Listed in shep#1104 after SK#1201.
2. Here: redo step 0 with `init-shep`, then a Tauri window that shows one project's board through SK's client crate. That is the first tracer bullet.

## Parked notes from the session

1. The overwhelm is about order, not about the plan. shep#1104 is the one place for order.
2. The last few days of SK stress testing were worth it: they found the PM's invented cap and the runner-lock timeouts.
3. The thing to watch: SK polish that does not point at the app.
