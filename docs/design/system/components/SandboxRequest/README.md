# SandboxRequest

The one prompt an agent can raise: asking to read, write or reach something outside its sandbox.

**The consumer provides** the agent ("PM", "Worker on #42"), the verb and target ("reach docs.rs", "write `~/.cargo/config.toml`"), and the agent's one-line reason.

**Copy**: "PM wants to reach docs.rs". Then exactly three actions: **Allow once** · **Allow for this project** · **Deny**.

**Rules**
- `status-needs-soft` fill and the needs-you glyph; it counts toward every NeedsYouCount above it until answered.
- Once answered, it collapses to one muted line saying what was decided. After **Allow for this project** the line adds "Undo in settings", since that choice is saved to the project's network allowlist (`docs/design/settings.md`).
- Never stack more than one open request per session: later ones queue behind it with "+2 more".
