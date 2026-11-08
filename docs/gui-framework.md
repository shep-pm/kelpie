# The app's GUI framework

A second look on 2026-11-08 at the shell and the frontend picked in `backend.md`. It covers webview shells, native Rust UIs, and the framework inside the webview. SolidJS was judged on purpose, and harshly.

Sources: two research reports from this session, using only primary sources (official docs, release notes, GitHub, the npm and crates.io registries). The key facts are below with their links. "Unconfirmed" means no primary source was found.

## Verdict

1. **The stack doesn't change:** Tauri 2, React + TS, Vite, TanStack Router, shadcn, React Flow, Streamdown, zustand and TanStack Query.
2. **Two corrections to `backend.md`:**
   - Agent logs and pty output stream over Tauri Channels, not events.
   - The first tracer bullet runs on Linux too.
3. **SolidJS comes fourth, behind Svelte and Vue.** Its one real edge is speed, and that edge doesn't matter at kelpie's scale.
4. **No native Rust UI fits yet.** GPUI is still the closest, but it is on shakier ground than the October survey said.

## Kelpie's needs, and who covers them

Kelpie needs these from a UI:
- a graph with automatic layout
- a terminal
- a diff view
- streaming markdown
- a component library
- code that AI agents write well

| Stack | Graph + layout | Terminal | Diff | Streaming markdown | Components | Agents write it well |
|---|---|---|---|---|---|---|
| **Webview + React** | ✅ React Flow 12.12 (20.6M/wk) + elkjs or dagre | ✅ xterm.js 6.0 | ✅ @pierre/diffs, @codemirror/merge | ✅ Streamdown 2.7 (Vercel) | ✅ shadcn + its MCP server | ✅ best |
| Webview + Svelte 5 | ✅ Svelte Flow (official xyflow) | ⚠️ thin wrapper by hand | ⚠️ CodeMirror merge | ⚠️ svelte-streamdown (community) | ✅ shadcn-svelte | ⚠️ runes vs Svelte 4 mix-ups; has an official MCP server |
| Webview + Vue 3 | ⚠️ Vue Flow (community, last release 2026-01) | ⚠️ by hand | ⚠️ CodeMirror merge | ⚠️ small community packages | ✅ shadcn-vue | ⚠️ below React in evals |
| Webview + Solid | ❌ one-person port, 1.7k/wk | ❌ wrappers stale, by hand | ⚠️ CodeMirror merge | ❌ none; build it | ⚠️ unofficial shadcn ports | ❌ no evals; footguns that compile |
| GPUI + gpui-kit | ❌ none | ❌ Zed's is GPL; one 0.1 crate | ✅ read-only diff | ✅ markdown, streaming example | ✅ dock, settings, table | ❌ little training data |
| Freya 0.5 | ❌ none | ✅ freya-terminal | ❌ none | ✅ freya-markdown | ⚠️ small | ❌ |
| egui 0.37 | ⚠️ egui_graphs (layered layout) | ❌ stale 0.1 | ❌ none | ✅ egui_commonmark | ⚠️ weak rich text | ⚠️ |

## Shells

| Shell | Latest | Verdict |
|---|---|---|
| **Tauri 2** | 2.12.5 (2026-10-21) | **Keep.** Linux is the risk: WebKitGTK issues [#10702](https://github.com/tauri-apps/tauri/issues/10702) (Wayland), [#5143](https://github.com/tauri-apps/tauri/issues/5143) (blank screen, open since 2022) and [#9304](https://github.com/tauri-apps/tauri/issues/9304) (NVIDIA) are still open. |
| Electron | 45.2.3, Chromium 156 | **The fallback** if Linux fails. The same React code moves over. The cost is size and memory, and the socket client moves into Node. |
| Tauri 3 + CEF | 3.0.0-alpha.6 (2026-11-06) | **Not yet.** CEF on Linux has open bugs: 100% CPU at idle ([#16189](https://github.com/tauri-apps/tauri/issues/16189)), lost keyboard input, broken drag-drop and broken AppImage builds. Plugin APIs break. Revisit in 2027. |
| Wails v3 | 3.0.0 stable (2026-10-26) | No. Its backend is in Go, which would make Go a third language. |
| Electrobun | 2.0.6, daily betas | No. It moves too fast for one reviewer, and adds Bun to the stack. |
| Dioxus desktop | 0.7.10 | No. It uses the same wry webview as Tauri, with less ecosystem. Its native renderer (Blitz) is beta. |

1. Tauri's own docs say events are "not designed for low latency or high throughput"; Channels are "fast and deliver ordered data" ([calling-frontend](https://v2.tauri.app/develop/calling-frontend/)). Agent logs and pty output go over Channels.
2. tauri-specta is still a release candidate (rc.26), and its Tauri 3 support is unconfirmed.
3. The October plan was to escape WebKit problems through CEF without leaving Tauri. That still holds, but not before 2027.

## Native Rust

1. **GPUI + gpui-kit:** the best native option, but worse than the survey said.
   - The official `gpui` crate on crates.io is a year stale (0.2.2, 2025-10-22).
   - gpui-component was renamed gpui-kit (0.9.0, 2026-11-02). It pins `gpui-pre` snapshots of Zed that one person republishes.
   - It has no graph widget. Zed's terminal is GPL.
   - Windows screen-reader support is still missing ([zed#41138](https://github.com/zed-industries/zed/issues/41138)).
2. **Freya 0.5.1:** a surprise, with a terminal, code editor and markdown. But it is pre-1.0 with one maintainer, and has no graph or diff.
3. **egui 0.37:** egui_graphs has a layered layout, so "no layout" was not strictly true. Rich text and chat feel weak. It is the right pick only if the canvas is the whole app.
4. **Not viable for this widget set:** Slint (attribution or GPL licence, no widgets we need), Iced (no accessibility), Makepad (its own DSL, 2.0 rewrite unpublished), Xilem, Floem and Vizia.
5. Layout can run in SK either way. That closes the layout gap for native UIs, but not the drawing and interaction gap.

## The frontend

### Ranking

| # | Framework | Why |
|---|---|---|
| 1 | **React 19.3** | The only one with a maintained package for every need. Best odds that agents write it right. The maintainer reviews it fluently. React Compiler 1.0 removes the memo footguns. |
| 2 | Svelte 5.57 | Official Svelte Flow and shadcn-svelte, plus the best AI tooling outside React (llms.txt and an MCP server that checks for common AI mistakes). It loses on runes vs Svelte 4 mix-ups, thin terminal and markdown wrappers, and less review fluency. |
| 3 | Vue 3.5 | A big, boring ecosystem. Vue Flow is one maintainer with no release since January. Vapor is still a release candidate. |
| 4 | SolidJS 1.9 | See below. |
| — | Preact 11 | No gain inside a desktop app, and compat with React Flow and Radix is unverified. |
| — | Leptos, Dioxus web, Yew, Sycamore | The graph crates are toys, with no xterm, Monaco or markdown crates. Tauri IPC is hand-written wasm-bindgen calls. All are pre-1.0. |

### SolidJS, judged harshly

**For it:**
1. It is the fastest mainstream framework in js-framework-benchmark: geomean 1.16, against Svelte 1.25, Vue 1.31 and React 1.87 (1.0 = fastest) ([krausest](https://github.com/krausest/js-framework-benchmark)).
2. It is JSX + TS, so it looks like React. Its fine-grained signals suit live items moving through a state machine.
3. TanStack Query, Virtual and Router have Solid adapters, and there is an official Tauri template.
4. State of JS 2025 ranks it highest in satisfaction for five years running ([stateofjs](https://2025.stateofjs.com/en-US/libraries/front-end-frameworks/)).

**Against it:**
1. **The speed doesn't count here.**
   - Kelpie draws 5 to 50 nodes and a virtualised log. Appending rows takes 24.8 ms in Solid and 28.6 ms in React.
   - Layout and indexing already run in Rust, in SK.
   - The one big gap is swapping rows (13 vs 87 ms), and kelpie doesn't do that.
2. **The ecosystem is splitting in two.**
   - Stable is 1.9.18. 2.0 is at rc.21 (2026-11-04), and the libraries' new work is on rc and alpha tags: solid-query 6 rc, Kobalte 2 alpha, solid-flow 1.0-next.
   - 2.0 renames or removes core APIs ([migration](https://github.com/solidjs/solid/blob/next/documentation/solid-2.0/MIGRATION.md)): Suspense, createResource, onMount, classList and `use:`.
   - So kelpie would build either on a 1.x line its libraries are leaving, or on a pre-release stack that agents haven't seen.
3. **Kelpie would build or own these by hand:**
   - The graph: `@dschz/solid-flow` 0.2.6 has 71 stars, 1.7k downloads a week and one author, and the 0.2 line gets fixes only. React Flow has 20.6M a week.
   - Components: no official shadcn port. `solid-ui` was last pushed in 2026-02, and Kobalte is pre-1.0. shadcn's MCP server and skills don't apply.
   - A streaming markdown renderer: nothing like Streamdown exists for Solid.
   - Wrappers for Monaco and xterm: `solid-monaco` gets 250 downloads a week and `solid-xterm` gets 10, and both are stale.
4. **Agents would make silent mistakes.**
   - Destructuring props breaks reactivity ([docs](https://github.com/solidjs/solid-docs)). An early `return` in a component breaks it too.
   - Both compile and run, so the reviewer has to catch them.
   - No LLM eval includes Solid, and it has no MCP server.
5. **One person carries it.** ryansolid has 1,482 commits; the next contributor has 33.
6. **Rin's own tooling is React's:** the react-doctor, zustand, TanStack and shadcn skills.

**What would change this:** all of the following being true at once. None of them is today.
1. Solid 2.0 is stable.
2. solid-flow 1.0, Kobalte 2.0 and the TanStack Solid adapters are out of rc and alpha.
3. A Streamdown-class renderer exists for Solid.

A side project would be a better place to try Solid than kelpie.

## Meta frameworks: none

1. Tauri does not support SSR, and recommends Vite ([frontend](https://v2.tauri.app/start/frontend/)).
2. Kelpie has no server, no SSR and no SEO, so a meta framework adds only config, plus server/client boundary mistakes for agents to make.

| Option | Verdict |
|---|---|
| **Vite 8 + TanStack Router** | **Yes.** Client-first, type-safe search params, and shadcn has an install guide for it. |
| TanStack Start | Stable, but its SPA mode is overhead here, and Tauri has no guide for it. |
| SolidStart, Nuxt | Built for SSR. No. |
| SvelteKit (static adapter) | The only meta framework Tauri ships as a template. Fine if we used Svelte. |
| Next.js static export | Needs `output: 'export'` and has a list of unsupported features. No. |

## What would change the shell verdict

1. **The Linux spike fails** on WebKitGTK, X11 or Wayland, or NVIDIA: move to Electron, keeping the React code.
2. **Tauri 3 + CEF ships stable** with clean Linux behaviour: it beats Electron on size.
3. **GPUI or gpui-kit gains** a maintained graph widget and a permissively licensed terminal, and the official crate catches up with Zed's main.
4. **A native framework reaches 1.0** with screen-reader support verified on all three OSes.

## Changes to the plan

1. The tracer bullet in `backend.md` (Next steps, 2) also runs on Linux. It should show:
   - React Flow with live nodes
   - xterm.js with fast output
   - streaming markdown

   Test it on Ubuntu and Fedora, under X11 and Wayland, and on NVIDIA.
2. Agent logs and pty output go over Tauri Channels.
3. Turn on React Compiler (`babel-plugin-react-compiler` 1.0) from the start.
4. Plan for tauri-specta to stay a release candidate. Generating TS types straight from the `protocol` crate is the fallback.

## Unconfirmed

1. Official memory figures for any shell.
2. A Tauri 3 stable date.
3. Real screen-reader quality in GPUI.
4. Whether the js-framework-benchmark results file matches its chrome156 release.
5. Any LLM eval on current models across frameworks. The only one found is Web-Bench ([arXiv 2505.07473](https://arxiv.org/html/2505.07473v1)), from the Claude 3.7 era, where React leads for all six models tested.
