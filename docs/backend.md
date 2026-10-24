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
| Types across | `tauri-specta`, TS types generated from Rust | One source of truth |
| Tauri's Rust side | A client only | The app is a view, not the engine |
| API | A `protocol` crate on the transport pattern of `shep_core::transport` | Borrows shep's crates; Windows already works there |
| Domain | SK's crates, minus the dog wrapper | Kept as they are |
| State | Files, as today; maybe SQLite for the board and journal later | Not a day-one change |
| Processes | shep: SK runs workers as sheep | Why kelpie exists |
| Install | Tauri bundles SK and shep as sidecars (`externalBin`) with their own `SHEP_HOME` | Nobody installs shep first |

The earlier view picks still stand (`gui-idea.md`, Tech): React Flow with ELK, its own MCP server.

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
