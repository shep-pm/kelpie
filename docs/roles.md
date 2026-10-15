# Two possible agent roles

The maintainer asked for two: one that solves SK#897 (starting a project), and one that watches the code map and files refactoring tickets. The question to settle first: new roles, or existing agents with new jobs?

## SK's existing agents

1. Issue writer: turns one request into agent-ready issues; `--interactive` plans with the maintainer.
2. Project manager: runtime only. Reads the board, never code, git or gh.
3. Advisor: the codebase expert on the panel. Read-only, votes blind on rulings.
4. Researcher: everything outside the code, on the panel.
5. Reviewers: one pull request each.
6. Representative: words kelpie's prose for GitHub.
7. Triager (coming, SK#887): first pass on untouched issues.
8. Workers already get the code graph (codebase-memory-mcp) when `code_graph` is on.

## The gap

1. Nobody owns where the code is going. The issue writer forgets after each request, the advisor only votes, the PM never reads code.
2. Both requested roles need that owner: one at a project's birth, one over its life.

## Recommendation

1. One new role, the architect. Not two.
2. The janitor is the architect's recurring job (a survey), not a separate agent. Two agents with opinions about structure and no single owner would pull against each other.
3. The architect owns the target: `docs/design/`, ADRs, the glossary and a target layout.
4. It is read-only like the advisor. It hands plans to the issue writer, which files them as parent and slices.
5. Code wakes it, like the PM: at a project's start, every N merges, or when the map gets worse.
6. SK#897's mechanical steps (repo, `init-shep` setup, GitHub settings, App access, `kelpie add`, settings) become code, a verb. Only the decisions and the design session are the architect's.

## Why not extend an existing agent

1. Advisor: its worth on the panel is an independent vote. If it wrote the target, it would vote on its own plan.
2. Issue writer: it works one request at a time, and keeping the planner apart from the filer keeps issues one pull request in size.
3. PM: it never reads code, by design.

## Cautions

1. The code graph matches Rust calls by name (every `.unwrap()` became a call into `guard/wrap`). The architect needs exact data too: `crate::` references (`map/build.py`), and later `cargo metadata`.
2. A whole-map view overwhelms. The architect's output should be small: one finding, one picture, one proposed slice.
3. A workspace split of SK is hard to reverse, so it starts with an ADR: the crates, the layering, which crates publish to crates.io, and how release-plz and the MSRV check cover them.

## Try before building

1. Write `agents/architect.md` in SK.
2. Run it once on SK as a planning session: survey, the workspace ADR draft, then parent and slices through the issue writer.
3. SK's workers do the refactor. Nobody does it by hand.
4. Build the architect's role into SK's code only once that run shows what it needs.
