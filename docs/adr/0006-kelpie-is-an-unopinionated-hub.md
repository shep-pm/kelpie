# Kelpie is an unopinionated hub

Decided on 2026-11-09 with the maintainer, in the UX grilling (`docs/design/ux.md`).

Kelpie assumes nothing about the harness, model, forge or indexer a person uses. Each sits behind an adapter, and every agent-facing feature is built against the adapter interface rather than one harness.

- **Parity:** a feature reaches every harness in the same change, or names the open issue that brings each one still missing.
- **Settings over assumptions:** wherever people would choose differently (harness, model, effort, output style, sandbox rules, skills, instructions), a setting holds the choice. Kelpie ships defaults, never requirements. The PM defaults to Opus 5.5 at high effort, and a project can change that.
- **A project carries its own agent setup:** vendored skills and its own instructions. A harness's global folder (`~/.claude`, `~/.codex`) is read only when the person turns it on.
- **Pushback:** when anyone, the maintainer included, asks for something that ties kelpie to one harness, model, tool or workflow, Claude, the PM or the worker in front of them says so and offers the unopinionated version first.

## Why

SK was built around Claude Code. Codex and local models came later, and features kept landing for Claude only: Codex's transcript counts (SK#1138) and the browser for Codex and goose workers (SK#863, SK#864) followed after, and goose as a fourth harness is still open (SK#784). SK needed a CI check and an issue rule ("Harness reach") to restore parity after the drift. Its shipped agents also name models and a script only the maintainer has (`docs/backend.md`, For other people).

## Considered options

- **Claude Code first, the rest later.** Fastest to the first tracer bullet, and how SK went. Turned down: each feature gets shaped around one harness, and parity becomes a backlog.
- **Adapter-first.** Slower at the start, since the interface has to exist before a second harness proves it. Chosen.

## Consequences

- The PM tracer's stream reader (#12) is the Claude Code adapter behind a harness interface, not code that belongs to the PM.
- A second harness, pi on a local model, joins the PM in Phase 1, so the interface is proven by two harnesses before anything else builds on it.
- Parity is held from the first adapter rather than restored later. An issue touching one adapter says which harnesses it reaches, and a CI check is designed alongside the adapter interface.
- Settings and the agent editor store harness, model and effort per agent. No model is named in kelpie outside a default.
- Kelpie's own stack is out of scope. Tauri, React, crabpot and shep's crates are kelpie's choices, recorded in their own ADRs.
