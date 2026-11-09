# Harnesses speak ACP

Decided on 2026-11-09 with the maintainer, while designing the PM chat (`docs/design/pm-chat.md`). The research is `docs/acp.md`.

Kelpie reaches harnesses through the Agent Client Protocol (ACP), as a client built on the `agent-client-protocol` crate. A harness that speaks ACP natively, such as goose, is run directly. One that does not, such as Claude Code or Codex, is run through its bridge from the ACP registry (`claude-agent-acp`, `codex-acp`), which drives the person's own install. Kelpie's own session interface sits above, with events shaped after ACP's, so a harness whose bridge falls short can have a native adapter behind the same interface. pi is the expected case, until its bridge exposes steer, fork and resume.

Phase 1 runs Claude Code and goose, both over ACP.

## Considered options

- **A native adapter per harness**, over Claude Code's stream-json, Codex's app-server, pi's RPC and goose. Every feature reachable, but four adapters on four protocols, and every new harness is another.
- **Rewrite the bridges in Rust.** The Claude and Codex bridges are about 42k lines of TypeScript each, and follow their harnesses weekly. Unlike crabpot, a rewrite would be chased forever.
- **pi as Phase 1's second harness**, over its own RPC. It would cost a second adapter that is dropped once pi's bridge catches up. goose speaks ACP natively and runs local models too.

## Consequences

- One adapter serves Claude Code, goose, Codex and any agent in the ACP registry.
- Bridges are prebuilt executables, checked against the registry's sha256, so neither Node nor Bun is needed. Kelpie bundles no harness: each runs the person's own install and login. The Claude Code bridge's archive carries a `claude` of its own, which kelpie leaves out, pointing the bridge at the person's with `CLAUDE_CODE_EXECUTABLE`.
- ACP churns: the Rust crate had four major versions between June and November 2026, and a breaking v2 is drafted. Kelpie pins the crate and each bridge.
- Some features live in extensions or unstable parts of the spec: steering mid-turn, fork, and per-turn cache counts. The Claude Code bridge drops the split between the one-hour and five-minute cache unless its raw messages are asked for. Capabilities say what each harness supports, and the chat degrades where one cannot.
- goose puts its shell commands in process groups of their own, so stopping a session stops everything carrying its `KELPIE_RUN`, not only the harness's group (ADR 0001).
- To check before #12 is rewritten: that the Claude Code bridge's `claude` stays in a group kelpie stops, and that an output style set through its settings reaches the session.
