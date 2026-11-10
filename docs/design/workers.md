# Workers

Phase 2's worker sessions (`docs/process-and-sk.md`, Order of work): an agent that builds one ticket in its own worktree, inside its own sandbox, while the maintainer watches and steers it from the same chat view the PM uses. It places itself in `docs/design/ux.md` (Workers, Sandbox, Attention) and lifts most of its parts from SK, row by row from `docs/lift-plan.md` (Phase 2). Terms are in `GLOSSARY.md`.

**Status:** draft for the maintainer, written on 2026-11-09. No issue is filed from it until it is confirmed.

## Use case

Today SK's workers build kelpie headless: the maintainer sees a pull request at the end, and steers through rulings and `shep kelpie tell`. Kelpie can build itself only once it runs workers of its own.

1. A ticket is ready. The maintainer clicks Start on it, or the PM starts it.
2. Kelpie makes the ticket a worktree and starts a worker there, on the harness and model the project's settings name.
3. The maintainer opens the building ticket and sees the worker's stream, its diff so far and its processes. A Note reaches it at its next tool boundary. Stop and tell interrupts the turn.
4. The worker commits in its worktree and ends its turn. Kelpie pushes the branch and opens the pull request, so the worker never holds a credential.
5. When kelpie quits mid-turn, the worker comes back Paused on the next launch and resumes only when the maintainer says so.

Why it matters: the maintainer can watch and redirect a worker instead of reading its result afterwards, and every harness kelpie has can do the building.

## Design

### Flow

```text
ticket (Ready)
  Start (the maintainer, or the PM's start_worker tool)
    take a slot                    held while the 5-hour or week window is full
    prepare the worktree           a clone with its own git dir, from origin/main
    Sandbox::start                 the worker's policy
    session in the process tree    harness and model from the worker agent's settings
      stream into the chat view    Note, Stop and tell
      tool calls judged            confine and guard, in the ACP permission handler
      kelpie lease run cargo test  waits its turn behind other workers
    turn ends
      fence check                  the diff against a trusted commit
      kelpie pushes, opens the PR  through the forge adapter
    slot freed, ticket to Review
quit mid-turn: process stops, worker Paused, slot freed
```

### Starting one

In Phase 2 a worker starts on request: the Start button on a ticket, or the PM's `start_worker` tool (`docs/design/kelpie-tools.md`), which needs no approval: starting work on a Ready ticket is the PM's job. Phase 3 adds the board's dispatch, which starts Ready tickets on its own. Both go through one function, so dispatch is a caller, not a second path.

The worker is an agent file like any other (`docs/design/settings.md`): a built-in `kelpie:worker` that a project or the maintainer can `extends` or replace, with its harness, model, effort and instructions. Kelpie ships the default and requires none of it (ADR 0006).

### The worktree

Lifted whole from SK's `crates/process/src/worktree/` and `crates/worker_folder/`:

1. Each ticket gets a clone with its own git dir beside the checkout, borrowing its objects, on a branch cut from the latest `origin/main`. Preparing is idempotent, so a resumed worker finds its worktree as it left it.
2. Every git command kelpie runs on a worktree names its git dirs instead of finding them, because the worker can plant a `.git` file or hooks there. Reads and writes kelpie makes in a worker's folder refuse links (`worker_folder`).
3. SK's stale lock, stray and loose-object handling come with it, with their tests.

### The worker's policy

Same shape as the PM's (ADR 0005), composed from SK's `WorkerProfile::reach` (`core/profile/mod.rs:278-363`):

| | PM | Worker |
|---|---|---|
| Reads | The project, its worktrees, sibling repos | Its worktree and the project, sibling repos |
| Writes | Its own folder, its harness's config folder | Its worktree, its state folder, its harness's config folder |
| Network | Model host plus the project's allowlist | The same |
| Credentials | Denied | Denied, plus the clone's remote config and push flags |
| Everything else | Kelpie's tools | Kelpie's tools |

Three checks sit on top of the sandbox, all lifted from SK:

1. **Confine** (`core/confine/`): each file tool call is judged against the worktree before it runs. Kelpie calls `judge_path` in process from the ACP permission handler, where SK spawned `kelpie confine` as a hook. Every harness gets it, since every harness asks permission over ACP (ADR 0007).
2. **Guard** (`core/guard/`): each shell command is read and its git calls judged in the same handler. `gh.rs` waits for the GitHub slice.
3. **Fence** (`core/fence/`): when the turn ends, the worktree's diff is checked against a trusted commit, which catches what the sandbox's globs miss. A harness's own files (`.claude/`, `.codex/` and the rest) are refused, with the list supplied by each adapter.

### Pushing and the pull request

The worker commits. Kelpie pushes, outside the sandbox, with the trusted git call, and opens the pull request through the forge adapter (GitHub first, `docs/lift-plan.md`, GitHub). SK's `push_commits` already refuses `Remote::Origin` from a worker, because its credential is scoped to one repo. Kelpie goes one step further: no credential enters the sandbox at all, which matches the PM (ADR 0005).

### Talking to a worker

As `docs/design/ux.md` (Workers) has it:

1. **Note** is queued and delivered at the next tool boundary. A harness that cannot take a message mid-turn gets it when the turn ends, and its row in the harness facts table says so.
2. **Stop and tell** cancels the turn over ACP and sends the message as the next prompt.
3. A note goes into the ticket's record. When it changes the scope, the PM updates the ticket's text. The PM gets one line saying what the maintainer told the worker.

### Slots, pacing and leases

1. **Slots** (`runner/slots.rs`, lifted): a per-project setting caps how many workers run at once, 2 by default. A worker that is Paused or waiting on the maintainer frees its slot (SK ADR 0007).
2. **Pacing** (`core/pacer.rs`, lifted): a harness limit at 100% of its 5-hour or week window holds every new turn until it resets. Keyed by a limit id, so local models need none.
3. **Leases** (`core/lease/counted.rs`, lifted as `kelpie lease run`): a capacity-N lease held by connection liveness, so two workers' `cargo test` runs take turns instead of starving the machine. The capacity per name is a setting. `kelpie lease run` reaches kelpie through a socket in the worker's state folder, one per session, which the worker's policy allows. Kelpie tools go over stdio MCP, but a lease must live exactly as long as the command it wraps.
4. **Flights** (`runner/flight/`, adapted as the worker supervisor): the running set, its news, and its ceilings. A ceiling counts from when the slot was granted, not from the launch.
5. **Usage** (`runner/in_flight.rs`, `core/usage/`): a draft opens when the turn starts and is written once when it ends, from what ACP reports.

### Ending, quitting and resuming

1. A worker ends when its turn ends with the work committed. Its session stays, so a review round in Phase 3 resumes the same conversation.
2. When kelpie quits, every worker's process stops (ADR 0001). On the next launch each worker that was mid-turn is Paused: its worktree and session are kept, and it resumes only when the maintainer says so.
3. A drain for an update lets running turns finish and starts no new ones.

### Seeing the app

A worker cannot open kelpie's window inside its sandbox (`docs/lessons/2026-11-09-sandboxed-agents-cannot-open-the-app.md`). Phase 2 workers test the frontend with Playwright on the Vite page with Tauri's IPC mocked, and a "By hand" box is the maintainer's at review. Driving the real app through a kelpie tool gets its own note.

## Synergy

| Reuses or connects to | How |
|---|---|
| The PM chat (`docs/design/pm-chat.md`) | The same chat view, harness interface, session log and resume. A worker is a session with a different policy and a worktree |
| The process tree (ADR 0001) | Every worker, and every command it runs, is in the tree, so a quit stops them all |
| crabpot (ADR 0005, #33) | `Sandbox::start` with the worker's policy. The PM's policy code grows a second caller, not a copy |
| Kelpie tools (`docs/design/kelpie-tools.md`) | `start_worker` for the PM, and the sandbox request for a worker that needs a domain |
| Settings and agents (`docs/design/settings.md`) | The worker agent, the slot cap, lease capacities |
| The drawer | A worker's processes show in the process panel |
| The board | Phase 3's dispatch calls the same start function |
| Attention (`docs/design/ux.md`) | A Paused worker, a fence refusal or a sandbox request adds to the count |

What it would duplicate: nothing in practice. SK's runner builds kelpie until Phase 3, while the maintainer trials kelpie's Phase 2 workers on a project SK never touches.

## Open for the maintainer

1. **Who pushes.** SK's worker holds a GitHub App installation token scoped to one repo (contents and workflows written, nothing else) and pushes its own branch. This draft has kelpie push instead. To settle before it is confirmed.

Terms to add to `GLOSSARY.md` once confirmed: Worktree, Slot, Lease, Note, Stop and tell.
