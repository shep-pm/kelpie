# Config lives in ~/.config/kelpie

Decided on 2026-11-09 with the maintainer, while designing settings (`docs/design/settings.md`).

Kelpie's config (settings, agent files, kelpie-global instructions, skills and MCP servers) lives in `~/.config/kelpie` on every OS. What kelpie produces (session logs, bridges, each session's harness state, `projects.json`) lives in Tauri's app data folder, and secrets live in the OS keychain. Config can be synced or kept in a dotfiles repo by symlinking the folder. Data never is.

## Considered options

- **Tauri's app config folder**, with an environment variable to move it. Native to each OS, but on macOS it is `~/Library/Application Support/<identifier>`, which dotfiles tools and people rarely reach for, and on macOS it is the same folder as the data, so syncing one syncs both.
- **Tauri's folder plus a button to move it.** Friendlier, but it is more to build for Phase 1, and the button can come later on top of either choice.
- **One folder for config and data.** Simpler, but a synced folder would carry logs, downloaded executables and harness state between machines.

## Consequences

- Kelpie resolves `~/.config/kelpie` itself on macOS and Windows too, instead of using Tauri's config path.
- Moving the folder later means migrating people's files, which is why this is an ADR.
- Nothing in the config folder may hold a secret or a machine-specific path that breaks on another machine. Project settings are keyed by project id, and `projects.json`, which maps ids to local paths, stays in data.
