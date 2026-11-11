# Workers

Phase 2's worker sessions (`docs/process-and-sk.md`, Order of work): an agent that builds one ticket in its own worktree, inside its own sandbox, while the maintainer watches and steers it from the same chat view the PM uses. It places itself in `docs/design/ux.md` (Workers, Sandbox, Attention) and lifts most of its parts from SK, row by row from `docs/lift-plan.md` (Phase 2). Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-09.

## Use case

Today SK's workers build kelpie headless: the maintainer sees a pull request at the end, and steers through rulings and `shep kelpie tell`. Kelpie can build itself only once it runs workers of its own.

1. A ticket is ready. The maintainer clicks Start on it, or the PM starts it.
2. Kelpie makes the ticket a worktree and starts a worker there, on the harness and model the project's settings name.
3. The maintainer opens the building ticket and sees the worker's stream, its diff so far and its processes. A Note reaches it when its turn ends. Stop and tell interrupts the turn.
4. The worker commits and pushes its branch with a token that reaches that one repo alone, and writes the pull request's title and body to a file. When the turn ends, kelpie opens the draft pull request from it.
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
      git push                     with its repo-scoped App token, judged by guard
      title and body to a file     the hand-over
    turn ends
      fence check                  the branch's diff against a trusted commit
      kelpie opens the draft PR    from the hand-over file
    slot freed, ticket to Review
quit mid-turn: process stops, worker Paused, slot freed
```

### Starting one

In Phase 2 a worker starts on request: the Start button on a ticket, or the PM's `start_worker` tool (`docs/design/kelpie-tools.md`), which needs no approval: starting work on a Ready ticket is the PM's job. Phase 3 adds the board's dispatch, which starts Ready tickets on its own. Both go through one function, so dispatch is a caller, not a second path.

The worker is an agent file like any other (`docs/design/settings.md`): a built-in `kelpie:worker` that a project or the maintainer can `extends` or replace, with its harness, model, effort and instructions. Kelpie ships the default and requires none of it (ADR 0006).

### The worktree

Lifted whole from SK's `crates/process/src/worktree/` and `crates/worker_folder/`:

1. Each ticket gets a clone with its own git dir in kelpie's data folder, as SK keeps them in its own home (`home/paths.rs:165`), borrowing the checkout's objects, on a branch cut from the latest `origin/main`. Preparing is idempotent, so a resumed worker finds its worktree as it left it.
2. Every git command kelpie runs on a worktree names its git dirs instead of finding them, because the worker can plant a `.git` file or hooks there. Reads and writes kelpie makes in a worker's folder refuse links (`worker_folder`).
3. SK's stale lock, stray and loose-object handling come with it, with their tests.

### The worker's policy

Same shape as the PM's (ADR 0005), composed from SK's `WorkerProfile::reach` (`core/profile/mod.rs:278-363`):

| | PM | Worker |
|---|---|---|
| Reads | The project, its worktrees, sibling repos | Its worktree and the project, sibling repos |
| Writes | Its own folder, its harness's config folder | Its worktree and its clone's git dir (less `CLONE_DENY`: config, hooks and the rest), its state folder, its harness's config folder |
| Network | Model host plus the project's allowlist | The same, plus `github.com` and `api.github.com` to push (`profile/mod.rs:186`) |
| Credentials | Denied | The maintainer's denied. Its own repo-scoped App token, in a `gh` folder of its own |
| Everything else | Kelpie's tools | Kelpie's tools |

Three checks sit on top of the sandbox, all lifted from SK:

1. **Confine** (`core/confine/`): each file tool call is judged against the worktree before it runs. Kelpie calls `judge_path` in process from the ACP permission handler, where SK spawned `kelpie confine` as a hook. Every harness gets it, since every harness asks permission over ACP (ADR 0007). Workers run in their harness's prompting mode (Claude Code's `default`, goose's `approve`), so every tool call reaches the handler, and a refusal names its reason on the refused step.
2. **Guard** (`core/guard/`, with `local_paths`): each shell command is read and its git calls judged in the same handler. `gh.rs` waits for the GitHub slice, and until it lands every `gh` command is refused.
3. **Fence** (`core/fence/`): when the turn ends, the branch's diff is checked against a trusted commit, which catches what the sandbox's globs miss. A harness's own files (`.claude/`, `.codex/` and the rest) are refused, with the list supplied by each adapter.

### Pushing and the pull request

As in SK, for a worker on the App's token (`HAND_OVER`, `core/profile/mod.rs:44`): the worker pushes its own branch, and kelpie opens the pull request.

1. Kelpie's GitHub App mints an installation token for the one repo, with `Grant::Worker` (`adapters/github/api.rs`): contents written, and workflows where the installation holds it. No pull request, comment, review, approval or merge.
2. It is minted fresh before each turn (`github/tokens.rs:132`), so no refresh timer runs, and a turn's ceiling sits inside the token's hour.
3. The token goes into a `gh` config folder of the worker's own (`core/profile/login.rs`), which git's credential helper answers from. No variable carries it: Codex drops variables named like tokens, and a variable shows in every process's environment. The maintainer's own logins stay denied. That helper is a program on the machine, so a `PATH` with no `gh` is read as the same case as no App on the repo: nothing is minted, nothing is written, and the worker's instructions say it cannot push. A turn with no credential takes out whatever the turn before it left in that folder, since the state folder outlives the turn and a minted token is live until its hour is up, and its instructions name which of the three it is (no App on the repo, no `gh` where its git runs, or a remote that is not GitHub), so the maintainer is not sent after a fix that changes nothing.
4. The worker writes the pull request's title, a conventional commit subject, and its body to a hand-over file. When a turn ends with the branch pushed, kelpie opens the draft pull request through the forge adapter and sets its title and body from the file. A file kelpie will not open, for a first line that is no title or a text naming this machine, is told to the worker as a Note so it can rewrite it. The same refusal at the next hand-over is the worker leaving its file as it was, and is a ruling: the refusal becomes the ticket's needs-you line, no slot is held, and kelpie tells it no more until the file changes. The refusal it told is kept in the worker's record, not beside the hand-over file, which sits in the folder the worker writes.
5. Kelpie signs the App's JWT with the system's `openssl` and calls GitHub through curl, both spawned through the process tree. It pushes only to a fork, from outside the sandbox (`push_commits`), since a repo-scoped token cannot reach one.

Other forges bring their own token through the forge adapter. The cost: a token that writes contents can push any branch, `main` included, and a private repo on the free plan has no ruleset. Guard judges every `git push`, but a direct API call is not git. Protecting `main` another way is left for later.

### Talking to a worker

As `docs/design/ux.md` (Workers) has it:

1. **Note** is queued in the ticket's record. Phase 2 delivers it when the turn ends, on every harness: the notes go out together as the next prompt, one to a paragraph, and the worker stays Running with its slot. A worker whose turn ended takes the note as its next turn. A turn that ends held or Paused keeps its notes for the next start. Steering a turn in flight follows per harness (Claude Code's queued second prompt, goose's `_goose/unstable/session/steer`), so the behaviour is the same on each until then and `docs/parity.toml` needs no row.
2. **Stop and tell** cancels the turn over ACP and sends the message as the next prompt. The cancelled turn's end settles nothing, so the worker stays Running and keeps its slot, and that end starts the new turn. A Stop before it keeps the message as a note, and a second Stop and tell before it goes out in the same prompt. **Stop** with no message cancels the turn and leaves the worker Paused, as the turn ceiling does, and the maintainer resumes it by hand.
3. A note goes into the ticket's record. When it changes the scope, the PM updates the ticket's text. The PM gets one line saying what the maintainer told the worker.

### Slots, pacing and leases

1. **Slots** (`runner/slots.rs`, lifted): a per-project setting caps how many workers run at once, 2 by default. A worker that is Paused or waiting on the maintainer frees its slot (SK ADR 0007). Where the cap comes down, the newest holders give their slots back and the oldest keeps its own; a turn already running is never interrupted.
2. **Pacing** (`core/pacer.rs`, lifted): a harness limit at 100% of its 5-hour or week window holds every new turn until it resets. Keyed by a limit id, so local models need none.
3. **Leases** (`core/lease/counted.rs`, lifted as `kelpie lease run`): a capacity-N lease held by connection liveness, N at least 1, so two workers' `cargo test` runs take turns instead of starving the machine. The capacity per name is a setting. `kelpie lease run` reaches kelpie through a socket in the worker's state folder, one per session, which the worker's policy allows. Kelpie tools go over stdio MCP, but a lease must live exactly as long as the command it wraps.
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
