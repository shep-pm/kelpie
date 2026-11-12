# What kelpie lifts from SK

Every SK module, with what happens to it: lifted, adapted, left behind or replaced. SK is shep-pm/shep-kelpie, read at `1dc51c040`. `docs/process-and-sk.md` (SK: a treasure chest) says why kelpie lifts instead of starting over; this file says what, where it lands and when. `.claude/skills/lift-from-sk` has the steps for one lift, and `docs/lifted.md` records each lift once it lands.

**Status:** confirmed by the maintainer on 2026-11-09. Mapped by Hob's traces of SK, with their file and line citations spot-checked.

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
| `core/profile/mod.rs` (`CREDENTIALS` `:110`, `home_denials` `:494`) | adapt, landed (#33, #52) | `sandbox/policy/` | #33, #52 | The credential paths stay denied when Read anywhere lifts the read limit, with macOS's keychain folder added and the folder holding every session's state denied whole and the session's own carved back (`docs/lifted.md`). `home_denials`' collapse of several homes comes with the worker policy row |
| `adapters/claude/escapes.rs` | lifted (#54) | `crates/kelpie-engine/tests/sandbox/escapes.rs` | #54 | The PM's escapes, run by `tests/sandbox.rs`: a lib test cannot be crabpot's Linux helper. The worker's escapes wait for workers (`docs/lifted.md`) |
| `runner/pm/notes.rs`, `core/confine/append.rs` | adapt | `session/seed.rs`, the standing-decisions file | #31 | The caps, which SK measured: 32 KiB of notes is about 8k tokens read on every wake, so the log rolls past it, and standing notes stop at about 1k tokens. The standing file is written only by kelpie, so a rule the maintainer gave cannot roll away. The append-only hook holds the PM to appending |
| `adapters/claude/sandbox.rs` (`config_folder` `:158`) | new | `harness/global.rs` | #53 | SK never read the person's own global folder: each call got a config folder of its own. #53 is new work. SK's folder layout, one per ticket and role, is the model for workers in Phase 2 |
| `src/upgrade`, `shep-handover` | leave | | #37, #38 | Kelpie drains and resumes instead of handing over (`docs/process-and-sk.md`, Updates) |

## Phase 2: the graph, the drawer, workers and adapters

| SK source | Verdict | Into | Cut, and what comes with it |
|---|---|---|---|
| `crates/process/src/worktree/` | lift, landed (#70, #71, #72) | a `worktree` module | A leaf crate with no shep edge. Drop the `refs/pull/<n>/head` fetch (`contribution.rs:186`), and keep `Remote` to `Origin` until repos the maintainer doesn't own arrive. Keep the trust checks, `clone.rs`, `loose.rs`, `stale_lock.rs`, `stray.rs` and the cache. SK ADR 0019. The reads landed first, as `crates/kelpie-engine/src/worktree/` (`in_process.rs`, `loose.rs`, `WorktreeError` and the runner), then `prepare` with the clone, the trust checks and `Remote::Origin` (`mod.rs`, `clone.rs`, `common.rs`, `stale_lock.rs`, `remote/`), then `remove` with `stray.rs`, `writable.rs` and `cache.rs`, with `holder_of` and `holds_work` (`mod.rs`, `tests.rs`, `remote/mod.rs`'s `delete`), then `catch_up.rs`, `conflicts.rs`, `workflows.rs` and `abort_merge` (lifted, #219), then `changed.rs`'s `changed_files` (lifted, #228): `docs/lifted.md`. `adopt.rs`, `changed.rs`'s `moved_since`, `active.rs`, `borrowed.rs`, `contribution.rs` and `push.rs` are still SK's, and come with Phase 3's rework, review and fork rows |
| `crates/worker_folder/` | lifted (#69) | a `worker_folder` module | Link-safe reads and writes in a folder a hostile writer owns. No shep edge |
| `core/profile/mod.rs` (`WorkerProfile::reach` `:278-363`) | lifted (#74) | `sandbox/worker/mod.rs`, a worker's policy | The composition point: `CLONE_DENY`, `CREDENTIALS`, `GITHUB` and `home_denials` became the worker's crabpot policy, and `cargo_credentials` and `toolchain_homes` came from `dep_sources.rs` into `sandbox/worker/toolchain.rs`. `env_for` and `git_config` wait for the token, and `GH_DENY`, `PUSH_FLAGS` and `push_to_base` for the guard, each in its own issue |
| `core/fence/` | lifted (#73) | `sandbox/fence/` | Data plus the check against a trusted commit (`differ`, `matches_main` `:273,304`), which catches what the sandbox's globs miss. `HARNESSES` (`:48`) becomes a list each adapter supplies |
| `core/confine/` | lifted (#76) | `confine/`, `permission/`, the ACP permission handler | `judge_path`, `resolve` and `Verdict` (`:26,134,171`) are plain functions: kelpie calls them in process instead of spawning `kelpie confine` as a hook. Each harness's tool JSON moves to its adapter. `write.rs` (project hooks) stays, and `append.rs` is #31's |
| `core/guard/` | adapt | the ACP permission handler | The command reader and the git judge (`shell.rs`, `wrap/`, `git/`, `script.rs`, `judging.rs`, `reading/`, `pause.rs`, `after.rs`). The command reader (`shell.rs`, `wrap/`, `script.rs`) and `local_paths/` are lifted (#75), and the judge (`Checkout`, `judge`, `judging.rs`, `git/`) is lifted (#77). Still to come: `gh.rs` with the GitHub slice, `reading/`, `pause.rs`, `after.rs`, `git/hooks.rs` with project hooks, the fork and pin flags, and the project's base-branch setting, which `Project::base` takes from the caller until then |
| `adapters/srt/` | leave | | crabpot replaced it (#33) |
| `runner/flight/` | adapt | the worker supervisor | `Flights`, `Launch`, `news.rs`, `rotation.rs`, `waiting/`, the ceilings (`REVIEW_CEILING` 3600 s), and "count the ceiling from the grant, not the launch" (`:345`). `carry/` is left: no handover. Tests: `flight/tests/mod.rs` |
| `runner/in_flight.rs` | lifted (#158) | usage | Open a draft at launch, whoever takes it at the end writes the line, never twice. Drop `PacerLine` and the handover marker. Tests: `runner/ledger_tests.rs` |
| `core/usage/` | lifted (#158) | usage | The ledger (`mod.rs`, `draft.rs`, `openmetrics.rs`). Drop the per-harness transcript parsers: ACP reports usage |
| `core/pacer.rs`, `runner/pace.rs`, `core/ports/pace.rs` | lifted (#79) | `pacer/` | A 5-hour window or a week at 100% holds every turn and dispatch until it resets. Key by a limit id instead of SK's `Account`. Dispatch waits its own row |
| `core/lease/counted.rs`, `core/lease/cli/counted_run.rs` | lifted (#81) | `lease/counted.rs`, `lease/run/` (`mod.rs`, `client.rs`, `tests.rs`), `kelpie lease run` | A capacity-N lease held by connection liveness, so a worker's `cargo test` waits its turn. No shep edge. The `cargo-test` name and its capacity came with #79 as the `lease-capacity` setting. The command runs through the engine's own process tree, and each worker session gets a socket of its own (`docs/lifted.md`) |
| `core/forwarder/`, `adapters/forwarder/` | lift | local-model workers | Passes only chat completions and swaps in the gateway key, so a model server's admin calls and the key stay out of the sandbox |
| `runner/off_lock.rs`, `runner/lock_hold.rs` | adapt | the engine's lock, if it keeps one | Re-check the item after a read with the lock let go. Log any hold over 5 s with the line that took it |
| `core/settings/agents/facts.rs` (`HarnessFacts` `:30`, `Harness::facts` `:89`, a row per harness from `:130`), `core/settings/agents/mod.rs` (`Harness` `:76`, `AgentHarness` `:112`, `ModelServer`) | adapt | `harness/` | One table of what each harness can do (MCP, hooks, who names the session, whether it needs a model server) is how kelpie stays a hub. Keep the table and its refusals (`Settings::under_worker_fence`, with its tests in `agents/tests.rs`). Drop the transcript formats, `DoctorProbe` and the handover facts (`FORWARDER`, `TRUST`): ACP and draining replace them |
| `adapters/claude/`, `adapters/codex/`, `adapters/pi/`, `adapters/goose/`, `adapters/harness/`, `adapters/codex_usage.rs` (about 22k lines) | adapt, file by file | `harness/` | Most of this is launching a CLI and parsing its stream, which ACP replaces. What may carry over: each harness's config folder (`claude/sandbox.rs`), usage readers, and `harness/`'s env and policy files. Hob's trace of these timed out twice, so read them when the first worker adapter is written, not before |
| `runner/pull_request_file.rs` (`parse`, `Text`, `Untitled`, `Refused`, `told`, `open_handed_over`, `edit_handed_over`) | lifted (#157) | `worker/hand_over.rs` | The hand-over read, its title check, and the draft opened or edited, over `Pulls` and `GithubTickets` instead of SK's `Forge`. The push check is `Pulls::branch_head` against the worktree's head, which SK's caller did outside the file. `Runner`'s notes and `StepReport` become an `Outcome` its caller reports |
| `runner/paragraphs/` (`join` and its line rules) | lifted (#157) | `worker/paragraphs/` | Whole, with its tests. The fixture's command is kelpie's own |
| `core/profile/mod.rs` (`HAND_OVER` `:44`, `ENDS_WITH_RESOLVES` `:50`) | lifted (#157) | `worker/instructions.rs`, what `{pull_request}` expands to | The hand-over text, with `{file}` and `{ends}` filled in. `OPEN_YOURSELF` stays behind: kelpie never hands a worker the maintainer's login, so every worker hands over. `RESOLVES` and `COMMIT_ONLY` wait for the fork row |
| The graph | new | | SK has none. CMM, then kelpie's own indexer |

## Phase 3: the work-item loop, review, leases and the PM board

Port `runner/several.rs` first. It is the acceptance suite for the loop: several work items at once, through the runner's stand-ins, asserting only what the outside world sees. Replace its lease book with kelpie's. Shape lifted (#216): `src-tauri/src/several/`, with the gate's cases added by each slice.

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
| `runner/slots.rs` | lifted (#79) | A parked item frees its slot (SK ADR 0007). `seat()` (`:25`) is a free function. Tests: `slots_tests.rs`. Brought forward into #79 over kelpie's workers: `seat()` and its rule came, the rulings cap and the board's overlap skips stay here |
| `runner/claim.rs` | adapt | `Notes`: a warning that stands until it ends, told once. The label writes stay with GitHub |
| `runner/finishing/` | adapt | Finish, pause and the saved state that survives a restart. `take_stop` and `stop_failed` become kelpie's own quit |
| `runner/queued.rs` | adapt | The pattern: guards, read with the lock let go, guards again, apply |

### The gate, review and merge

| SK source | Verdict | Cut, and what comes with it |
|---|---|---|
| `runner/gate/` | adapt, order lifted (#225) | The order (foreign change, agents' files, catch-up, CI, unread head, threads, merge), `back_to_worker`, `settled`, `lag.rs`, `borrowed.rs`, `unread.rs`. `queued/` and `rerun/` are GitHub Actions'. SK ADR 0008. Lifted in #225 as `crates/kelpie-engine/src/gate/`: the order (`check_ci`, `lag.rs`, `queued/`, `queued/none.rs`), `ci_failed` and `rerun/`, the ruling half of `catch_up.rs`, `fix_attempts.rs` and the red prompt, on plain values: `docs/lifted.md`. `borrowed.rs`, `unread.rs`, `threads.rs` and the rest of the pass stay SK's for notes 2 and 3 |
| `core/ports/findings.rs`, `core/ports/reviewer.rs` | lifted (#253) | `Finding` is what every reviewer returns. `Reviewer` is the port to implement with an ACP session. Lifted in #253 as `crates/kelpie-engine/src/review/` (`finding.rs`, `reviewer.rs`, `fake.rs`): `docs/lifted.md`. `LocalRound` and the rest of the command reviewers' half stay SK's |
| `runner/review/` | adapt | One pass, each reviewer once, at most one fix turn each, then the gate (SK ADRs 0006, 0007). The bot stages are deferred to a slice of their own (`docs/design/review.md`, Deferred: bot reviewers). Tests: `review/tests/mod.rs` |
| `runner/review/prompts.rs`, `runner/review/calls.rs` (`severity_tag`) | lifted (#258) | A reviewer's prompt and the notes after it that are not note 3's. Lifted as `crates/kelpie-engine/src/review/prompt.rs`: `docs/lifted.md` has the row. `asked_note`, `plan_note`, `standing_note` and `notes` stay behind for note 3 |
| `runner/review/criteria.rs` | lifted (#258) | What the issue asks for, cut to 4000 bytes. Lifted as `crates/kelpie-engine/src/review/criteria.rs` (`acceptance`, `cut`): `docs/lifted.md` has the row. The ticket port's read of the issue is the pass's |
| `runner/review/findings.rs`, `runner/review/attached.rs` (`TESTS_FOLDER`, `tests_folder`) | lifted (#258) | The findings file, the deferred and disputed files the worker writes, and the fix turn's prompts. Lifted as `crates/kelpie-engine/src/review/findings_file.rs`: `docs/lifted.md` has the row. `THREAD_LINES`, thread numbering and `drop_deferred` stay behind, the bots' and the merge's |
| `runner/reviewers_check.rs` | adapt | `left_in`, `check_local` |
| `runner/merge/` | adapt, merge lifted (#229) | `drop_work_item`'s refusals, `withdraw` (a yes holds only for the head it was asked about), `refused`, `merged`, `regate`, `cleanup`. `queue.rs` goes behind a merge queue trait. Lifted in #229: `mod.rs`'s `merge`, `withdraw`, `refused` and `regate`, `call.rs` and `queue.rs`, as the engine's `gate/merge.rs` on plain values and the app's `gate/merge.rs`: `docs/lifted.md`. `auto.rs`, `parent.rs`, drop and the PM's yes stay SK's for note 3, and closing and `cleanup` are lifted in #230 (`runner/merge/cleanup.rs`, below) |
| `runner/close_merged.rs` | lifted (#230) | Through `Tickets`, as the engine's `gate/after.rs` on plain values and the app's `gate/after.rs`: `docs/lifted.md`. `FORGE_CLOSE_WAIT` (600 s) was never measured: SK's `docs/facts/ci-and-the-merge.md` leaves it to the maintainer. The sweep of the ready list stays behind with dispatch |
| `runner/merge/cleanup.rs` | lifted (#230) | Its worktree, build folder and remote branch removal is the engine's `worktree::remove`, called from the app's `gate/after.rs`: `docs/lifted.md` |
| `runner/rework/` | adapt, red prompt lifted (#225) | The review handed over verbatim, the first prompt, which refusals are settled, and starting from `origin`'s head, never a rewrite. The red prompt and the merge queue prompt are lifted in `crates/kelpie-engine/src/gate/prompt.rs` (#225), and `back_to_worker`'s record is `Kept::fix_turn_sent`. The rest waits for the maintainer's review rework |
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
| `core/ticket.rs`, `core/ports/tickets.rs`, `core/ports/issue.rs`, `core/ports/forge_error.rs` | lifted (#139) | The seam above. The reads are lifted, and so are `comment_ticket` and `close_issue` (#222). The other writes (`issue_comments`, `add_blocked_by`, `add_sub_issue`, `finished_parents`, `sub_issues`, `file_ticket`, `set_issue_label`) and `fetch_image` stay for Phase 3 |
| `core/local_tickets/` | lift | Tickets as Markdown files, behind `Tickets` |
| `core/state/triage.rs`, `core/settings/triage.rs` | adapt | Re-keyed by `TicketId` |
| `runner/triage/` | adapt | Its posts go through `Tickets::comment_ticket`, which `close_merged` and `follow_up` already use, instead of the GitHub App |
| `runner/follow_up/`, `runner/parent/`, `runner/dependencies/` | adapt | Through `Tickets` |
| `core/lease/{book,window,saved,wire}.rs` | adapt, with review bots | One holder per kind, and `window.rs`: N calls per span, with the service's own retry time winning |

### GitHub

Mapped file by file for `docs/design/tickets.md` on 2026-11-10. SK's `adapters/gh/` is `gh/` below. One `Gh` struct answers `Forge` (47 methods, `ports/mod.rs:152`), `Tickets` and `Notifications`, and every call goes through `gh_as` (`gh/mod.rs:422`): one `gh` child per call, as the login `gh` holds, or as the App through a `hosts.yml` folder of its own for six reads. SK has no `Checks` port: `Checks` is an enum inside `PullRequest` (`ports/mod.rs:666`), so the CI port is cut out of `Forge`.

| SK source | Verdict | Cut, and what comes with it |
|---|---|---|
| `core/settings/mod.rs` (`ForgeSlug` `:774`, `forge_of` `:811`) | lifted (#84) | A repo as `owner/name`, parsed from a remote URL. It is `github::slug` in kelpie, where every GitHub call names its repo with it |
| `adapters/github/{mod,tokens,api}.rs`, `adapters/adapters/{github,curl}.rs`, `test/github.rs` (`FakeGithub`, `FakeSigner`) | lifted (#84) | The App store and the token path: `Apps`, `AppTokens`, `jwt`, `pushes_only`, `mint`, `installation`, `access_token`, `Signer`, `Grant`, `token_request`, and curl and openssl through the process tree, as `github/`. The forge's writes, `voice/` and the ports they answer stay for the slice that opens the pull request |
| `core/profile/login.rs`, `core/profile/mod.rs` (`env_for` `:409`, `git_config` `:467`) | lifted (#84) | The worker's `gh` folder and the environment that points git at it, as `worker/login.rs` and `worker::start`'s `credential`, with `login::helper_on_path` reading the `PATH` through `harness::on_path` so a machine with no `gh` mints nothing and says the worker cannot push, and `login::clear` taking out what an earlier turn left where a turn has no credential. `login_token`, the Windows trusted-dir parameters and `housekeeping::PARAMETERS` stay out: kelpie never hands a worker the maintainer's login |
| `gh/tickets.rs`, `gh/issues.rs` (the open-issue listing and `issue()` from `gh/mod.rs`) | lifted (#142) | The `Tickets` port's GitHub source, `github::gh::GithubSource`: `issue`, `ready_issues`, `open_issues` and the new `closed_issues`, held by the rate limit and cached by ETag. `account_created` goes to the GitHub-only trait with triage |
| `gh/issues.rs` (`sub_issues`, `finished_parents`, the other writes; `comment` and `close_issue` are lifted, #222), `gh/blocked.rs` | lift, Phase 3 | `blocked.rs` resolves an issue's database id first and refuses a pull request as a target. Each comes with its Phase 3 caller |
| `gh/board.rs` (`ready_issues`) | lifted (#142) | `gh issue list --limit 1000` feeds the board, its label from settings. Kelpie asks for `number,title,labels,author,createdAt`: the blockers, parent and sub-issues come with Phase 3's dispatch. gh lists only 50 blockers, so the board takes one counted but not listed as open |
| `gh/board.rs` (`open_pull_requests`) | lifted (#143) | One paged GraphQL query that feeds the Review column now and the gate later. Kelpie asks for the number, head branch and commit, base branch, labels, draft flag, closing references and the last commit's check rollup and suites, so `open_pulls` reads the head, labels and checks (#218). Review threads and fork flags come with Phase 3's gate |
| `gh/attachment.rs` | adapt | `fetch_image`, GitHub-only: host-pinned curl with the token on stdin. `is_github_attachment` lifts as is |
| `gh/pulls.rs` | lifted (#156) | Opens the draft pull request and edits its text through `gh api` with `-f` fields, so a leading dash is safe. As the maintainer, never the App |
| `gh/mod.rs` (the check rollup `:558-719`), `gh/suites.rs`, `gh/runs/` | lifted (#217) | The CI port's `head_status`, `rerun` and `logs`. Suites turn a workflow that started no job red and a held one pending. `failure_tail` and its filters are pure and lift with `runs/tests.rs` and the rollup tests in `gh/tests.rs` |
| `gh/mod.rs` (`merge`, `mark_ready`, `disable_auto_merge`, the state read of `pull_request`), `gh/mod.rs` (`comment`), `gh/rules.rs`, `gh/access.rs` (`can_push`) | `merge`, `mark_ready`, `disable_auto_merge` and the state read lifted (#220), the rest adapt, Phase 3 | Merge commit pinned to the head with an empty body. Rulesets then classic protection for up to date |
| `gh/queue.rs` | lifted (#221) | A pure parser of one GraphQL read, with its tests: `isInMergeQueue`, `autoMergeRequest` and the removal events, as `github::gh::queue`. `QueueStanding` moves out of SK's `ports` into `ticket::queue`, behind the `MergeQueue` trait |
| `gh/talk.rs`, `gh/thread.rs`, `gh/review.rs` (`reviewed`) | adapt, Phase 3 | The conversation source. Trusted is an `authorAssociation` of owner, member or collaborator, or a collaborator's write permission, and a bot never answers. `comment_marks` and `Thumb` stay unless rulings return to GitHub |
| `gh/mod.rs` (`gh_as`, `gh_command`), `gh/failed.rs`, `gh/rate_limit.rs`, `gh/review.rs` (`viewer`) | lifted (#140) | One `gh` child per call through the process tree with `GH_DEBUG=api`, errors from `gh`'s stderr with `Retry-After` and the reset read into `Cooldown`, `gh api rate_limit`, which counts against no limit, and `gh api user`'s login, as `github::gh`. The config-folder path and `login_token` stay behind |
| `gh/conditional/` | lifted (#141) | The ETag cache keyed by path, in memory, as `github::gh::conditional`. The login in the key, `pages_in` and the 304 count for the ledger stay behind |
| `gh/app.rs`, `gh/review_bot.rs`, `gh/notifications.rs`, `gh/review.rs` (`login_token`), `gh/access.rs` (`last_bot_review`) | leave | App reads for a second budget, which one process does not need, review bots, the notifications verb, and the maintainer's login handed out |
| `core/ports/rate_limit/` (`RateHeld`, `says_used_up`, `FALLBACK`, `ForgeHold`, `LocalPace`), `core/ports/pace.rs` | lifted (#141) | A used-up limit holds every call until it lifts, and calls are paced at 60 a minute and 5 a second, as `github::gh::hold`. The fair share between projects, `AppReads`, `PacedVoice` and the ledger hooks stay behind |
| `core/forge_ledger/`, `core/metrics/forge.rs` | adapt | The ledger of each call |
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
