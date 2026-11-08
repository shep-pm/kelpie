# Journey

Every step of starting kelpie (the app), in order. Who: `maintainer`, `by hand` (an assistant session doing it directly), or `SK` (shep-kelpie doing it).

| # | Date | Step | Who | Could SK have done it? |
|---|---|---|---|---|
| 0.1 | 2026-10-14 | Indexed SK into codebase-memory-mcp and ran its viewer | by hand | No verb for it; workers index their worktree only when `code_graph` is on |
| 0.2 | 2026-10-14 | Built the module map (`map/`) | by hand | No |
| 0.3 | 2026-10-14 | Created the private repo shep-pm/kelpie with `gh repo create` | by hand | No. Skipped the org's `init-shep` skill, so no licences, CLAUDE.md, CI, commit hook, ruleset or labels |
| 0.4 | 2026-10-14 | First commit pushed straight to `main`, no pull request | by hand | No |
| 0.5 | 2026-10-14 | Filed SK#897, the parent for starting a project from nothing | by hand | Partly: `shep kelpie issue` |
| 0.6 | 2026-10-14 | Parked the notes (`docs/goals.md`, this file, `docs/roles.md`) through a pull request | by hand | No |
| 0.7 | 2026-10-23 | Decided the backend, the repo split and the stack (`docs/backend.md`) | maintainer, by hand | No: the maintainer's call |
| 0.8 | 2026-10-23 | Filed SK#1236, SK's own socket, and listed it in shep#1104 | by hand | Partly: `shep kelpie issue` |
| 0.9 | 2026-11-08 | Re-checked the shell and frontend (`docs/gui-framework.md`) and decided the process tree and SK's role (`docs/process-and-sk.md`) | maintainer, by hand | No: the maintainer's call |
| 0.10 | 2026-11-08 | Filed shep#1616, an opt-in kill-on-close job in `shep-tree` | by hand | Partly: `shep kelpie issue` |
| 0.11 | 2026-11-08 | Redid step 0.3 with `init-shep`, through a pull request | by hand | No: nothing in SK runs `init-shep` |

From 0.6 on, changes here went through pull requests. Since 2026-11-08, docs and research may go straight to `main`.
