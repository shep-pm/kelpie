# Kelpie, shep and SK

Proposed and decided on 2026-11-08. It reopens these parts of `backend.md`:
- **Shape:** SK as a headless daemon, and "closing the app never stops work".
- **Stack:** the Domain, Processes and Install rows.
- **What changes in SK.**
- **Order, 2:** the first tracer bullet.

`backend.md` now points here for each of them.

The code was read at origin/main of shep (`144517474`) and SK (`bdea157f`). Hob traced shep, and the claims below marked ✓ were checked against the source by hand.

## The two rules

1. **Kelpie owns the whole process tree.** Nothing goes into launchd or systemd. When kelpie quits, crashes or is killed, every process it started goes with it, grandchildren included. **Sessions outlive processes:** the next launch finds every conversation waiting, as Claude Desktop does.
2. **The PM is the maintainer's one conversation.** It runs inside kelpie's tree, and the app's chat box is its front end. You pay for one Opus session, not two.
3. Not negotiable: SK keeps building kelpie, as it does today, until kelpie can build itself.

## Recommendation

1. **Don't ship shep or SK.** Kelpie's engine is its own Rust code, hosted by the app.
2. **Take one shep crate as a library: `shep-tree`.** It is 1.6k lines and depends on no other shep crate and no async runtime ✓ (`shep-tree/Cargo.toml`).
3. **Treat SK as a treasure chest.** Lift chunks into kelpie as each slice needs them, tests included. Archive SK at the end. The ADRs, `docs/facts` and agent files come over first, as the spec.
4. **The first tracer bullet is the PM chat**, not SK's board. It means one claude session in kelpie's tree, streaming into the chat box. Its process dies when you quit, and the conversation is waiting when you reopen.

## Why not ship shep: its job is the opposite of ours

Shep is built to keep things running after whatever started them is gone. That's the right design for a pm2-style supervisor, and the wrong one for kelpie.

| What kelpie needs | What shep does | Where |
|---|---|---|
| Never detach | The daemon detaches: `process_group(0)` plus redirected stdio, so it outlives the CLI | `shep-cli/src/launch.rs:73` |
| No init units | `shep startup` writes systemd units and launchd plists with `KeepAlive`, on request | `shep-cli/src/commands/startup/unit.rs:42,77` |
| Children die with their parent | No parent-death mechanism anywhere: no PDEATHSIG, no NOTE_EXIT, no pidfd | grep of the workspace |
| Windows: a closed job kills its tree | `KILL_ON_JOB_CLOSE` is left off on purpose, "so the platforms stay aligned" | `shep-tree/src/job.rs:9-12` ✓ |
| Restarts take everything down | Handover `execve`s in place so the sheep survive a daemon restart | `shep-handover/src/lib.rs:1-4` |
| A crash takes everything down | A SIGKILLed daemon leaves its sheep running under init. The lamb sweep lives in the sheep's task and dies with it. One incident left "70 orphaned `shep daemon` processes, the oldest six days old" | `boot/signals.rs:135` ✓, `docs/specs/deferred-history.md:366` |

Embedding shep's daemon as a library doesn't fix this:
- Only `start`, `start_dog`, `list` and `shutdown` are public ✓ (`supervisor/handle.rs`).
- Without the private "extras" there is no lamb sweep ✓ (`supervisor/sheep.rs:37-45`).
- Its model of sheep, dogs and `SHEP_HOME` files comes along anyway.

**What's worth taking:**
- `shep-tree`:
  - group signals
  - a Windows job a member can't leave
  - a `Sweeper` that dates pids so it never kills a recycled one
  - a stop ladder (TERM, grace, KILL)
- The pty design: rustix, a broadcast tap, raw bytes both ways ✓. This is the recent attach work. It is unix-only, and ConPTY is deferred (`docs/specs/deferred.md:416`).

`shep-tree` needs one upstream change: an opt-in `kill_on_close` for `Job` (shep#1616).

## The proposed tree

```
kelpie (Tauri app: UI + engine in one Rust process)
 ├─ reaper           tiny; holds a pipe from kelpie; on EOF kills everything below
 ├─ PM session       claude, long-lived, stream-json in and out, one per project
 ├─ worker sessions  claude -p / codex, one per work item
 │   └─ tools, cargo, node, dev servers, MCP servers
 └─ pty sessions     the terminal tab
```

Every child gets these four marks:

| Mark | macOS | Linux | Windows |
|---|---|---|---|
| Own process group (Windows: a job) | ✅ | ✅ | ✅ |
| Kernel kills the tree when kelpie dies | ❌ | Partly (PDEATHSIG on direct children) | ✅ job with `KILL_ON_JOB_CLOSE` |
| Orphans come back to kelpie | ❌ | ✅ `PR_SET_CHILD_SUBREAPER` | n/a (a job can't be left) |
| `KELPIE_RUN=<id>` in its environment, found by the sweep | ✅ | ✅ | n/a |

How each way of quitting is covered:

| Way kelpie stops | What cleans up |
|---|---|
| Quit | The engine's own stop ladder (shep-tree), then a sweep |
| App crashes or is SIGKILLed | The kernel closes the reaper's pipe. The reaper kills the recorded groups and sweeps by `KELPIE_RUN` |
| App and reaper both killed | The next launch sweeps by `KELPIE_RUN` and the run file before starting anything |

**Gaps:**
1. A grandchild on macOS can still escape, but only by double-forking **and** scrubbing its environment before a sweep sees it. Nothing ordinary does both. On Linux and Windows nothing escapes.
2. Nothing runs while kelpie is closed, which is the point of the rule. For overnight work, kelpie stays open in the tray, not as a daemon.

**Visibility:** the app shows the live tree (pid, command, CPU, age) and a count in the tray. If kelpie owns it, you can see it.

## Sessions outlive processes

Processes are short-lived. Conversations are not.

| Session | On quit | On the next launch |
|---|---|---|
| PM | The process stops. Kelpie keeps the session id per project | The chat history shows at once, read from the transcript. The process starts with `claude --resume <id>` (or `codex exec resume`) only when you send a message, so nothing runs while you just look |
| Worker mid-turn | Stopped. The turn is lost | Shown as **paused**, with one click to resume. It resumes from its transcript, which SK already does after a runner crash (ADR 0002) |
| pty tab | Gone | Reopens in the same folder. Scrollback isn't kept |

1. Workers come back paused, not running. Quitting is the maintainer saying "stop". A launch that quietly restarts five agents would break the rule from the other side.
2. Both CLIs resume: `claude --resume <id>` works with `--print` and stream-json input, and `codex exec resume` exists. The PM tracer bullet confirms it end to end.

## Updates: drain and resume, not handover

`shep-handover` keeps children alive through an update: it `execve`s a new binary in place, and the sheep stay its children. That breaks no rule, since the pid never changes. Kelpie still doesn't take it now:

| | Handover | Drain and resume |
|---|---|---|
| What's lost | Nothing | At most an in-flight turn, and only if you force it |
| Platforms | Unix only. Windows falls back to stop-and-start | All three |
| Fits a GUI process | Poorly. `execve` replaces the window, the webview's helper processes and the signed app bundle mid-flight | Yes. It is how Claude Desktop updates |
| Code | The blob, the fitness check, the dry-run adoption, and re-adopting every pipe and pty | "Restart to update" plus resume, which kelpie needs anyway |

How an update runs:
1. The update downloads. The app shows "restart to update".
2. On restart, kelpie drains. No new turns start, the turns already running finish, and the PM's turn finishes. A "now" button skips the wait.
3. Kelpie quits, updates and relaunches. The sessions resume as above.

Revisit this if updates mid-turn turn out to hurt. The move then would be to run the engine as a child process of the app. The engine could hand itself over with `shep-handover` while the window restarts on its own, and that option stays open.

## SK: a treasure chest, not a backend

Measured at origin/main:

| | Lines | Note |
|---|---|---|
| All of SK | 184k code + 214k tests | A few months old, written mostly by agents |
| runner | 79k + 101k | 111 modules: pm 8.9k, triage 7.4k, review 6.7k, contributions 5.3k, review_bot 4.1k, turn 3.8k |
| adapters | 22k + 32k | claude, codex, pi, goose, gh, git, srt sandbox, MCP |
| core | ~40k | state, settings 5.2k, ports 6.5k, guard 4.1k, lease 3.1k, board 2.6k, work_item 2.4k |
| Being a shep dog | dog 4.7k, flock 5.7k, upgrade 2.2k, doctor 2.8k, sheep 1.1k | Kelpie needs none of this |

**Why it's a treasure chest:**
1. **SK's process model is exactly what kelpie replaces.**
   - Its runner is a sheep and its workers are lambs.
   - It put each worker in its own process group, and stops leaked until shep grew the sweep. ADR 0005 calls the earlier state "a supervisor inside a supervisee".
   - SK even has its own `tree` crate beside shep-tree.
2. **The design churned.** ADR 0002 is reopened by 0005, 0006, 0013 and 0017. ADR 0003 is reopened by 0004, 0005 and 0016. The code carries the layers.
3. **It is wired to shep throughout.** SK depends on `shep-client`, `shep-channel` and `shep-handover`. Using it as a library brings in the world we're leaving.

**Why not start from zero:**
1. Some of it is hard-won and expensive to learn again:
   - the adapters' stream parsing
   - the sandbox fence and the srt-on-Windows lessons (ADR 0019)
   - per-worker clones
   - the review rules (ADRs 0007 and 0008)
   - leases and rate limits
   - the PM and rulings
2. The ADRs, `docs/facts` and agent files say *why* each piece exists. They are the spec that keeps us from relearning it.

**Lift first, as each slice needs them:**

| Chunk | Lines | Ease |
|---|---|---|
| `protocol` | 1.2k | Easy. Becomes kelpie's own |
| claude adapter: resume argv, the no-session error | part of the 22k | Easy. SK runs claude with `--output-format json`, so kelpie's stream-json reader is new code (#12) |
| work_item, board, lease | ~8k | Medium |
| PM and ruling | ~11k | Hard. Tangled with the runner |
| fence, srt, confine | ~6k | Medium. Lift with its tests, in Phase 1, since the PM runs in crabpot too (ADR 0005) |

**Leave behind:**
- the dog, flock, upgrade and doctor code
- the shep channel and lookout plumbing
- most of `settings` (Rin's two configs)
- GitHub-bot features until a slice asks for them: review_bot, coderabbit, thread_replies

## Order of work

| Phase | Kelpie | SK |
|---|---|---|
| 0, now | Workspace: app and `kelpie-engine` (#5). `kelpie-protocol` and shep-tree wait for Phase 1, where they are first used | Builds kelpie as a shep dog, unchanged |
| 1, tracer (#9) | **PM chat:** one claude session in the tree, sandboxed with crabpot (ADR 0005), streamed over a Tauri Channel into the chat box. Quit kills the process, and a relaunch resumes the conversation. Runs on macOS and Linux | Builds it |
| 2 | The graph (`docs/design/ux.md`), then the drawer (process panel, terminal tab), worker sessions, adapters lifted | Builds it. SK's board view is optional and read-only |
| 3 | Work-item loop, review and leases lifted. Kelpie builds kelpie | Archived |

Why the PM goes first:
1. It's the feature that saves the most (one Opus session instead of two).
2. It proves the tree rule, the stream and the chat UI in one go.
3. It needs nothing from SK's runner.

## Risks

1. **Second-system effect.** The cap: lift only what the next slice needs, and SK keeps shipping meanwhile.
2. **Losing edge cases on extraction.** Every lift brings its tests and a short note on what SK's version handles. The extraction is SK's own workers' job, since they wrote the code.
3. **Windows pty.** Shep's pty is unix-only. The terminal tab needs ConPTY there (portable-pty has it), or the tab waits on Windows.
4. **The phone reaches kelpie only while the desktop app runs.** This reopens `backend.md`'s mobile plan, which assumed an always-on SK. Kelpie in the tray covers overnight. A phone that can wake a closed kelpie would break the rule.
5. **shep loses its biggest user.** That's a relief more than a loss. The kelpie-driven parts (dog plane, lookout panes, channel) can shrink back, which answers the Nov 5 worry that kelpie was bending shep.

## Taken on trust (Hob, not re-read)

1. The daemon's detach, init units and handover details.
2. The pty internals.
3. The 70-orphans incident line.
4. SK module sizes come from a line counter that splits at `#[cfg(test)]`, so treat them as approximate.
