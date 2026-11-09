# kelpie

A desktop app where people and agents plan, run and follow work on a codebase. MIT OR Apache-2.0.

Kelpie owns every process it starts: when it quits, crashes or is killed, they all stop, and the sessions resume on the next launch. The PM agent is the maintainer's one conversation, and the app is its front end. SK (shep-pm/shep-kelpie) builds kelpie until kelpie can build itself, then SK is archived.

## Rules that are never broken

From `docs/adr/0001-kelpie-owns-its-whole-process-tree.md`:

- Every process kelpie starts goes through the engine's process tree. No bare `std::process::Command::spawn`, no Tauri shell plugin, no spawning from the webview.
- Nothing detaches: no `setsid`, double fork, `nohup` or `disown`, and nothing installed into launchd, systemd or login items.
- Every child runs in its own process group (a job object on Windows) with `KELPIE_RUN=<run id>` in its environment.
- When kelpie quits, every process stops. Sessions are what persist: the next launch resumes them, and workers come back paused.

## An unopinionated hub

Kelpie is a hub of adapters: harnesses (Claude Code, Codex, pi, goose), the models behind them, forges such as GitHub, indexers such as CMM. It assumes nothing about which a person uses. SK was built for Claude Code first, so Codex and local models kept falling behind until CI forced parity. Kelpie is adapter-first from its first agent-facing line (`docs/adr/0006-kelpie-is-an-unopinionated-hub.md`).

- An agent-facing feature is built on the adapter interface and reaches every harness in the same change, or names the open issue that brings each one still missing.
- Wherever people would choose differently (harness, model, effort, output style, sandbox rules, skills, instructions), a setting holds the choice. Kelpie ships defaults, never requirements.
- A project carries its own agent setup: vendored skills, its own instructions. A harness's global folder (`~/.claude`, `~/.codex`) is read only when the person turns it on.
- **Push back on opinionated asks.** When anyone, the maintainer included, asks for something that ties kelpie to one harness, model, tool or workflow, say so and offer the unopinionated version before building. This holds for Claude, the PM and every worker. Build the opinionated version only once the maintainer confirms it after hearing the alternative.

This covers what kelpie runs for people. Kelpie's own stack is settled in its ADRs.

## Design before building

The maintainer asked for this gate and wants it held even when they push to skip it. Say so plainly when a feature is being rushed.

A feature's issues go `ready-for-agent` only once its design note in `docs/design/<feature>.md` has, confirmed by the maintainer:

1. **Use case:** who does what, and why it matters to them.
2. **Design:** the flow or a UI sketch, its terms in `GLOSSARY.md`, and an ADR where a decision is hard to reverse.
3. **Synergy:** what it reuses or connects to (the PM, sessions, the process tree, the board), and what it would duplicate.

Each issue links its note. Workspace, CI and other plumbing with no behaviour of its own need no note.

## Commands

The Rust workspace is `crates/*` plus `src-tauri`, with `kelpie-engine` and `kelpie-app`. CI runs the four commands below (`.github/workflows/test.yml`), plus `cargo +1.90 check --workspace --all-targets --locked` for the `rust-version` in `Cargo.toml`, plus `pnpm tauri build --no-bundle` in the `app` job.

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `cargo doc --workspace --no-deps --locked`, with `RUSTDOCFLAGS=-D warnings`

Iterate with `cargo test --workspace --lib --bins`.

The web frontend is at the repo root, and `src-tauri` loads it in the Tauri window. pnpm 12 is pinned in `packageManager`.

- `pnpm dev`: Vite on port 1520, `strictPort`, so a taken port fails the run instead of moving.
- `pnpm build`: `tsc -b && vite build`. Output lands in `dist/`.
- `pnpm typecheck`: `tsc -b`.
- `pnpm lint`: `biome check .`.
- `pnpm test`: `vitest run`.
- `pnpm tauri dev`: the Tauri window on the Vite dev server. It runs `pnpm dev` first, then waits on `build.devUrl`.
- `pnpm tauri build`: the release build, then the bundles. `cargo build --release` is not a release build: it leaves Tauri's `custom-protocol` feature off, so the binary loads `devUrl` instead of its own assets.
- `pnpm tauri icon docs/design/system/assets/logos/kelpie-app-icon.svg`: regenerates `src-tauri/icons/` from the app icon.
- Biome covers the frontend only: `biome.json` excludes `docs/`, `map/` and all Markdown, which are prose its formatter would rewrite.
- Module map of SK: see `README.md`, Build the map. Run Python with `-I`.
- CI runs on the maintainer's self-hosted runner pool, not GitHub's: every job uses `runs-on: [self-hosted]`, or `[self-hosted, X64]` to pin it to the x86 box. The runner image has no node or system libraries, so jobs install what they need with `setup-*` actions and apt.

## Where things live

- `docs/design/system/`: the design system. Read its `README.md` and `brand-book.md` before building any UI; `index.html` shows every component and screen. Its tokens are in `src/styles/globals.css`, and `src/components/ui/` is shadcn's base set, added with `pnpm dlx shadcn@4.22.2 add <name>` as shadcn ships it. A slice restyles each one it uses to the brand book.
- `docs/process-and-sk.md` is the current plan: the process tree, sessions, updates, and what to lift from SK.
- `docs/backend.md` is partly replaced by it. Each replaced part is marked.
- `docs/gui-framework.md`: why Tauri 2 + React.
- `docs/goals.md` and `docs/journey.md`: the two goals, and a log of every step of starting the project.
- `map/`: builds SK's module map.
- `docs/adr/`: decisions. Read the ones touching your area first.
- `docs/lessons/`: incidents from SK and its tools, and what kelpie does about each. Read the ones touching your area before designing it.
- `.claude/skills/`: read `tauri` before touching `src-tauri/`, `tauri.conf.json`, capabilities or any `invoke` call, since Tauri 2 differs from Tauri 1 in ways models get wrong. Read `lift-from-sk` before bringing in any SK code, and `zustand-subscription-patterns` before wiring a component to a store.
- `docs/lifted.md`: every chunk lifted from SK, with what it handles and what was dropped.
- `src-tauri/`: the `kelpie-app` crate, `tauri.conf.json`, `capabilities/` and `icons/`. `src-tauri/gen/` is the ACL schema `build.rs` writes, and is ignored.

## Style

- `CMM` is codebase-memory-mcp (github.com/DeusData/codebase-memory-mcp), the code graph tool whose viewer inspired kelpie's graph.
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

Single-context: `GLOSSARY.md` at the root and ADRs in `docs/adr/`, both created when a term or decision is first settled. See `docs/agents/domain.md`.
