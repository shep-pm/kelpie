# Tauri 2 and React

Decided on 2026-10-23 in `docs/backend.md`, and re-checked on 2026-11-08 in `docs/gui-framework.md`, which holds the versions and sources.

Kelpie needs a graph with automatic layout, a terminal, a diff view, streaming markdown, a component library, and code that agents write well. Tauri 2 with React, TypeScript and Vite is the only stack that covers all six with maintained libraries, and it keeps the backend in Rust beside `shep-tree`.

## Considered options

- **Electron.** The fallback if WebKitGTK on Linux fails us. The same React code moves over; the cost is size, memory, and a Node layer between the UI and the engine.
- **Tauri 3 with CEF.** Not before 2027: CEF on Linux has open bugs (100% idle CPU, lost keyboard input), and plugin APIs still break.
- **A native Rust UI** (GPUI, Freya, egui). None has a graph and a diff view together, and the closest, GPUI, ships through snapshots one person republishes.
- **Svelte, Vue or SolidJS.** Each has gaps in the six needs (terminal, streaming markdown, the graph), and agents write React best. SolidJS's speed doesn't matter at kelpie's scale.

## Consequences

- The engine has no Tauri dependency, so leaving Tauri for Electron would replace the app crate and the IPC, not the engine.
- Linux is the platform to watch: three WebKitGTK issues (Wayland, blank screen, NVIDIA) were open on 2026-11-08.
- Streams go over Tauri Channels, not events.
