# Settings and agents

Phase 1's settings page and global agent editor. It places itself in `docs/design/ux.md` (Settings and agents) and serves `docs/design/pm-chat.md` (Switching harness or model, the PM's instructions). Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-09.

## Use case

1. The maintainer picks the PM's harness, model and effort in a form, never in a file, and the next message uses them.
2. They give kelpie a harness's login once, and never see it again in a file.
3. They tune the PM's instructions by extending `kelpie:pm` in an editor, and see the whole agent with its chain resolved before saving.
4. They keep kelpie's config in a dotfiles repo or a synced folder, and a second machine picks it up.
5. A setting they changed shows where its value comes from, and goes back to the layer below in one click.

Kelpie ships defaults, never requirements (ADR 0006): every field works empty.

## Design

### Where things live

| Folder | Holds | Synced |
|---|---|---|
| Config, `~/.config/kelpie` on every OS (ADR 0008) | `settings.json`, `agents/`, kelpie-global instructions, skills and MCP servers | Can be, by symlinking the folder |
| Data, Tauri's app data folder | Session logs, bridges, each session's harness state (`projects/<id>/pm/`), the harness state of a brief ask (`asks/<id>/`), what each harness was last asked (`options/<id>.json`), `projects.json` | Never |
| OS keychain | Logins and other secrets | Never |

Settings are JSON with a `$schema`, so an editor checks them too. A project's settings sit in the config folder under `projects/<id>/settings.json`, not in the repo, since allowlists and models are personal and a repo can be public.

### Layers

Built-in, then global, then project. A field shows its value and the layer it came from (*from kelpie*, *from global*), with a reset to the layer below. A project layer holds only the fields it overrides.

The same component edits agent files, where `extends` is one more layer: a field set by `kelpie:pm` shows *from kelpie:pm*.

The PM's harness, model and effort are kept once: their global value is the global `pm` agent's keys, and a project's override is its `pm` setting. The settings page and the agent editor edit the same value, so the PM resolves as `kelpie:pm`, then the global `pm` agent, then the project.

### One schema

The settings types live in Rust, and `schemars` writes their JSON Schema. The form, the validation, the `$schema` file and the docs all come from it. A field's description in Rust is its help text in the form.

The form is rendered by `@rjsf/shadcn`. The layered field is its first test: if showing the layer and its reset fights rjsf's templates, that one widget is kelpie's own, not the whole form.

The layered field fit rjsf's `FieldTemplate`, so it is one template and no widget of kelpie's own (`src/settings/LayeredField.tsx`). rjsf's own selects and switch are replaced by kelpie's shadcn ones, and the schema is made drawable first (`src/settings/schema.ts`): nullable types collapse, `$ref`s resolve, and each `const` is titled. A choice and a switch are written as they change, a text field and a list when it is left.

### Fields

| Field | Layer | Notes |
|---|---|---|
| PM harness | Global, project | Claude Code or goose |
| PM model, effort | Global, project | Listed from the harness's own config options (ACP), so kelpie names no model outside the default. Kelpie starts the harness briefly to ask, sending no prompt, and keeps the list until a refresh |
| Worker harness, model, effort | Project | Empty by default: kelpie names none (ADR 0006). The global value is the global `worker.md`'s own keys over `kelpie:worker`, as the PM's is `pm.md`'s (`workers.md`, Starting one) |
| Worker slots | Project | How many of the project's workers run at once, 2 by default, 0 to 64. A worker that is Paused or waiting on the maintainer holds none, and lowering the cap takes slots back from the newest holders (`workers.md`, Slots, pacing and leases) |
| Provider and host | Global, project | For a harness that runs a model on a server, such as goose on Ollama |
| Login | Global | Per harness, in the keychain. The field says *set* or *not set* and never shows the value. For Claude Code it is pasted, beside the command that makes it (`claude setup-token`). Kelpie running that command itself waits for the drawer's terminal |
| Reply style | Global, project | Kelpie's Concise-plus style by default (`ux.md`, item 4) |
| Network allowlist | Project | Domains a sandbox request saved with *Allow for this project* (ADR 0005) |
| Read anywhere by default | Project | Off (`ux.md`, Sandbox) |
| Harness global folder | Project | Off. When on, the entries each harness reads from its own global folder are linked into the session's config folder: Claude Code's `CLAUDE.md`, `settings.json`, `skills`, `agents`, `commands` and `output-styles` from `~/.claude` (or `CLAUDE_CONFIG_DIR`), and goose's `config.yaml` from `~/.config/goose`. Logins and secrets stay closed, and the harness writes nothing there. Applies when the session next starts (ADR 0006) |
| Bridge updates | Global | Per bridge: *Registry, automatic* by default, *Registry, ask first*, or *Kelpie's pin* (`bridge-updates.md`) |
| Lease capacity | Global | How many commands hold each lease at once, keyed by the lease's name, with `cargo-test` at 3. Each count is 1 to 64: a lease with no place can never be taken, and every command queued behind one waits forever. A lease no layer names holds one at a time. A lease is the machine's, so its capacity is one value for every project. The form has no row for the map yet, so the file is the way in (`workers.md`, Slots, pacing and leases) |

### Agent files

SK's agent format comes over whole with its tests (`.claude/skills/lift-from-sk`): Markdown with YAML frontmatter, `extends` with `kelpie:<name>` built-ins and `body: append`, and `_`-prefixed files ignored. Phase 1 keeps the keys a PM uses (`role`, `harness`, `model`, `effort`, `extends`, `body`). The board brings the worker keys (`description`, `kinds`, `tier`).

The editor: the frontmatter as the layered form, the body in an MDX editor (chelone's `mdx-editor`, `docs/component-sources.md`), and a preview of the agent with its `extends` chain resolved.

### When a change applies

From the next message, as `pm-chat.md` settles for switching. A model change keeps the session. A harness change starts a new one behind a divider.

## Synergy

| Piece | Reused by |
|---|---|
| The layered field | Settings, agent files, and later the agent environment table (`ux.md`) |
| The schema from Rust | The form, validation, the `$schema` file, the docs |
| ACP config options (#12) | The model and effort lists, for every harness |
| SK's agent loader and its tests, lifted | `kelpie:pm` and every agent after it |
| chelone's `mdx-editor` | Agent bodies, and later ticket notes |
| The keychain | Every harness's login, later forge tokens |

It would duplicate SK's TOML settings and its `shep kelpie set` command, which stay with SK. Kelpie has no settings command: the form is the way in, and the file is there for editors and sync.

Hard to reverse, so recorded as an ADR: the config folder is `~/.config/kelpie` on every OS, apart from the data (ADR 0008).
