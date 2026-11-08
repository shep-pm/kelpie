# kelpie

A desktop app where people and agents plan, run and follow work on a codebase. MIT OR Apache-2.0.

Kelpie owns every process it starts: when it quits, crashes or is killed, they all stop, and the sessions resume on the next launch. The PM agent is the maintainer's one conversation, and the app is its front end. SK (shep-pm/shep-kelpie) builds kelpie until kelpie can build itself, then SK is archived.

## Rules that are never broken

From `docs/adr/0001-kelpie-owns-its-whole-process-tree.md`:

- Every process kelpie starts goes through the engine's process tree. No bare `std::process::Command::spawn`, no Tauri shell plugin, no spawning from the webview.
- Nothing detaches: no `setsid`, double fork, `nohup` or `disown`, and nothing installed into launchd, systemd or login items.
- Every child runs in its own process group (a job object on Windows) with `KELPIE_RUN=<run id>` in its environment.
- When kelpie quits, every process stops. Sessions are what persist: the next launch resumes them, and workers come back paused.

## Commands

No code yet. The Phase 0 workspace adds the app and the Rust crates, and their commands go here (`docs/process-and-sk.md`, Order of work).

- Module map of SK: see `README.md`, Build the map. Run Python with `-I`.
- CI runs on the maintainer's self-hosted runner pool, not GitHub's: every job uses `runs-on: [self-hosted]`, or `[self-hosted, X64]` to pin it to the x86 box. The runner image has no node or system libraries, so jobs install what they need with `setup-*` actions and apt.

## Where things live

- `docs/process-and-sk.md` is the current plan: the process tree, sessions, updates, and what to lift from SK.
- `docs/backend.md` is partly replaced by it. Each replaced part is marked.
- `docs/gui-framework.md`: why Tauri 2 + React.
- `docs/goals.md` and `docs/journey.md`: the two goals, and a log of every step of starting the project.
- `map/`: builds SK's module map.
- `docs/adr/`: decisions. Read the ones touching your area first.
- `.claude/skills/`: read `tauri` before touching `src-tauri/`, `tauri.conf.json`, capabilities or any `invoke` call, since Tauri 2 differs from Tauri 1 in ways models get wrong. Read `lift-from-sk` before bringing in any SK code, and `zustand-subscription-patterns` before wiring a component to a store.
- `docs/lifted.md`: every chunk lifted from SK, with what it handles and what was dropped.

## Style

- When writing about shep, use shep's vocabulary: a `sheep` is one managed process, the plural is `flock`, dogs are plugin processes, and the daemon is only ever "the shepherd". Kelpie's own processes are not sheep.
- Public text in other shep-pm repos never names kelpie while this repo is private.
- Committed text says "the maintainer", never a name, and uses repo-relative paths.

## Commits and pull requests

- Conventional subjects: `type(scope): summary`, with types `feat` `fix` `perf` `refactor` `docs` `test` `ci` `chore` `style`, and `!` on the commit that breaks something. `.githooks/commit-msg` checks locally (run `git config core.hooksPath .githooks` once per clone) and `.github/workflows/commits.yml` checks every pull request.
- Pull request titles are conventional too. `merge_commit_title` is set to the PR title, so a merge commit's subject is the title.
- Merge with a merge commit, never a squash.
- One commit per item. Bodies carry the full reasoning.

## Agent skills

### Issue tracker

GitHub Issues on `shep-pm/kelpie`, via `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default roles, label string equal to role name. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` at the root and ADRs in `docs/adr/`, both created when a term or decision is first settled. See `docs/agents/domain.md`.
