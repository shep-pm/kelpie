# Kelpie's UX

The app as a whole, settled with the maintainer on 2026-11-09. Every feature's design note places itself here, and its synergy section points back at this file. It builds on `docs/gui-idea.md` and `docs/goals.md`. Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-09.

Kelpie is an unopinionated hub (ADR 0006): every agent-facing part below works on any harness behind an adapter, and Claude Code is only the first one built.

## Use case

The maintainer is a visual learner. Long chat sessions, walls of text and long todo lists overwhelm, so they end up agreeing to everything. A codebase run by agents then drifts until no person can find their way around it, and getting it back on track costs as many tokens as building it. SK is the example: it grew past the point where the maintainer knows where to start, and they can only give design and direction.

Kelpie is project-forward, not code-forward:

1. The maintainer sees the project as a picture and points at the part they mean.
2. Work is planned, built and followed on that picture.
3. Any worker can be watched and redirected while it runs.

What SK lacks today, and kelpie fixes:

| SK today | Kelpie |
|---|---|
| No UI. GitHub plus `shep kelpie status`, `board` and `rule` | One window per project |
| Rulings reach the maintainer through the log or polling | A count in the rail, the panel and the graph, and the PM says it |
| A working worker can't be seen | Attach to its session, watch it, message it |
| Two PMs: SK's, plus the maintainer's own chat | One PM per project |

## Layout

```
┌──┬──────────────────────────────┬───────────┐
│P │                              │ PM        │
│r │          Graph               │ 🧵 threads│
│o │   (areas, overlays, links    ├───────────┤
│j │    lit by the open ticket)   │ Board     │
│  ├──────────────────────────────┴───────────┤
│  │ Drawer: Processes · Terminal             │
└──┴──────────────────────────────────────────┘
```

1. **Project rail**, left: one icon per project, with a badge counting what needs the maintainer.
2. **Graph**: fills the window.
3. **Panels**, right: PM, Board and Git. Docked like VS Code's secondary sidebar: resizable, stacked or side by side, collapsible to an icon strip, and maximisable to full width for focus. Floating windows were turned down because they cover the graph.
4. **Drawer**, bottom, shared by every panel: Processes (the live tree) and Terminal.
5. **Tray**: the process count and the needs-you count.

Each project reopens with the layout it was left in.

## The graph

1. **Built from the code.** It grows as the code changes, without anyone asking. Code facts come from CMM, run in kelpie's process tree and read through its CLI behind kelpie's own interface. Kelpie's own indexer is expected to replace it soon.
2. **Areas on top.** An area holds rules, mostly folders or path patterns, plus symbols picked by hand. New code matching a rule joins the area. Code matching none goes to **Unsorted**, with a badge, and the PM proposes a home. On a project's first open the PM proposes areas from CMM's folder regions, and the maintainer renames or merges them.
3. **Zoom:** areas, then files, then symbols, about 5 to 9 boxes at each level.
4. **2D first**, with React Flow. A 3D overview may follow, and CMM's CBM Lens (MIT, React Three Fiber) is the candidate to lift. Data and layout assume no renderer.
5. **Branches:** it shows the project's base branch, with a selector for any other. Each open ticket's branch shows as an overlay on the areas it touches.
6. **Editing the graph never edits code.** Moving a node between areas regroups it. A real code change becomes a draft ticket that a worker carries out.

First interactions:

1. Select areas or nodes, then **Comment**: a draft ticket linked to them, and a thread where the PM researches and asks questions.
2. **New area** for a planned feature: a thread where the PM suggests where it fits.
3. A ticket's links light up its areas and nodes.
4. Asking the PM about something sends the selection as context.

## Tickets and links

1. **Board:** a kanban, Draft, Ready, Building, Review, Done. A ticket that needs the maintainer carries a marker. Filter by area. Its own design note comes later.
2. **A ticket holds more than its issue:** links, its threads, notes to its worker. Issues stay the git flow on GitHub.
3. **Links**, most stable first: an area, a symbol, a file, lines within a symbol.
4. **Links follow the code.** A file watcher reindexes what changed. Renamed files are found by git's rename detection, moved symbols through the index, and line ranges are shifted by the diff's hunks. The PM fixes the obvious ones and logs them, and asks about the rest (a function split in two, a file deleted).
5. **Audits:** a link check after every update to the base branch, and a full reindex weekly or on request. Both are code, so no agent tokens are spent unless something is broken.
6. **New GitHub issues** become tickets with a badge. Picking them up is a per-project setting: on for the maintainer's issues, off for other people's.

## The PM and threads

1. One PM per project, one living conversation. Standing decisions sit in a file that survives compaction. It defaults to Opus 5.5 at high effort, since its decisions are the last place to save effort, and a project can pick any harness and model.
2. **Threads**, like Discord threads in a channel. A thread is a fork of the PM session, tied to a ticket or a topic. Its transcript stays with it, and only its outcome returns to the PM, in one line. One level deep.
3. **Token cost is the open worry.** A fork shares the PM's prefix, and the prompt cache lasts an hour on a subscription. The PoC measures it with `usage.cache_creation`. If forks cost too much, a thread starts from a short brief instead, with no change to the UX.
4. **Output style:** kelpie ships a reply style modelled on Claude Code's Concise, adding lists, tables and before and after comparisons. Each adapter renders it in its harness's form (an output style on Claude Code, instructions elsewhere). The PM, threads and workers all use it, and a project can override it.
5. **Readability:** tool calls fold into one-line steps, and a long reply folds after a screenful. Later, a decision or a plan renders as a small graph instead of text.
6. **When resume fails** (the harness has lost the session): kelpie starts a new one seeded with the standing-decisions file and a short summary of the last exchanges, keeps the old transcript above a divider, and says it happened.
7. **Quitting mid-turn** loses the turn. On relaunch the last message shows as interrupted, with a Retry button that sends it again.

## Workers

1. Click a ticket that's building, or its overlay on the graph, to open the worker's session in the same chat view the PM uses: its live stream, its diff so far, its processes, and a message box.
2. **Note** is the default send. Claude reads it at its next tool boundary. **Stop and tell** interrupts the turn.
3. A note goes into the ticket's record. When it changes the scope, the PM updates the ticket's text so review checks the new ask. The PM gets one line saying what the maintainer told the worker.

## Sandbox

Every agent runs in crabpot (ADR 0005): the PM, its threads and every worker, starting with the PM tracer bullet.

1. The PM reads wide (the project, worktrees, sibling repos, secrets denied) and writes only its own folder and Claude's config folder.
2. Every other change goes through kelpie's own tools: tickets, links, areas, notes to workers, GitHub. The PM's permissions are its tool list.
3. When the sandbox refuses something, the agent asks through a kelpie tool and a prompt appears in its panel: *"PM wants to reach docs.rs: Allow once · Allow for this project · Deny"*. The sandbox widens while the session runs. "Allow for this project" is saved to the project's settings, where it can be undone.
4. A per-project setting, **Read anywhere by default**, lifts the read limit (secrets still denied). It is off by default.
5. Windows is unproven: whether the sandbox user's processes stop when kelpie dies is still to be tested.

## Settings and agents

Both get UI in Phase 1. Their own note is `docs/design/settings.md`.

1. **No setting is file-only.** A setting gets UI in the slice that introduces it. Each form is generated from one schema, so form, validation and docs agree.
2. **Stored as JSON** with a `$schema`, in kelpie's own data folder, not the repo, since allowlists and models are personal and a repo can be public.
3. **Layers:** built-in, then global, then project. Each field shows which layer its value comes from, with a reset to that layer. Agent files use the same component, and `extends` is one more layer.
4. **Agent editor:** global agents in Phase 1, per project later. SK's agent format comes over whole with its tests: its frontmatter, `extends` with `kelpie:<name>` built-ins, `_`-prefixed files ignored. A frontmatter form, an MDX body (chelone's components), and a preview of the agent with its `extends` chain resolved.

**An agent's environment** is layered the same way, and kelpie renders it into each harness's format (CLAUDE.md or AGENTS.md, skill folders, MCP config):

| Layer | Source | Default |
|---|---|---|
| 1. The repo | Its instructions, vendored skills, MCP config | Always |
| 2. Kelpie global | Instructions, skills and MCP servers edited in kelpie. Instructions can point at a file the maintainer already keeps | On |
| 3. Kelpie project | The same, per project | Later |
| 4. A harness's global folder (`~/.claude`, `~/.codex`) | Whatever that harness keeps globally | Off, opt in per project |

Kelpie's config is one folder, so it can sit in a synced folder or a dotfiles repo. A skill dependency manager that fills layers 1 and 3 comes later (`docs/skills-managers.md`).

## Attention

One count of what needs the maintainer, shown wherever it applies: the project in the rail, the panel it lives in, the area on the graph. The PM mentions it in its next reply. An OS notification fires only when work is blocked on the maintainer.

## Projects

1. **Adding one:** pick a checkout or clone from GitHub, confirm the base branch, let kelpie index it, and the PM proposes areas.
2. **Base branch** per project, read from the repo and changeable, such as `develop` for Dragonite. Workers branch from it, pull requests target it, and the graph shows it.

## Synergy

| Piece | Reused by |
|---|---|
| The chat view | The PM, threads and workers |
| The stream-json reader (#12) | The PM, threads and workers |
| The process tree (ADR 0001) | Every session and CMM, all shown in the drawer |
| crabpot (ADR 0005) | Every agent session |
| The harness interface (ADRs 0006, 0007) | The PM, threads and workers, on Claude Code and goose from Phase 1 |
| The layers component | Settings, agent files and the agent environment |
| Kelpie's own tools for the PM | The PM, threads, and the graph's and board's actions, which call the same tools |
| SK's work-item loop, lifted in Phase 3 | The board |
| GitHub | Stays the git flow. The board links out to it for what kelpie doesn't show |

It would duplicate CMM's own viewer (kelpie never shows the raw graph) and GitHub's issue page (the ticket holds more and links out).

The order of work changes: the graph becomes slice 2, ahead of the drawer (`docs/process-and-sk.md`).

Hard to reverse, so recorded as ADRs: every agent runs in crabpot (ADR 0005), kelpie is an unopinionated hub (ADR 0006), and running CMM as a second sidecar beside the reaper (written when the graph slice starts).

## Later

1. A 3D overview of the graph.
2. Draft layout mode: drag nodes into a new shape and propose it as a series of refactor tickets, the way SK went from one crate to a workspace.
3. An editor panel (Monaco) that opens a symbol, file or lines from the graph.
4. The Git panel, GitHub Desktop style. The last pillar to be built.
5. Workers' chats in a modal, if watching them in a panel is more than people want.
6. Repos the maintainer doesn't own, with a fork-based flow.
7. Decisions and plans drawn as small graphs in the chat.
8. Kelpie's own indexer in place of CMM.
9. Per-project agents and a kelpie project layer.
10. A skill dependency manager that vendors and locks skills, MCP servers and instructions for every harness.
11. Settings synced across machines.
