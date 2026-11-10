# Claude Code under kelpie

What was measured about Claude Code that still holds when kelpie runs it through `claude-agent-acp` (ADR 0007). Lifted from SK's `docs/facts/headless-claude-code.md` @ `932c57a5`. Each measurement was made under `claude -p` by SK, so the bridge, which runs the same binary, is expected to behave the same and was not re-measured unless a line says so. `docs/acp.md` has what was checked through the bridge itself.

## The config folder

- Claude Code writes a session's transcript to `<config folder>/projects/<slug>/<id>.jsonl`, and a resume reads it there. The config folder is `CLAUDE_CONFIG_DIR`, or `~/.claude` when unset. The bridge's `session/resume` and `session/load` read the same transcripts.
- A resume whose transcript is not where Claude Code looks fails with "No conversation found with session ID". SK met it on 2026-10-27 (shep-pm/shep-kelpie#1421): the transcript sat in `~/.claude/projects/<slug>/<id>.jsonl`, written minutes before, while the folder the session was given held none. Sessions restarted within 2 minutes were 11% of Claude Code starts before SK's sandbox change and 19% after it. The cause was the sandbox's own handling of `~/.claude`, so kelpie gives each session a config folder of its own and keeps its transcripts there (`harness::claude`).
- A call killed within its first second leaves no transcript, so a resume onto its id fails the same way. Once a transcript exists, starting a session on that id is refused as already in use.

## The login is per config folder

- Claude Code keeps the keychain login per config folder. With `CLAUDE_CONFIG_DIR` at an empty folder and no `CLAUDE_CODE_OAUTH_TOKEN`, `claude -p` wrote its transcript there and answered "Not logged in" (measured 2026-10-27).
- So a session with a config folder of its own has no login until it is given a token. `claude setup-token` makes one, and kelpie passes it as `CLAUDE_CODE_OAUTH_TOKEN` from its own login store (`harness::HarnessError::NoLogin`).

## Prompt cache sharing

- Measured 2026-09-24 on Claude Code 2.1.282, `--model sonnet`, one sample each. Two identical fresh calls share the whole prefix: the second read 41,010 tokens from cache, on a first call whose input, cache write and cache read summed to 41,012.
- A different working directory shares only about 10k of it. Sessions that should read each other's cache have to run in the same folder.
- The first `--resume` onto a fresh session rewrote about 29k of cache in one early probe. Seven runs in the same series wrote only the new turn, about 5.3k, so it did not reproduce.
