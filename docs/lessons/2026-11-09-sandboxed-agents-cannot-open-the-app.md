# A sandboxed agent cannot open the app

Nov 9, 2026. Found when the maintainer asked whether a worker had ever opened kelpie's Tauri window, and tested the same day on macOS.

## What happened

1. #8 asked its worker to check that `pnpm tauri dev` opens one window. The worker could not: "the sandbox refuses to bind port 1520, so no window was opened here" (PR #19, Not tested). SK's gate merged it anyway, with that box unticked.
2. The same worker found `pnpm test` exits 1 in the sandbox, because vitest reads above the repo root for its package scope.
3. #13, #14, #30 and #39 carry the same kind of box: "By hand with `pnpm tauri dev`".

## The test

A release build of `main` (`pnpm tauri build --no-bundle`), started inside the exact `srt` profile SK wrote for #26's worker, with its own folder added to the read list. A window counted only when CoreGraphics listed it on screen, and a page only when a capture of that window showed the "Kelpie" heading.

| Run | Result |
|---|---|
| Outside the sandbox, as a control | Window on screen, page drawn |
| The worker's profile as written | The process runs, never shows a window. `com.apple.hiservices-xpcservice` and `com.apple.windowserver.active` are refused |
| Bind 127.0.0.1:1520 | `Operation not permitted` |
| Bind with `network.allowLocalBinding: true` | Binds |
| `network.allowMachLookup: ["*"]` | Window on screen and page drawn. The file and network rules still hold |
| `allowMachLookup` naming `com.apple.windowserver.active`, `com.apple.CARenderServer` and `com.apple.windowmanager.server` | The fewest names that put a window on screen, but its page stays blank |
| `allowMachLookup` naming all 41 services the app looked up under `"*"` | Still blank. What is left refused is `file-issue-extension` and `mach-issue-extension`: WebKit handing sandbox extensions to its helper processes, which `srt` has no setting for. Why `"*"` gets past it is not known |

Not tested: Linux, Windows, and whether the webview reaches the Vite server at `localhost:1520` in dev mode, where the profile denies `localhost` outright. `srt`'s README says `allowLocalBinding` also lets a process reach loopback ports directly on macOS.

## Why `"*"` is not the fix

It lets everything the agent runs look up any macOS service: the pasteboard, accessibility, input events, Launch Services, the privacy database. An agent that can do that can reach out of its sandbox through other apps, so it is a hole, not a widening.

## Lessons for kelpie

1. An agent never opens the app inside its own sandbox. Kelpie starts the build in a fence of its own, with the agent's file and network rules and the window services open, and the agent drives it and reads screenshots through a kelpie tool. SK already does this for the browser: Chromium cannot start in its sandbox, so SK runs Playwright outside it and filters what the worker may ask.
2. A check the builder cannot run is not the builder's check. An issue's "By hand" box is the maintainer's at review until that tool exists, and the issue says so.
3. A gate does not merge past an unticked box silently. #19 named the gap, and the merge still read "every gate passed".
4. Most of the frontend needs no window. Playwright on the Vite page with Tauri's IPC mocked runs in any sandbox and in CI.
5. Each sandbox backend gets this test before kelpie claims an agent can see the app: crabpot's on every platform (ADR 0005), and `srt`'s while SK builds kelpie.

## How to run it again

1. `pnpm tauri build --no-bundle`, outside any sandbox.
2. Copy the worker's `srt` settings, add the build folder to `filesystem.allowRead`, and start `target/release/kelpie-app` under `srt -s <settings> -c`.
3. List windows with `CGWindowListCopyWindowInfo` and check `kCGWindowIsOnscreen`, then `screencapture -l <window id>` and look for the heading.
4. `log stream --predicate 'eventMessage CONTAINS "kelpie-app" AND eventMessage CONTAINS "deny"'` shows what the sandbox refuses. Write `/usr/bin/log` in full under zsh, where `log` is a builtin, and pass each service name as its own argument: zsh does not split an unquoted variable into words.
