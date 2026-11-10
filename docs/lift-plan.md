# What kelpie lifts from SK

Every SK module, with what happens to it: lifted, adapted, left behind or replaced. SK is shep-pm/shep-kelpie, read at `1dc51c040`. `docs/process-and-sk.md` (SK: a treasure chest) says why kelpie lifts instead of starting over; this file says what, where it lands and when. `.claude/skills/lift-from-sk` has the steps for one lift, and `docs/lifted.md` records each lift once it lands.

**Status:** draft for the maintainer, 2026-11-09. Mapped by Hob's traces of SK, with their file and line citations spot-checked.

## How to use it

Lifting is the default. A module this file marks **lift** or **adapt** is not written from scratch.

1. **Writing an issue** in an area below: find its row. The issue's first `What to build` bullet names the SK paths at `1dc51c040`, the cut, and the SK tests that come with it, and `Touches:` includes `docs/lifted.md`. #54 is the model.
2. **Building an issue** whose area has a row, even if the issue does not name it: follow `.claude/skills/lift-from-sk` for those paths. Writing fresh code where a row says lift needs a reason in the pull request.
3. **Once a lift lands**, change its verdict here to `lifted (#n)` in the same pull request as the `docs/lifted.md` row.

| Verdict | Meaning |
|---|---|
| lift | Comes over whole with its tests. Only names and imports change |
| adapt | The generic part comes over with its tests, cut at the seam named |
| new | SK has nothing for it, so it is built |
| leave | Stays in SK, and the reason is given |

Paths below are SK's, from its root. `runner/` is `crates/runner/src/runner/`, `core/` is `crates/core/src/`, and `adapters/` is `crates/adapters/src/adapters/`.

## How much of SK this keeps

Measured at `1dc51c040`, tests included:

| | Lines |
|---|---|
| All of SK | 396k |
| Mapped to a phase below, lift or adapt | 150k: Phase 1 7k, Phase 2 32k, Phase 3 111k |
| Left behind: the dog, flock, upgrade, doctor, GitHub bot features, the srt glue | 91k |
| The rest: harness adapters, gh, state, settings, test rigs | 155k, mapped in the rows for those areas as they come |

## The seams

Most rows lift once a few seams are cut. Each is already a trait or a type in SK, so a cut is a narrower signature, not a rewrite.

| Seam | SK today | In kelpie |
|---|---|---|
| Running an agent | `ports::Agents::run(&AgentCall, &Ending)` (`core/ports/agent.rs:586`), over each harness's CLI | A session on the harness interface over ACP (#12). `Ending` is the cancel. Nothing above the seam knows a harness |
| Picking an agent | `ports::AgentBoard` (`core/ports/agent_board.rs:14`) over the dog's lease door | A plain struct over `agent_board::Candidate` (`core/agent_board/pick.rs:25`). `rank`, `startable`, `Budget` and the unreachable-board branch go |
| Leases | `ports::Leases` (`core/ports/mod.rs:885`), running totals sent to the dog through shep | The `LeaseBook` held in the engine. Process liveness comes from kelpie's own tree |
| Tickets | `ports::Tickets` (`core/ports/tickets.rs:31`), 16 methods, each taking `&ForgeSlug` | The same trait without `&ForgeSlug`. `open_issues` and `fetch_image` move to a GitHub-only sub-trait |
| Ticket ids | `TicketId::{Github, Local}` (`core/ticket.rs:21`), but triage keys by `u64` (`runner/triage/mod.rs:98`) | `TicketId` everywhere. `docs/facts/ticket-scope.md` in SK budgets the re-keying |
| Labels | Constants in `core/board/mod.rs:28-63` | A labels value from the project's settings |
| CI | `ports::Checks` and the forge's rerun and log calls | A CI port with `head_status`, `rerun` and `logs`. The gate's order stays |
| Posting a ruling | `ruling/post.rs`: a GitHub comment, by the App or `gh` | A ruling sink: a card in the panel and the attention count (`docs/design/ux.md`, Attention) |
| What people wrote | `talk/` and `pm/case/` read GitHub comments, trusted by `AuthorAssociation` | A conversation source yielding author, trusted or not, text and an optional thread |
| Settings | `#[dog_config]` from `shep_client` (`core/settings/mod.rs:17`) | Kelpie's own schema and layers (#26) |
| Sockets and pipes | `core/ipc/mod.rs:15`, through `shep_core::transport` | The same module, on tokio and a named pipe crate |

## Phase 1: the PM chat

| SK source | Verdict | Into | Ticket | Cut, and what comes with it |
|---|---|---|---|---|
| `core/bridge.rs` (`kelpie mcp-connect <socket>`), `adapters/bridge/mod.rs`, `core/ports/mcp.rs`, `core/ipc/` | adapt | `tools/relay.rs`, `tools/door.rs` | #58 | The door: a socket under a name kept in a file, one connection served at a time, the socket removed on drop. `connect` (`core/bridge.rs:228`) is the relay. Its id tracking and the browser's filter (`judge`, `browse::judge`) stay behind, since kelpie's relay parses no MCP and a token gates the door. SK refuses these sockets on Linux (`adapters/claude/escapes.rs:532`, SK ADR 0011), so the Linux TCP door is new. Tests: `adapters/bridge/tests.rs`, `adapters/bridge/client/tests.rs`, `core/ipc/tests.rs` |
| `core/lease/door.rs` | adapt | `tools/door.rs` | #58 | The framing only: one JSON line in, answers out, held while the connection is open |
| `core/profile/mod.rs` (`CREDENTIALS` `:110`, `home_denials` `:494`) | adapt | `sandbox/policy.rs` | #33, #52 | The credential paths stay denied when Read anywhere lifts the read limit |
| `adapters/claude/escapes.rs` | adapt | `sandbox/escapes.rs` | #54 | Already in the issue |
| `runner/pm/notes.rs`, `core/confine/append.rs` | adapt | `session/seed.rs`, the standing-decisions file | #31 | The caps, which SK measured: 32 KiB of notes is about 8k tokens read on every wake, so the log rolls past it, and standing notes stop at about 1k tokens. The standing file is written only by kelpie, so a rule the maintainer gave cannot roll away. The append-only hook holds the PM to appending |
| `adapters/claude/sandbox.rs` (`config_folder` `:158`) | new | `harness/global.rs` | #53 | SK never read the person's own global folder: each call got a config folder of its own. #53 is new work. SK's folder layout, one per ticket and role, is the model for workers in Phase 2 |
| `src/upgrade`, `shep-handover` | leave | | #37, #38 | Kelpie drains and resumes instead of handing over (`docs/process-and-sk.md`, Updates) |

## Phase 2: the graph, the drawer, workers and adapters

| SK source | Verdict | Into | Cut, and what comes with it |
|---|---|---|---|
| `crates/process/src/worktree/` | lift | a `worktree` module | A leaf crate with no shep edge. Drop `workflows.rs` and the `refs/pull/<n>/head` fetch (`contribution.rs:186`), and keep `Remote` to `Origin` until repos the maintainer doesn't own arrive. Keep the trust checks, `clone.rs`, `loose.rs`, `stale_lock.rs`, `stray.rs` and the cache. SK ADR 0019 |
| `crates/worker_folder/` | lift | a `worker_folder` module | Link-safe reads and writes in a folder a hostile writer owns. No shep edge |
| `core/profile/mod.rs` (`WorkerProfile::reach` `:278-363`) | adapt | `sandbox/policy.rs`, a worker's policy | The composition point: `CLONE_DENY`, `CREDENTIALS`, `GH_DENY`, `PUSH_FLAGS`, `push_to_base` and `home_denials` become the worker's crabpot policy |
| `core/fence/` | adapt | `sandbox/fence.rs` | Data plus the check against a trusted commit (`differ`, `matches_main` `:273,304`), which catches what the sandbox's globs miss. `HARNESSES` (`:48`) becomes a list each adapter supplies |
| `core/confine/` | adapt | the ACP permission handler | `judge_path`, `resolve` and `Verdict` (`:26,134,171`) are plain functions: kelpie calls them in process instead of spawning `kelpie confine` as a hook. Each harness's tool JSON moves to its adapter |
| `core/guard/` | adapt | the ACP permission handler | The command reader and the git judge (`shell.rs`, `wrap/`, `git/`, `script.rs`, `judging.rs`, `reading/`, `pause.rs`, `after.rs`). `gh.rs` waits for the GitHub slice |
| `adapters/srt/` | leave | | crabpot replaced it (#33) |
| `runner/flight/` | adapt | the worker supervisor | `Flights`, `Launch`, `news.rs`, `rotation.rs`, `waiting/`, the ceilings (`REVIEW_CEILING` 3600 s), and "count the ceiling from the grant, not the launch" (`:345`). `carry/` is left: no handover. Tests: `flight/tests/mod.rs` |
| `runner/in_flight.rs` | lift | usage | Open a draft at launch, whoever takes it at the end writes the line, never twice. Drop `PacerLine` and the handover marker. Tests: `runner/ledger_tests.rs` |
| `core/usage/` | adapt | usage | The ledger (`mod.rs`, `draft.rs`, `openmetrics.rs`). Drop the per-harness transcript parsers: ACP reports usage |
| `core/pacer.rs`, `runner/pace.rs`, `core/ports/pace.rs` | lift | pacing | A 5-hour window or a week at 100% holds every turn and dispatch until it resets. Key by a limit id instead of SK's `Account` |
| `core/lease/counted.rs`, `core/lease/cli/counted_run.rs` | lift | `kelpie lease run` | A capacity-N lease held by connection liveness, so a worker's `cargo test` waits its turn. No shep edge |
| `core/forwarder/`, `adapters/forwarder/` | lift | local-model workers | Passes only chat completions and swaps in the gateway key, so a model server's admin calls and the key stay out of the sandbox |
| `runner/off_lock.rs`, `runner/lock_hold.rs` | adapt | the engine's lock, if it keeps one | Re-check the item after a read with the lock let go. Log any hold over 5 s with the line that took it |
| `core/settings/agents/facts.rs` (`HarnessFacts` `:30`, `Harness::facts` `:89`, a row per harness from `:130`), `core/settings/agents/mod.rs` (`Harness` `:76`, `AgentHarness` `:112`, `ModelServer`) | adapt | `harness/` | One table of what each harness can do (MCP, hooks, who names the session, whether it needs a model server) is how kelpie stays a hub. Keep the table and its refusals (`Settings::under_worker_fence`, with its tests in `agents/tests.rs`). Drop the transcript formats, `DoctorProbe` and the handover facts (`FORWARDER`, `TRUST`): ACP and draining replace them |
| `adapters/claude/`, `adapters/codex/`, `adapters/pi/`, `adapters/goose/`, `adapters/harness/`, `adapters/codex_usage.rs` (about 22k lines) | adapt, file by file | `harness/` | Most of this is launching a CLI and parsing its stream, which ACP replaces. What may carry over: each harness's config folder (`claude/sandbox.rs`), usage readers, and `harness/`'s env and policy files. Hob's trace of these timed out twice, so read them when the first worker adapter is written, not before |
| The graph | new | | SK has none. CMM, then kelpie's own indexer |

## Phase 3: the work-item loop, review, leases and the PM board

Port `runner/several.rs` first. It is the acceptance suite for the loop: several work items at once, through the runner's stand-ins, asserting only what the outside world sees. Replace its lease book with kelpie's.

### Work items, board and dispatch

| SK source | Verdict | Cut, and what comes with it |
|---|---|---|
| `core/work_item/{seat,heads,timings,tally,disputes,follow_ups,weight}.rs` | lift | No edge outside `core::ports`. `timings/` is the cleanest module in SK |
| `core/work_item/spend/` | lift | Drop `QwenTally`. `unpriced_calls` matters under ACP, where most agents report no dollars |
| `core/work_item/mod.rs` | adapt | Split the fields: the loop (`Phase`, `Turn`, `Seat`, `CallRecord` and the rest) from the forge's facts (pull request, fork, threads, labels), kept beside it by ticket |
| `core/work_item/review.rs` | adapt | `ReviewStage` keeps `Round`, `Found`, `Fixing`, `Pushing`, `SecondLook`. `Summon`, `Summoned`, `Settling` are review bots' |
| `core/work_item/known.rs` | adapt | The shape `foreign_change(believed, seen)`, over any view of external state |
| `core/board/mod.rs` | adapt | `pick`, `keep_order`, `rule_order`, `Skip`, `Pick` verbatim: pure functions over `ReadyIssue`. Labels become settings. SK ADR 0014 |
| `core/board/check.rs` | lift | Kelpie's issues use the same format, so the check comes whole. `HARNESSES` becomes the harnesses kelpie has |
| `core/board/briefing/` | adapt | The overlap table (`Files`, `shared`), items, rulings and the ready queue. A briefed PM avoided more conflicts at half the cost (SK `docs/facts/the-board-briefing.md`). Drop agents, plans, credit |
| `core/agent_board/` | adapt | `typical.rs`, `record.rs`, `wait.rs`, `fit.rs` and `pick::pick` whole. `rank` and `startable` go with the door. SK ADRs 0018, 0020 |
| `runner/dispatch/` | adapt | The precedence loop, one work item per pass. Drop the unreachable-board branch and the adopt, rework, contribution and triage prefixes until those land |
| `runner/slots.rs` | lift | A parked item frees its slot (SK ADR 0007). `seat()` (`:25`) is a free function. Tests: `slots_tests.rs` |
| `runner/claim.rs` | adapt | `Notes`: a warning that stands until it ends, told once. The label writes stay with GitHub |
| `runner/finishing/` | adapt | Finish, pause and the saved state that survives a restart. `take_stop` and `stop_failed` become kelpie's own quit |
| `runner/queued.rs` | adapt | The pattern: guards, read with the lock let go, guards again, apply |

### The gate, review and merge

| SK source | Verdict | Cut, and what comes with it |
|---|---|---|
| `runner/gate/` | adapt | The order (foreign change, agents' files, catch-up, CI, unread head, threads, merge), `back_to_worker`, `settled`, `lag.rs`, `borrowed.rs`, `unread.rs`. `queued/` and `rerun/` are GitHub Actions'. SK ADR 0008 |
| `core/ports/findings.rs`, `core/ports/reviewer.rs` | lift | `Finding` is what every reviewer returns. `Reviewer` is the port to implement with an ACP session |
| `runner/review/` | adapt | One pass, each reviewer once, at most one fix turn each, then the gate (SK ADRs 0006, 0007). The bot stages wait for the GitHub slice. Tests: `review/tests/mod.rs` |
| `runner/reviewers_check.rs` | adapt | `left_in`, `check_local` |
| `runner/merge/` | adapt | `drop_work_item`'s refusals, `withdraw` (a yes holds only for the head it was asked about), `refused`, `merged`, `regate`, `cleanup`. `queue.rs` goes behind a merge queue trait |
| `runner/close_merged.rs` | lift | Through `Tickets`. `FORGE_CLOSE_WAIT` (600 s) was never measured: SK's `docs/facts/ci-and-the-merge.md` leaves it to the maintainer |
| `runner/rework/` | adapt | The review handed over verbatim, the first prompt, which refusals are settled, and starting from `origin`'s head, never a rewrite |
| `runner/adopt/` | adapt, later | With repos the maintainer doesn't own (`ux.md`, Later). `own_push`, `pushed_by_someone_else`, `Failing`. SK ADR 0010 |
| `runner/reread/` | adapt | `changed.rs` and the shape: validate as a start, then swap, and name what waits for the next start |

### The PM and rulings

| SK source | Verdict | Cut, and what comes with it |
|---|---|---|
| `core/pm_answer/` | lift | No edges at all |
| `core/state/pm_holds.rs` | lift | A hold ends if the ticket's text changed or nothing runs |
| `runner/ruling/history.rs` | lift | `rulings.jsonl`. `RulingName::named` tolerates a retired kind |
| `core/state/ruling.rs` | adapt | The pull request number becomes the ticket id |
| `runner/pm/wakes.rs` | adapt | `Wake`, `StuckState`, `same`, `held` and the back-off are data. `pm_noticed` becomes a diff of two snapshots |
| `runner/pm/answer/` | adapt | Last object wins, each field dropped and named on its own |
| `runner/pm/act.rs` | adapt | Validate, then apply. A refusal goes back to the PM |
| `runner/pm/rulings.rs`, `runner/pm/asked.rs` | adapt | What the PM may answer, and the maintainer's three asks (recommend, options, decide) as buttons instead of thread replies |
| `runner/pm/question.rs`, `runner/ruling/question.rs` | adapt | Every ruling says what a yes does and what a no does. The answer command becomes data |
| `runner/ruling/mod.rs` | adapt | `decide` and `Move` are free functions. Posting goes through the ruling sink |
| `runner/pm/mod.rs` | adapt | The wake scheduler, with the board, logs and launch as three closures |
| `runner/pm/case/`, `runner/talk/` | adapt | The quarantine of what people wrote, through the conversation source |

### Tickets and triage

| SK source | Verdict | Cut, and what comes with it |
|---|---|---|
| `core/ticket.rs`, `core/ports/tickets.rs`, `core/ports/issue.rs` | adapt | The seam above |
| `core/local_tickets/` | lift | Tickets as Markdown files, behind `Tickets` |
| `core/state/triage.rs`, `core/settings/triage.rs` | adapt | Re-keyed by `TicketId` |
| `runner/triage/` | adapt | Its posts go through `Tickets::comment_ticket`, which `close_merged` and `follow_up` already use, instead of the GitHub App |
| `runner/follow_up/`, `runner/parent/`, `runner/dependencies/` | adapt | Through `Tickets` |
| `core/lease/{book,window,saved,wire}.rs` | adapt, with review bots | One holder per kind, and `window.rs`: N calls per span, with the service's own retry time winning |

### GitHub

Not yet mapped: Hob's trace of it timed out twice. What is known:

| SK source | Verdict | Cut, and what comes with it |
|---|---|---|
| `core/settings/mod.rs` (`ForgeSlug` `:774`, `forge_of` `:811`) | lift | A repo as `owner/name`, parsed from a remote URL |
| `adapters/gh/` (about 9k lines: issues, pulls, runs, suites, rules, thread, talk, tickets, queue, rate limit) | adapt | The forge adapter behind `Tickets`, the CI port, the merge queue trait and the conversation source. Map it file by file when the GitHub slice is designed |
| `core/ports/rate_limit/`, `core/forge_ledger/`, `core/metrics/forge.rs` | adapt | Pacing forge calls against the rate limit, and the ledger of each call |
| `core/test/forge/` | lift | The fake forge every runner test uses. It counts each call, and can fail one or hold it open mid-pass. Port it with the first test that needs it |
| `runner/main_ci/`, `runner/ci_failure/`, `runner/checks/` | adapt | Through the CI port |

## Left behind

| SK | Why |
|---|---|
| `crates/dog`, `src/flock`, `src/sheep`, `src/doctor` | SK being a shep dog |
| `src/upgrade`, `shep-handover`, `runner/flight/carry/` | Kelpie drains and resumes |
| `core/shep_home.rs`, `core/shepherd/`, `core/metrics/shepherd_sink.rs` | Shep's home and channel. The `Sink` port is the generic half |
| `core/webhook.rs`, `core/totp/`, `runner/replies/` | ntfy and Discord. Kelpie's prompts are in its window |
| `runner/review_bot/`, `runner/coderabbit/`, `runner/thread_replies/`, `runner/voice/`, `runner/contributions/` | GitHub bot features, until a slice asks for them |
| `runner/token_wait/`, the login refresh | GitHub App tokens |
| `adapters/srt/` | crabpot |

## What kelpie supplies where shep was

1. **Process liveness.** SK's lease book reclaims a lease when shep says its holder died. Kelpie's tree knows directly.
2. **The stop and the sweep.** Shep walks the tree and kills what is left (SK ADR 0005). Kelpie's own tree does it (ADR 0001). Keep `runner/flight/` and `adapters/stop.rs`, SK's half of it.
3. **Time budgets.** `runner/gpu.rs` samples on a thread because shep gives an action 3 s, and `door_pace.rs` gives the dog 2 s. Set kelpie's own budgets before copying either number.

## Numbers that carry a measurement

Read the source before changing one.

| Constant | Value | Behind it |
|---|---|---|
| `CHECKS_SETTLE` (`runner/gate/mod.rs:27`) | 120 s | A check set registers one check at a time (`docs/facts/ci-and-the-merge.md`) |
| `FORGE_CLOSE_WAIT` (`runner/close_merged.rs:35`) | 600 s | Not measured. The maintainer's call |
| `NOTES_MAX` (`runner/pm/notes.rs`) | 32 KiB | About 8k tokens on every wake |
| `READ_AGAIN` (`runner/gate/unread.rs:22`) | 30 min | A session resumed past an hour loses its prompt cache (SK ADR 0007) |
| `LOCAL_FAILURES_DOWN` (`core/work_item/local.rs:8`) | 2 | A local reviewer that reviewed nothing twice is dropped |
| `TALK_ROUNDS` (`core/work_item/talked.rs:10`) | 3 | Endless commentary becomes a ruling |
| `STALLED_RESUMES` (`runner/pm/rulings.rs:32`) | 2 | |
