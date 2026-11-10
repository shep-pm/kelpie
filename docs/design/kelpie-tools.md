# Kelpie tools

How an agent inside its sandbox calls tools that kelpie itself serves, and the first of them, the sandbox request. It follows `docs/design/ux.md` (Sandbox) and ADR 0005, where the PM changes everything outside its own folder through kelpie's tools. Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-09.

## Use case

1. The PM is checking a crate and fetches docs.rs. The sandbox refuses it, the step fails, and today all the PM can do is say so in prose. The maintainer has no way to let it through short of editing settings and restarting the session.
2. With a kelpie tool, the PM asks for docs.rs and says why. A prompt appears in its panel: *"PM wants to reach docs.rs: Allow once · Allow for this project · Deny"*. One click widens the running sandbox, and the PM carries on in the same turn.
3. Every later kelpie tool (tickets, links, areas, notes to workers, GitHub) rides the same path, so the PM's permissions stay its tool list (ADR 0005) on every harness.

## Design

### One MCP server, delivered over stdio

Kelpie serves its tools as an MCP server, one per session, and hands it to the harness in ACP's `mcpServers` on `session/new`, `session/load` and `session/resume`. It is given as a stdio server, since stdio is the one transport every ACP agent must take: Claude Code's bridge and goose both do (`docs/acp.md`), and so will any agent added later (ADR 0006).

The stdio server is kelpie's own binary run as `kelpie tools`, which relays MCP between its stdin and stdout and kelpie. The tools themselves run in kelpie, which holds the sandbox, the settings and the window. The harness starts the relay as its own child, so it sits in the session's process group with its `KELPIE_RUN` (ADR 0001) and stops with the session.

```
kelpie ── ACP over stdio ──> harness            (inside crabpot)
  ▲                            └─ kelpie tools  (MCP over stdio)
  └──────── the route out ────────┘
```

### The route out of the sandbox

The relay runs inside the sandbox, so it reaches kelpie only by a door the policy names. The door differs by platform, because crabpot's narrow grants do:

| Platform | Door | crabpot key |
|---|---|---|
| macOS | A Unix socket in kelpie's data folder | `allowUnixSockets`, that one path |
| Linux | TCP on `127.0.0.1:<port>`, a port kelpie picks at launch | `allowTcp`, that one address and port |
| Windows | A named pipe | Waits with the rest of Windows (ADR 0005) |

Neither key opens anything else. Linux cannot take the socket today, since crabpot's seccomp filter refuses every `AF_UNIX` socket whatever `allowUnixSockets` lists. macOS cannot take `allowTcp`, which Seatbelt cannot narrow to one host. `allowLocalBinding` would work on both and is turned down, since it opens every loopback port to the agent.

Each session gets a token, passed to the relay in its environment. Kelpie answers only a relay holding a live session's token, and the token says which session is asking, so one session's agent cannot act as another's.

### The sandbox request

The first tool is `request_network(host, why)`:

- It waits for the maintainer. The prompt shows in the session's panel and adds one to the attention count (`ux.md`, Attention)
- *Allow once* adds the host to the running sandbox's network lists with crabpot's `Sandbox::replace_network_lists`, for as long as the session runs. *Allow for this project* does the same and saves the host to the project's network allowlist, where the settings page can undo it. *Deny* changes nothing
- The tool answers the agent with the outcome in one line, so it can retry or move on
- A turn interrupted, or kelpie quitting, while it waits answers *denied*, and the prompt goes away
- crabpot's proxy already refuses with a 403 naming the reason, so the agent can tell a sandbox refusal from a site that is down. `kelpie:pm` gains one line: when the sandbox refuses a host, ask with `request_network`

Reads and writes outside the policy are not requests in this note. *Read anywhere by default* (#52) covers reads, and writes outside the PM's folder go through later tools by design.

### To check while building

- That `allowTcp` reaches a listener on the host's `127.0.0.1` from inside the sandbox on Linux. The proxy lets an IP literal through on its listed port, which the code shows and crabpot's tests cover, but no run has tried it end to end
- That Claude Code's bridge starts a stdio server given in `mcpServers` inside crabpot, with the relay's environment intact

## Synergy

| Piece | Reused from |
|---|---|
| Starting and stopping the relay | The process tree, through the harness (ADR 0001) |
| Widening a running sandbox | crabpot's `Sandbox::replace_network_lists`, held per session by #33 |
| *Allow for this project* | The network allowlist setting (#26) and its row on the settings page (#30) |
| The prompt | The chat panel (#14) and the attention count |
| The door and its Windows named pipe | SK's lease door (`crates/core/src/lease/door.rs` and `crates/core/src/ipc`), lifted |
| Telling the PM to ask | `kelpie:pm` (#27) |
| Every later kelpie tool | This server and its route out |

It would duplicate ACP's own `session/request_permission`, which the harness sends before a tool runs. That prompt is off inside the sandbox (`pm-chat.md`, Permissions), and it cannot widen anything, so it stays off.

When ACP's MCP over ACP is stable and the harnesses take it, the relay and its door can go: the tools would travel over the ACP connection kelpie already holds. Today it is an unstable RFD that neither Claude Code's bridge nor goose accepts, and its polyfill dials loopback TCP, which the sandbox blocks.

Built in Phase 1, after #33, since the sandbox prompt is one of Phase 1's promises.

No ADR: the tools are MCP either way, and the route out is kelpie's own plumbing, which can change without touching a tool.
