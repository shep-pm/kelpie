# Every agent runs in crabpot

Decided on 2026-11-09 in the UX grilling (`docs/design/ux.md`, Sandbox). Amended the same day so its rules name no harness (ADR 0006).

Every agent session kelpie starts runs inside a crabpot sandbox (shep-pm/crabpot): the PM, its threads and every worker. crabpot is a Rust library, so the engine wraps each agent's command with `Sandbox::start` and `Sandbox::command`, and the process tree spawns the result as it spawns anything else (ADR 0001). Starting with the PM tracer bullet, not later.

The PM reads wide and writes narrow:

- **Reads:** the project, its worktrees and sibling repos, with secrets denied.
- **Writes:** its own folder and its harness's config folder only.
- **Changes everything else through kelpie's own tools:** tickets, links, areas, notes to workers, GitHub. The PM never holds a GitHub token, and its permissions are its tool list.
- **Network:** its harness's model host plus an allowlist: Anthropic for Claude Code, or the host of a local model, `localhost` included, for goose. The host comes from the session's harness and model settings, not from kelpie's code. crabpot's `--control-fd` widens it while the session runs, once the maintainer approves a domain.

Workers get the same shape, with write access to their own worktree.

## Considered options

- **Sandbox the workers only.** Less friction for the PM, which handles more state than a worker. Turned down because a PM built unsandboxed grows habits the sandbox later breaks, and nobody knows what it touches until that is taken away. Changing state through kelpie's tools removes most of the friction.
- **Sandbox the PM later, from Phase 2.** Same objection, paid later.
- **Containers.** Heavier, and Docker would be one more thing that runs outside kelpie's tree.

## Consequences

- Kelpie's `main` calls `crabpot::run_from_args()` first, before Tauri, since crabpot runs the host binary again as its helper on Linux and Windows.
- crabpot's proxy lives inside kelpie until each `Sandbox` drops, so it stops when kelpie stops.
- crabpot replaces SK's srt glue (fence, srt, confine, about 6k lines). SK's escape tests came over with the PM's sandbox (`crates/kelpie-engine/tests/sandbox/escapes.rs`), tried against the PM's policy. The fence and `confine` come with workers, and so do the escapes that need one.
- Linux hosts on Ubuntu 24.04 or later need crabpot's AppArmor profile, naming kelpie's own binary as the helper.
- **Open risk on Windows:** the command runs as a separate sandbox user, through `CreateProcessWithLogonW`. srt-win's runner holds a kill-on-close job over the command, but the runner is created by the seclogon service and may sit outside kelpie's job. Whether it dies when kelpie dies has to be tested before Windows is claimed.
- The sweep must still find sandboxed processes by `KELPIE_RUN`, which a test confirms on each platform.
- A project whose folder is the home folder, or a folder above it, is refused when its session's sandbox starts, whichever way *Read anywhere by default* is set, whether it reaches the home folder by name or through a link: the guard compares folders as crabpot matches them, with their links resolved. The home folder holds every harness's login and every secret, so a sandbox over it would read them along with the project; the person moves the project into a folder of its own.
