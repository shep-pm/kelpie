# ACP as kelpie's harness interface: findings (2026-11-09)

Method: spec repo, each bridge and agent repo, crates.io, npm, vendor docs. Source clones were shallow and read only, nothing was built or run. "Source" means I read code, so it is undocumented behaviour that can change. "Unconfirmed" means I could not verify it.

Pinned commits (default branch on 2026-11-09):
- spec: agentclientprotocol/agent-client-protocol@75438f7
- claude bridge: agentclientprotocol/claude-agent-acp@766fd14
- codex bridge: agentclientprotocol/codex-acp@52cc518 (the older Rust bridge zed-industries/codex-acp@296069e is archived and points here)
- goose: aaif-goose/goose@0edf2bd
- pi: earendil-works/pi@e5d9ff9
- pi bridge: pi-acp@04d0a15 (repo URL is in agentclientprotocol/registry, file pi-acp/agent.json)
- Rust SDK: agentclientprotocol/rust-sdk@e75d6f1

## 1. Comparison table (what kelpie needs, per harness, over ACP)

| Need | Claude Code via claude-agent-acp | Codex via codex-acp | pi via pi-acp | goose (native `goose acp`) |
|---|---|---|---|---|
| Native or bridge | Bridge (Node/TS). Also standalone binary per release, which ships the `claude` CLI beside it. | Bridge (Node/TS). Rewritten from Rust to TS; Rust one archived. | Bridge (Node/TS), third party, one individual owner, pi itself has no ACP code. | Native. `goose acp` is a subcommand of the Rust binary. |
| Licence, last release, stars | Apache-2.0 (LICENSE). Registry marks it "proprietary" because it bundles Claude Code. v0.101.1, 2026-11-09. 2.7k stars. | Apache-2.0 per package.json and LICENSE text (GitHub shows NOASSERTION). v2.6.0, 2026-11-08. 559 stars. | MIT. v0.0.34, 2026-09-24. 767 stars. Needs pi >= 0.81.0 (pi is at 1.5.0). | Apache-2.0. v1.57.0, 2026-10-28 (main is 1.58.0). 56k stars. Part of the Agentic AI Foundation (Linux Foundation) per README. |
| How it drives the harness | Claude Agent SDK (`@anthropic-ai/claude-agent-sdk` 0.3.319) `query()`, which runs the `claude` binary. | Spawns `codex app-server` (JSON-RPC), uses `thread/*` and `turn/*` methods. Bundles `@openai/codex` ^0.166.0. | Spawns `pi --mode rpc --no-themes`, speaks pi RPC. | In-process agent, no second harness. |
| Send while a turn runs | Yes. A second `session/prompt` is queued (source: `turnQueue`, `_meta.claudeCode.promptQueueing: true`). Or `_session/steering` (custom ext), delivered at priority `now`, or `later` if a permission or elicitation is open. Outcomes: injected, startedNewTurn, promptRequired. | Yes via `_session/steering` (maps to `turn/steer`), docs/steering.md. Plain second `session/prompt` behaviour: Unconfirmed. | Only queued client-side, one at a time. pi RPC has `steer`, `follow_up` and `streamingBehavior`, but pi-acp sends none of them (command union has no steer or follow_up). | A second `session/prompt` is rejected with invalid_params ("use _goose/unstable/session/steer"). Steer exists only as a goose custom method. |
| Interrupt | `session/cancel` interrupts the CLI, stopReason `cancelled`. | `session/cancel` maps to `turn/interrupt`. | `session/cancel` maps to RPC `abort`, and clears the client-side queue. | `session/cancel` trips a cancellation token. |
| Resume after process died | `session/resume` (no replay) and `session/load` (replay) advertised. Reads Claude Code's on-disk transcripts. | `session/resume` and `session/load` advertised (`thread/resume`). | `session/load` and `session/list`/`delete` only. Keeps `~/.pi/pi-acp/session-map.json` to find the pi session file. No `resume`. | `session/load` (replay), list, delete, close advertised. No `resume`. |
| Fork | Advertised (`session/fork`, unstable in spec). Uses SDK `forkSession`. | Advertised (`thread/fork`). | No. pi RPC has `fork` and `clone`, bridge does not expose them. | Handler `on_fork_session` exists, but the initialize response does not advertise `fork`. Reachable from a client: Unconfirmed. |
| Model and effort | Config options for model and effort (`session/set_config_option`), plus `_meta.claudeCode.options.model/effort`. | Config options: model, `reasoning_effort`, fast mode, mode. | Models via unstable set-model; pi thinking levels exposed as a "mode" selector. | Config options and `session/set_model`. Effort only for providers that support it (source). |
| Claude output styles | No ACP option. `outputStyle` in Claude settings files should apply because the bridge loads user, project and local settings; settings can also be passed in `_meta.claudeCode.options.settings`. Whether a style set this way takes effect in SDK sessions: Unconfirmed. | n/a | n/a | n/a |
| Usage and cache per turn | `PromptResponse.usage` has inputTokens, outputTokens, cachedReadTokens, cachedWriteTokens, per turn (accumulator reset at turn start). Also `_meta.quota` with a per-model split, and `usage_update`. The 5m vs 1h split (`usage.cache_creation.ephemeral_*`) is not forwarded; it is only available in raw SDK messages if you set `_meta.claudeCode.emitRawSDKMessages` (sent as `_claude/sdkMessage`). | `PromptResponse.usage` with cachedRead and cachedWrite, `thoughtTokens`, plus `_meta.quota`. | Only `usage_update` (context used and size). Cache counts appear only in a `/session` text reply. pi RPC `get_session_stats` has cacheRead and cacheWrite, but cumulative. | `PromptResponse.usage` has total, input, output only. No cache counts. `usage_update` has used, size, cost. |
| MCP servers | Yes: stdio baseline, http and sse advertised. Merged with the SDK's own. | Yes: stdio and http. SSE off. | Accepted and stored, not wired into pi. pi has its own MCP support (docs/mcp.md), outside ACP. | Yes: stdio baseline, http advertised, no SSE. |
| Harness permission prompts off | Mode `bypassPermissions`, unless running as root (then clamped to default unless `IS_SANDBOX` is set). Can be removed per session. | Mode `agent-full-access` (approval `never`, sandbox `dangerFullAccess`). | Nothing to turn off: pi never asks (docs/security.md). | Mode `auto` (the default) approves all tool calls. |
| Read the transcript | `session/load` replays history as `session/update`. Optional raw SDK stream. Claude's own jsonl stays readable on disk. | `session/load` replays. Codex rollouts on disk. | `session/load` replays from pi's session files (`~/.pi/agent/sessions`). | `session/load` replays from goose's session store. |
| Auth | Uses the `claude` CLI login. Subscription works (see section 4). | ChatGPT login, API key, or custom gateway. | Terminal login, delegated to `pi` itself. | `goose configure` (provider and key). Also acts as an ACP client of the bridges on the left. |

Cross-cutting: per-turn cache counts (`Usage`) are an unstable spec feature, so a client has to opt into unstable types. Both Claude and Codex bridges also share the same pre-standard `_session/steering` extension.

## 2. The spec

Covers (v1 stable, spec docs under docs/protocol/v1, schema/v1/schema.json):
- Lifecycle: `initialize` (version int plus capabilities), `authenticate`, `logout`, `session/new` (cwd, mcpServers, additionalDirectories, systemPrompt), `session/load` (needs `loadSession`, replays history), `session/resume` (no replay), `session/list`, `session/delete`, `session/close`, `session/prompt`, `session/cancel` (notification).
- `session/update` kinds: user_message_chunk, agent_message_chunk, agent_thought_chunk, tool_call, tool_call_update, plan, available_commands_update, current_mode_update, config_option_update, session_info_update, usage_update (context used, size, cost), notice, compaction_update, compaction_summary_chunk (docs/protocol/v1/prompt-turn.mdx).
- Agent to client requests: `session/request_permission` (allow_once, allow_always, reject_once, reject_always), `fs/read_text_file`, `fs/write_text_file`, `terminal/create|output|release|wait_for_exit|kill`, `elicitation/create`.
- Modes (`session/set_mode`) and generic config options (`session/set_config_option`) with categories `mode`, `model`, `model_config`, `thought_level` (docs/protocol/v1/session-config-options.mdx). Model selection is a config option, not a dedicated method.
- MCP passthrough: `mcpServers` on new, load and resume. stdio mandatory, http and sse by capability.
- Fork: `session/fork` is draft, only in schema.unstable.json (docs/rfds/session-fork.mdx).
- Per-turn token usage (input, output, thought, cachedRead, cachedWrite) on `PromptResponse.usage`: draft (docs/rfds/end-turn-token-usage.mdx, only in schema.unstable.json). Stable v1 only has context-window `usage_update`.
- Steer or queue: not in v1. In draft v2, `session/prompt` may be sent while work is active; the agent either inserts it at once or holds it; "explicit queueing or steering capabilities" are left to a later RFD (docs/protocol/v2/prompt-lifecycle.mdx, docs/rfds/v2/prompt.mdx).
- Request cancellation `$/cancel_request` is stable.

Version and stability: wire `protocolVersion` is an integer, 1 is stable (schema 1.0 on 2026-06-24, per CHANGELOG.md). v2 is draft (schema 2.0.0-alpha.14). v2 removes client fs, client terminal and session modes (docs/protocol/v2/migration.mdx), so a client that leans on those will have to change. Capabilities are the additive mechanism.

Licence: Apache-2.0 (LICENSE). Governance: interim, jointly governed by Zed and JetBrains, aiming at an independent foundation, not yet formed (agentclientprotocol.com/community/governance). RFD process with Draft, Active, Preview, Completed.

Cadence: schema releases roughly weekly (1.16.0 on 2026-11-09, 1.15.1 on 11-02, 1.15.0 on 10-31); 55 changelog headings since the 0.x days. Additive within v1.

Transport: stdio JSON-RPC. Streamable HTTP and WebSocket is an Active RFD, not stable.

## 3. Who speaks it natively, and who needs a bridge

- Native: goose (`goose acp`, aaif-goose/goose crates/goose/src/acp/server.rs), Gemini CLI (`--acp`, registry entry gemini/agent.json), and many others listed on agentclientprotocol.com/overview/agents and in the registry (OpenCode, Cursor, Copilot CLI, Kiro, Junie, Qwen Code, Kimi, Mistral Vibe, and more). That page does not mark native vs adapter except for four adapters.
- Bridges: Claude Code (claude-agent-acp), Codex (codex-acp), pi (pi-acp). Those three are the only ones the agents page labels "Adapter" besides Bub.
- Claude Code: bridge is agentclientprotocol/claude-agent-acp, npm `@agentclientprotocol/claude-agent-acp`, Node >= 22, TypeScript, Apache-2.0, release-please driven (about 17 releases a month in Sept and Oct 2026). Drives the Claude Agent SDK, which runs the `claude` binary. Registry lists authors as Anthropic, Zed and JetBrains. Stated author in package.json: Zed Industries.
- Codex: agentclientprotocol/codex-acp, npm `@agentclientprotocol/codex-acp` 2.6.0, TypeScript, Apache-2.0 (copyright line is JetBrains). Drives `codex app-server`. The registry lists authors OpenAI, JetBrains and Zed.
- pi: pi-acp, MIT, TypeScript, Node, one individual maintainer, 0.0.34, depends on `@agentclientprotocol/sdk` ^0.26.0 while the SDK is at 2.5.0, so it lags. README calls itself MVP-style and "centered around Zed".
- Registry: agentclientprotocol/registry (Apache-2.0, 451 stars) lists all of the above, with npx or binary distribution.

## 4. Auth and cost (Claude Code)

- Subscription works through the bridge. It uses the `claude` binary's own credentials, and offers terminal login methods "Claude Subscription" (`auth login --claudeai`) and "Anthropic Console" (`--console`) (claude-agent-acp@766fd14 src/acp-agent.ts, around line 2990). goose's own docs say the Claude ACP provider "uses the same Claude subscription" (documentation/docs/guides/acp-providers.md).
- Cache TTL is decided by Claude Code, not the bridge. Claude Code docs: Agent SDK turns fall in the "main conversation" bucket, which gets 1 hour on a subscription within plan usage and 5 minutes on API key, cloud provider or usage credits. Overridable with `CLAUDE_CODE_PROMPT_CACHE_TTL` or `ENABLE_PROMPT_CACHING_1H` (code.claude.com/docs/en/prompt-caching, "Which TTL each request gets"). The bridge accepts `_meta.claudeCode.options.env`, so kelpie can set these. It also runs an idle compaction timer keyed to the 1h TTL (src/idle-compaction.ts).
- Policy risk, not a technical block: Claude Code docs say developers building products, including on the Agent SDK, should use API keys, and Anthropic "does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, Max, or Ultra plan credentials on behalf of their users" (code.claude.com/docs/en/legal-and-compliance). The same page says an end user may sign in to the unmodified Claude Code binary with their own subscription. The SDK overview repeats "Unless previously approved ... offer claude.ai login or rate limits" (code.claude.com/docs/en/agent-sdk/overview). The bridge ships a `--hide-claude-auth` flag so hosts can refuse subscription billing (src/hide-claude-auth.ts). Whether kelpie (a local app launching the user's own CLI login) is inside the allowed case is a legal reading I did not make. Native adapter over `claude` stream-json would face the same wording.
- Codex: ChatGPT subscription login or API key (codex-acp README). Pi and goose: provider keys or login handled by the harness.

## 5. Process shape (for process groups and tree kill)

- Claude: kelpie > bridge (node, or standalone binary) > `claude` native binary (spawned by SDK, path from `CLAUDE_CODE_EXECUTABLE`, the binary beside it, or the SDK's platform package, src/claude-cli.ts) > Bash and MCP stdio children. Whether the SDK detaches the child into its own group: Unconfirmed (SDK is closed source, not read).
- Codex: kelpie > bridge (node) > `@openai/codex/bin/codex.js` > native `codex app-server`. On Unix with a Node that has `process.execve`, codex.js replaces itself, so it is one process; otherwise it spawns and forwards SIGINT, SIGTERM and SIGHUP (openai/codex codex-cli/bin/codex.js on main). The bridge spawns without `detached` (src/CodexJsonRpcConnection.ts). The bridge may also run `codex` for login.
- pi: kelpie > pi-acp (node) > `pi --mode rpc` > tools, extensions. Plain spawn, SIGTERM on dispose (src/pi-rpc/process.ts).
- goose: one Rust process, `goose acp`. It puts its shell tool commands and other subprocesses into their own process groups (crates/goose/src/subprocess.rs `process_group(0)`, developer/shell_process.rs). Killing goose's group will not reach those. Kelpie would need goose to clean up (it sets `kill_on_drop`), or a different tree walk. This is a real gap for "kill the tree on quit".
- Good news on the Rust side: `AcpAgent` in agent-client-protocol makes the agent its own process-group leader and, on shutdown, closes stdin, sends SIGTERM to the group, then SIGKILL (src/agent-client-protocol/src/acp_agent.rs). This covers `npx`/`node` wrappers. It does not cover descendants that leave the group (goose shell).

## 6. Rust client side

- Crate `agent-client-protocol` 5.1.0 (2026-11-09), Apache-2.0, MSRV 1.88, 88 versions, about 6.1M total downloads, repo agentclientprotocol/rust-sdk (244 stars) (crates.io API).
- Supports the Client role: `Client.v1()` builder, `AcpAgent` to spawn a stdio agent, examples `yolo_one_shot_client.rs` and `v2_one_shot_client.rs` (rust-sdk@e75d6f1 src/agent-client-protocol/examples). Also http transport crate, rmcp integration, conductor and proxy crates.
- Maturity: widely used (Zed, goose) but churny. Major version went 1.0.0 on 2026-06-24 to 5.0.0 on 2026-11-02, with many "breaking" entries, e.g. 5.0.0 drops JSON-RPC batching and renames types (src/agent-client-protocol/CHANGELOG.md). Unstable features sit behind cargo features (`unstable_*`), including end-turn token usage and session archive. goose pins schema 1.9.1 and crate 2.2.0, so even the biggest Rust user lags.
- Schema crate `agent-client-protocol-schema` 1.16.0 (Apache-2.0).
- TypeScript: `@agentclientprotocol/sdk` 2.5.0, Apache-2.0 (npm).

## 7. Other clients (adoption)

agentclientprotocol.com/overview/clients lists, among others: Zed, JetBrains IDEs (AI Assistant), Neovim (CodeCompanion, avante.nvim, agentic.nvim), Emacs (agent-shell), VS Code extensions, Sublime Text, Qt Creator, Obsidian plugins, Unity, Pulsar, Visual Studio, Jupyter and marimo, plus 60-odd desktop and web apps (several are Tauri, e.g. Athas), mobile clients and CLI/TUI clients (acpx, Toad, others). The page says it does not verify features or endorse projects. Zed docs list Claude, Codex, OpenCode, Copilot, Cursor and Pi as external agents through an ACP registry (zed.dev/docs/ai/external-agents). goose is itself an ACP client of Claude, Codex, pi and Amp bridges, and documents its limits: no session fork or resume through ACP providers (guides/acp-providers.md).

## 8. Verdict

What ACP would save:
- One wire protocol, one Rust client (`agent-client-protocol` + `AcpAgent`) instead of four adapters on three IPC styles (SDK/stream-json, app-server JSON-RPC, pi RPC, goose).
- Uniform handling of: streamed content, tool calls with diffs, permission requests, plans, cancel, session list/load, config options for model and effort, MCP passthrough.
- Free harnesses later (Gemini CLI, OpenCode, Cursor and others are already registered) and process-group spawn plus escalating kill from the SDK.
- The Claude and Codex bridges are actively maintained by the ACP org, with new features landing weekly.

What it would cost or block:
- Dependency on churn: spec adds features weekly, the Rust SDK made four major-version bumps (1.0 to 5.0) between 2026-06-24 and 2026-11-02, v2 is a breaking draft that drops client fs, terminal and modes. Pin versions and budget for upgrades.
- Extra hop and extra moving parts: Node (>= 22) or a bundled binary per bridge, plus a second process (`claude`, `codex app-server`, `pi`). pi-acp is third-party, MVP, one owner, behind pi's current version.
- Useful things live outside the standard: steering (`_session/steering`, pre-standard, shared by Claude and Codex bridges only), cache TTL split, raw SDK stream, goose steer, `_meta.claudeCode.options`. Kelpie ends up writing per-harness `_meta` code anyway.
- Per-turn token and cache counts need the unstable `usage` feature. The bridge paths are not uniform (goose has no cache counts, pi-acp none per turn).
- The Claude subscription policy question in section 4 applies to either option.

Gaps where a native adapter would still be needed:
- Mid-turn steer on pi (pi RPC `steer`, `follow_up`: not exposed by pi-acp) and goose (custom method only).
- Fork on pi and goose (pi RPC has it, goose handler not advertised). Resume on pi and goose (load only).
- Cache TTL breakdown (`cache_creation.ephemeral_1h/5m`) per turn, which only a raw SDK or stream-json feed gives you.
- Claude output styles, and any `claude` flags not in `_meta.claudeCode.options`.
- Tree kill for goose's shell subprocesses.
- Reading the harness-native transcript format directly, if kelpie wants it without replay through `session/load`.

Suggested shape (opinion, not a finding): hide harnesses behind kelpie's own session trait (option B) and implement the Claude and Codex adapters on ACP via the Rust crate (they have the best coverage), keeping a native pi RPC adapter and probably native or ACP goose with the gaps above. That is a hybrid, and it keeps ACP as an adapter detail rather than the interface.

## Checked by hand (2026-11-09)

One Haiku session through `claude-agent-acp` v0.101.1 (darwin-arm64 executable, its sha256 matching the registry's, the bundled `claude` deleted so `CLAUDE_CODE_EXECUTABLE` pointed at the installed Claude Code 2.1.319). A small Python ACP client started the bridge in its own process group, sent `initialize`, `session/new` with `_meta.claudeCode.emitRawSDKMessages` and model `haiku`, and one prompt, from a folder whose `.claude/settings.json` set `outputStyle` to `Explanatory`.

| Check | Result |
|---|---|
| Protocol | `protocolVersion` 1, stop reason `end_turn` |
| `claude` in the bridge's process group | Yes. Its MCP servers launched through npm stayed in the group too. One Python MCP server started by `claude` made a group of its own |
| SIGTERM to the bridge's group | Every process seen during the turn was gone two seconds later, the one in its own group included, since its parent's exit closed its input |
| Output style | The raw `system` `init` message said `output_style: Explanatory` |
| Usage per turn | `PromptResponse.usage`: `cachedWriteTokens` 28157, `cachedReadTokens` 0 |
| Cache lifetime split | The raw `result` message: `ephemeral_1h_input_tokens` 28157, `ephemeral_5m_input_tokens` 0, so the subscription's one-hour cache applied |
| Settings the bridge read | User settings too: the person's own MCP servers started, and the bridge warned that `permissionMode` was `bypassPermissions`, from the person's settings |

## Sources

- Spec: agent-client-protocol@75438f7 docs/protocol/v1/{initialization,prompt-turn,session-setup,session-config-options,tool-calls}.mdx, docs/protocol/v2/{prompt-lifecycle,migration}.mdx, docs/rfds/{session-fork,end-turn-token-usage}.mdx, schema/v1/{meta,schema,schema.unstable}.json, CHANGELOG.md, MAINTAINERS.md, docs/community/governance.mdx
- https://agentclientprotocol.com/overview/agents , /overview/clients , /community/governance
- claude-agent-acp@766fd14 README.md, package.json, src/{acp-agent,claude-cli,hide-claude-auth,idle-compaction,session-effort,fork-session}.ts, docs/{air-extensions,model-configuration}.md
- codex-acp@52cc518 README.md, package.json, docs/steering.md, src/{CodexAcpServer,CodexJsonRpcConnection,CodexCli,AgentMode,TokenCount,CodexAppServerClient}.ts
- pi-acp@04d0a15 README.md, src/acp/{agent,session}.ts, src/pi-rpc/process.ts; pi@e5d9ff9 packages/coding-agent/docs/{rpc,rpc-commands,mcp,security}.md
- goose@0edf2bd crates/goose/src/acp/server.rs, crates/goose/src/acp/server/fork_session.rs, crates/goose/src/acp/response_builder.rs, crates/goose-provider-types/src/goose_mode.rs, crates/goose/src/subprocess.rs, documentation/docs/guides/{acp-providers,zed}.md, README.md
- rust-sdk@e75d6f1 README.md, src/agent-client-protocol/{CHANGELOG.md,src/acp_agent.rs,examples}
- https://github.com/openai/codex codex-cli/bin/codex.js (main)
- https://agentclientprotocol.com registry: agentclientprotocol/registry {claude-acp,codex-acp,goose,pi-acp,gemini}/agent.json
- crates.io API for agent-client-protocol, -schema, -http, -rmcp; npm for @agentclientprotocol/sdk and claude-agent-acp
- https://code.claude.com/docs/en/prompt-caching , /legal-and-compliance , /agent-sdk/overview
- https://zed.dev/docs/ai/external-agents
