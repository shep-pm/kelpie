# Lifted code

One row per lift from shep-pm/shep-kelpie (SK) or chelonejs/chelone. Each row has to stand on its own once SK is archived. `.claude/skills/lift-from-sk` has the steps for SK, and `docs/component-sources.md` names what chelone has.

| Date | Kelpie path | Source path @ commit | Handles (edge cases kept) | Dropped, and why | Tests carried |
|---|---|---|---|---|---|
| 2026-11-09 | `crates/kelpie-engine/src/harness/mod.rs` (`HarnessError::NoLogin`), `crates/kelpie-engine/src/harness/claude.rs` (`LOGIN_ENV`) | `crates/adapters/src/adapters/claude/mod.rs` @ `932c57a5` (`LOGIN_FIX`, `LOGIN_VAR`, `LoginToken`) | Claude Code keeps its login per config folder, so a session with a `CLAUDE_CONFIG_DIR` of its own has none, and says so with how to fix it: run `claude setup-token` and put the token it prints in kelpie's environment as `CLAUDE_CODE_OAUTH_TOKEN`. A blank token counts as no login | `shep secret set` and the runner's `env` line, which kelpie has no. SK's `LoginToken` type and its `AgentError::Setup`: kelpie has `harness::Login` and `harness::HarnessError` in their place. SK's `API_VAR` branch for a billed API key, which kelpie has no setting for yet | `harness::tests::no_login_says_how_to_make_one` |
