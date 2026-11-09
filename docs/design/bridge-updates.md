# Bridge updates

How kelpie keeps the bridges it installs current. It follows `docs/design/pm-chat.md` (The harness interface), which pins one version of each bridge in kelpie's code, and ADR 0007. Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-09.

## Use case

1. The maintainer uses the PM daily on Claude Code. Its bridge, `claude-agent-acp`, shipped about 17 releases a month in September and October 2026 (`docs/acp.md`), so a version pinned in a kelpie release is weeks behind within a month.
2. They get each new bridge without waiting for a kelpie release, and without a turn breaking mid-way.
3. A version that breaks a session puts it back on the last one that worked, and says so.
4. Someone who wants every version reviewed first can keep to kelpie's pin (ADR 0006).

## Design

### Scope

Only the bridges kelpie installs from the ACP registry, such as `claude-agent-acp` and later `codex-acp`. A harness is the person's own install (ADR 0007), so kelpie never updates `claude`, `goose` or `codex`. When a bridge reports its harness is too old, the chat shows the error with the harness's own update command.

### Flow

```
launch, then every 24h ── fetch the bridge's agent.json from the registry
  newer version ──> download, check its sha256 (#22's installer)
                ──> probe: start it in the tree, initialize, no prompt
                      fails ──> held back, shown in settings
                      passes ──> ready
ready ──> each session, at its next idle point:
            the next send starts the new version and resumes the session
            divider: "Updated claude-agent-acp from 0.101.1 to 0.102.0"
          the session fails to start or resume ──> back on the previous version, resumed
            the new version is marked bad until a newer one arrives
            divider: "claude-agent-acp 0.102.0 failed to resume, back on 0.101.1"
```

A failed or offline check changes nothing.

### The setting

One global field per bridge, on the settings page beside the harness's login (`docs/design/settings.md`):

| Value | What happens |
|---|---|
| Registry, automatic | The default. A version that passes the probe is used at each session's next idle point |
| Registry, ask first | A quiet notice with an Update button. It is not in the needs-you count, since nothing is blocked |
| Kelpie's pin | Only the version in kelpie's code, which changes with kelpie's releases |

The same row shows the version in use, where it came from (*registry* or *kelpie's pin*), and any version held back or marked bad, with the reason. Nothing shows in the rail or the count.

### The probe

Bridges are 0.x and release often, so a version number says nothing about compatibility. Before a new version is used, kelpie starts it briefly in the process tree, sends `initialize` and no prompt, and stops it. It passes when it speaks protocol 1 and keeps every capability kelpie uses for that harness, as SK's parity matrix lists them once lifted (`pm-chat.md`, Synergy). This is the same brief start, with no prompt, that lists a harness's models and efforts (#29).

### Swapping

Each session runs its own bridge process, and its state lives in the harness's own folder, so a bridge can change between turns without a drain. A turn already running finishes on the version it started with. Kelpie keeps three versions of each bridge on disk: the one in use, the one before it, and kelpie's pin. Any other is removed once no session uses it.

Kelpie's own updates still drain first (`docs/process-and-sk.md`, Updates). A bridge update never needs that.

### To check while building

That a session made on one bridge version resumes on the next. The state is the harness's, not the bridge's, so it should. If it does not, a swap starts a new session seeded the way a harness switch is (`pm-chat.md`, Switching harness or model).

## Synergy

| Piece | Reused from |
|---|---|
| Download, checksum and atomic install | #22's `bridge::ensure` |
| The probe | #29's brief start with no prompt |
| Swapping at an idle point | Resume, which every session already uses after a quit |
| The dividers | TurnMarkers (`docs/design/system/components/TurnMarkers`) |
| The setting and its row | The settings page and its layered field (`settings.md`) |
| The capabilities a bridge must keep | SK's parity matrix, lifted |

It would duplicate kelpie's own app updates, which drain and restart everything. A bridge update replaces one process per session and needs neither.

Built in Phase 1, after the PM chat's slices (#9), since a stale Claude bridge is the likeliest thing to break the PM in daily use.

No ADR: the setting makes each choice here reversible.
