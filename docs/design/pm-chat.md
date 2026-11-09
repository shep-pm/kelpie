# The PM chat

Phase 1's tracer bullet (#9, ADR 0004): the maintainer talks to a project's PM in kelpie's window. It places itself in `docs/design/ux.md` (The PM and threads, Sandbox, Settings and agents). Terms are in `GLOSSARY.md`.

**Status:** draft, settled with the maintainer on 2026-11-09, awaiting their confirmation.

## Use case

The maintainer plans a project through one conversation, today held in a terminal, beside SK's own PM. In kelpie:

1. They open kelpie, pick the project's folder once, and talk to its PM in the window.
2. Replies stream in, short, with steps folded, so a long session stays readable.
3. Quitting stops every process. Reopening shows the whole history at once, and the next message carries on the same conversation.
4. The PM runs on whichever harness and model the maintainer picks. Claude Code is first and goose on a local model follows in the same phase (ADR 0006).

## Design

### Flow

```
first launch ── Pick a folder ──> projects.json
send ──> the PM's session starts in the tree, sandboxed ──> events stream into the chat
quit ──> every process stops (ADR 0001)
relaunch ──> history from kelpie's log, no process yet ──> send resumes the session
```

### Window

In Phase 1 the PM fills the window, since there is no graph yet. The docked panels of `ux.md` arrive with the graph in Phase 2.

### Projects

One project, picked as a folder on first launch. `projects.json` holds a list from the start, so the rail adds more later without a migration.

### The harness interface

Kelpie's engine and chat see one stream of harness-neutral events, shaped after ACP's `session/update`, and the PM, threads and workers share it. Behind it sit adapters (ADR 0007):

| Adapter | Harnesses | When |
|---|---|---|
| ACP, through the `agent-client-protocol` crate | Claude Code (`claude-agent-acp`), goose (native `goose acp`), later Codex (`codex-acp`) and any agent in the ACP registry | Phase 1, Claude Code then goose |
| pi over its own RPC | pi | Later, and only until pi's ACP bridge exposes steer, fork and resume |

What the interface carries, with each harness saying which it supports in ACP's capabilities:

- send a message, and stream the reply as text, tool steps and an end of turn
- a note while a turn runs, and an interrupt. A harness that cannot take a note mid-turn holds it until the turn ends, and the chat says so
- resume a saved session after its process stopped
- fork a session, for threads later
- pick model and effort
- usage per turn, including what the prompt cache read and wrote

Each harness runs the person's own install and login. Kelpie never ships a harness or a login of its own. A bridge is a prebuilt executable from the ACP registry, checked against the registry's sha256. Phase 1 pins one version of each in kelpie's code. Updating bridges from the registry, with an alert and a swap between turns, gets its own note later.

goose puts its shell commands in process groups of their own, so stopping a session stops every process carrying its `KELPIE_RUN`, not only the harness's group (ADR 0001). A test holds it.

### History

Kelpie keeps its own log of every session's events, in its data folder. On relaunch the chat reads that log and starts no process. The log also holds what no harness records: notes, interruptions, and the dividers below. The harness's own transcript is only used to resume.

### Switching harness or model

| Change | What happens |
|---|---|
| Model, same harness | The same session carries on with the new model |
| Harness | A session cannot move between harnesses, so kelpie starts a new one seeded with the standing-decisions file and a summary of the recent exchanges. A divider marks it: *"Switched from Claude Code (Opus 5.5) to goose (qwen)"* |

A switch applies from the next message. It shares one path with a failed resume (`ux.md`, The PM and threads), which uses the same seed and a divider of its own. The prompt cache does not carry over, so the first turn after a switch costs more.

### Permissions

crabpot is the boundary (ADR 0005). Inside it the harness runs with its own permission prompts off, so the PM reads freely and writes only its own folder. The only prompt the maintainer sees is a sandbox request (`ux.md`, Sandbox). Kelpie's own tools (tickets, links, areas) come with the board.

### The PM's instructions

The PM is an agent file: a built-in `kelpie:pm`, which the maintainer extends in the agent editor. It is SK's `agents/pm.md` trimmed to what Phase 1 has: the standing-decisions file and the reply style, with no board.

### Reading the chat

- Each PM reply renders as Markdown while it streams. Tool calls fold into one-line steps, and a reply folds after a screenful.
- Each turn ends with a one-line footer: tokens in and out, cache read and written, and which cache lifetime applied. Every turn's usage is logged too.
- A turn lost to a quit shows as interrupted, with Retry.

### Threads

Not in Phase 1. One fork is tested by hand, and its footer shows what a fork costs, which decides whether threads are forks or start from a brief (`ux.md`, The PM and threads). Threads arrive with the board, since most are tied to a ticket.

## Synergy

| Piece | Reused by |
|---|---|
| The chat view | Threads and workers |
| The harness-neutral events and kelpie's session log | Threads and workers, on every harness |
| The process tree (ADR 0001) and crabpot (ADR 0005) | Every session |
| The settings page | The PM's harness, model, effort and reply style |
| The agent editor | The PM itself, as `kelpie:pm` |
| SK's parity matrix (`docs/parity.toml`, checked in CI), lifted | Holds every harness to the same features (ADR 0006) |
| The ACP registry | Installing bridges now, and any registry agent later |
| SK's Claude Code facts (`docs/facts`), lifted | The Claude Code bridge's settings, login and config folder |

It duplicates SK's PM, which keeps running for SK until kelpie builds itself. SK's `Agents` port runs one call to its end, so the live chat needs a new interface rather than that trait.
