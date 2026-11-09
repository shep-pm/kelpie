# Settings and agents

Phase 1's settings page and global agent editor. It places itself in `docs/design/ux.md` (Settings and agents) and serves `docs/design/pm-chat.md` (Switching harness or model, the PM's instructions). Terms are in `GLOSSARY.md`.

**Status:** draft. Its four open questions were settled with the maintainer on 2026-11-09.

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
| Data, Tauri's app data folder | Session logs, bridges, each session's harness state (`projects/<id>/pm/`), `projects.json` | Never |
| OS keychain | Logins and other secrets | Never |

Settings are JSON with a `$schema`, so an editor checks them too. A project's settings sit in the config folder under `projects/<id>/settings.json`, not in the repo, since allowlists and models are personal and a repo can be public.

### Layers

Built-in, then global, then project. A field shows its value and the layer it came from (*from kelpie*, *from global*), with a reset to the layer below. A project layer holds only the fields it overrides.

The same component edits agent files, where `extends` is one more layer: a field set by `kelpie:pm` shows *from kelpie:pm*.

### One schema

The settings types live in Rust, and `schemars` writes their JSON Schema. The form, the validation, the `$schema` file and the docs all come from it. A field's description in Rust is its help text in the form.

The form is rendered by `@rjsf/shadcn`. The layered field is its first test: if showing the layer and its reset fights rjsf's templates, that one widget is kelpie's own, not the whole form.

### Phase 1's fields

| Field | Layer | Notes |
|---|---|---|
| PM harness | Global, project | Claude Code or goose |
| PM model, effort | Global, project | Listed from the harness's own config options (ACP), so kelpie names no model outside the default. Kelpie starts the harness briefly to ask, sending no prompt, and keeps the list until a refresh |
| Provider and host | Global, project | For a harness that runs a model on a server, such as goose on Ollama |
| Login | Global | Per harness, in the keychain. The field says *set* or *not set* and never shows the value. For Claude Code it is pasted, beside the command that makes it (`claude setup-token`). Kelpie running that command itself waits for the drawer's terminal |
| Reply style | Global, project | Kelpie's Concise-plus style by default (`ux.md`, item 4) |
| Network allowlist | Project | Domains a sandbox request saved with *Allow for this project* (ADR 0005) |
| Read anywhere by default | Project | Off (`ux.md`, Sandbox) |
| Harness global folder | Project | Off. Reads `~/.claude` or goose's own config when on (ADR 0006) |

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
