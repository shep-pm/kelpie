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
| Worker turn ceiling | Global, project | How many minutes a worker's turn may run, 60 by default, 1 to 1440. Past it the turn is cancelled over ACP and the worker Paused, so the maintainer decides what comes next. The count starts when the worker's slot was granted, not at the launch, as SK's `worker.turn_timeout` and `Flight` have it (`workers.md`, Slots, pacing and leases). A project's value wins over the global one |
| Provider and host | Global, project | For a harness that runs a model on a server, such as goose on Ollama |
| Login | Global | Per harness, in the keychain. The field says *set* or *not set* and never shows the value. For Claude Code it is pasted, beside the command that makes it (`claude setup-token`). Kelpie running that command itself waits for the drawer's terminal |
| Reply style | Global, project | Kelpie's Concise-plus style by default (`ux.md`, item 4) |
| Theme | Global | *System* by default, *Light* or *Dark*. *System* follows the OS as the app did before the setting. The choice sets the window's own theme too (Tauri's `setTheme`), so the title bar matches the page, and it is read before the window's first paint, so a launch shows no flash of the other theme. A quick toggle elsewhere waits until one is missed |
| Network allowlist | Project | Domains a sandbox request saved with *Allow for this project* (ADR 0005) |
| Read anywhere by default | Project | Off (`ux.md`, Sandbox). On, it opens the whole file system and keeps every harness's login, every secret and every other session's state folder closed |
| Run without a sandbox | Project | Off. Applies only while this machine's sandbox cannot start, and only to the PM and its threads (`sandbox-setup.md`) |
| Harness global folder | Project | Off. When on, the entries each harness reads from its own global folder are linked into the folder the harness reads its config from: Claude Code's `CLAUDE.md`, `settings.json`, `skills`, `agents`, `commands` and `output-styles` from `~/.claude` (or `CLAUDE_CONFIG_DIR`) into `<state>/claude`, and goose's `config.yaml` from `~/.config/goose` (or `<GOOSE_PATH_ROOT>/config` when the person set it) into `<state>/goose/config`, which is where goose looks and not in the root kelpie gives it. Logins and secrets stay closed whatever this setting says, and the harness writes nothing there, for the PM or a worker. Applies when the session next starts (ADR 0006) |
| Bridge updates | Global | Per bridge: *Registry, automatic* by default, *Registry, ask first*, or *Kelpie's pin* (`bridge-updates.md`) |
| Lease capacity | Global | How many commands hold each lease at once, keyed by the lease's name, with `cargo-test` at 3. Each count is 1 to 64: a lease with no place can never be taken, and every command queued behind one waits forever. A lease no layer names holds one at a time. A lease is the machine's, so its capacity is one value for every project. The form has no row for the map yet, so the file is the way in (`workers.md`, Slots, pacing and leases) |
| Pick up issues from | Project | *Me* by default, which takes the issues the signed-in login opened, or *Everyone* for every open issue. An issue carrying the ready label is a ticket either way (`tickets.md`, Where tickets come from) |
| Ready label | Project | The name the ticket source gives the label a Ready ticket carries, `ready-for-agent` by default (`tickets.md`, The board). It names one label, so kelpie refuses an empty one and one holding a comma, since `gh issue list --label` splits its argument on commas |
| Done window | Global, project | How many days a closed ticket stays in the board's Done column, 14 by default, 1 to 365. A closed ticket older than this is off the board. A project's value wins over the global one, as how far back to look is one person's (`tickets.md`, The board) |
| CI holds the merge | Project | On by default. Off reads no checks and asks for the merge once the branch has the latest base (`gate-and-merge.md`, Settings) |
| Fix attempts | Project | A count from 0, and absent for no cap, which is the default. How many fix turns red CI gets for one ticket before a ruling. 0 rules on the first red run. SK spelled no cap as -1, which a file here refuses |
| CI wait | Project | Minutes from 1 to 1440, 60 by default. How long a check may sit queued before a rerun, and how long a head with no check waits before a ruling |
| Catch up | Project | Always, or Only on conflict, and Always by default. Whether a pull request behind the base is rebased whenever it lacks the latest base, or only when merging it into the base conflicts. Only on conflict lets a clean branch go on unrebased, and the merge queue or the base's CI tests the combination, which spares a repo whose merge queue does that a rebase and a CI rerun on every base move. SK's `[git]` `catch_up` (`gate-and-merge.md`, The order, catch-up) |
| Reviewers | Project | `kelpie:reviewer` by default. The reviewer agents a project uses, by name, in the order they read. An empty list is allowed and means no reviewer can run. A name listed twice is refused, since a reviewer reads a head once (`review.md`, Reviewers) |
| Test command | Project | Absent by default. A command that runs one failing test a reviewer wrote, holding `{test}`, the test file's path, or `{name}`, its file stem, such as `cargo test --test {name}`. A command with neither is refused. Set, a reviewer is asked to leave a failing test for each finding it can. SK's `ci.test_command` (`review.md`, Settings) |
| Reviewer harness, model, effort | Project | Empty by default: kelpie names none (ADR 0006). Each sits over every listed reviewer's own keys, as the worker's rows sit over `worker.md`'s (`review.md`, Settings) |

### Agent files

SK's agent format comes over whole with its tests (`.claude/skills/lift-from-sk`): Markdown with YAML frontmatter, `extends` with `kelpie:<name>` built-ins and `body: append`, and `_`-prefixed files ignored. Phase 1 keeps the keys a PM uses (`role`, `harness`, `model`, `effort`, `extends`, `body`). The board brings the worker keys (`description`, `kinds`, `tier`).

A reviewer is a file with `role: reviewer` (`review.md`, Reviewers). It takes two more keys, which SK refuses on any other role and kelpie refuses on a `pm` or `worker` file, by name:

| Key | Value | What it does |
|---|---|---|
| `paths` | A list of globs | Limits the reviewer to diffs that touch one. Empty or absent: every diff |
| `second_look` | `true` or `false` | A second fresh session reads after the first, shown what it found. Off when absent |

Kelpie ships `kelpie:reviewer`: SK's `defect-hunter` prompt with `second_look: true`, naming no harness or model (ADR 0006). Its body keeps `{{BASE}}` and `{{DIFF}}`, which the review slice fills. A global `reviewer.md` layers over it as `worker.md` does over `kelpie:worker`. The editor's form has no row for the two keys: it keeps them as the file has them, and the maintainer edits them in the file.

The editor: the frontmatter as the layered form, the body in an MDX editor (chelone's `mdx-editor`, `docs/component-sources.md`), and a preview of the agent with its `extends` chain resolved.

### When a change applies

From the next message, as `pm-chat.md` settles for switching. Theme applies at once, with no restart. A model change keeps the session. A harness change starts a new one behind a divider.

Read anywhere by default and Harness global folder are the exception: each lands at the session's next start, a relaunch or a harness switch, since crabpot widens only the network lists of a sandbox that is already running, and a link is made when the session's config folder is built.

A project whose folder is the home folder, or a folder above it, is refused at that start whichever way the setting is set, whether it reaches the home folder by name or through a link, and the refusal says so with the folder's path as the person chose it. The home folder holds every harness's login and every secret, so the project needs a folder of its own.

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
