# The gate and the merge

Phase 3's first slice (`docs/process-and-sk.md`, Order of work): what happens to a worker's pull request between the draft kelpie opens and the issue closing. Kelpie keeps the branch on the latest base branch, waits for CI, sends red runs and conflicts back to the worker, asks the maintainer for the merge, merges, closes the issue and cleans up. Review (note 2) and rulings with dispatch (note 3) build on it. It places itself in `docs/design/tickets.md` (the Review column, the move table) and `docs/design/ux.md` (Attention), and lifts from SK's `runner/gate/`, `runner/merge/`, `runner/rework/`, `runner/close_merged.rs` and the worktree's `catch_up.rs` and `conflicts.rs`, row by row from `docs/lift-plan.md` (Phase 3, The gate, review and merge). Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-11: the merge yes is Merge on the ticket or the Review to Done drag, and a hand merge counts; a Review ticket shows one status line; a fix turn moves the ticket to Building; until note 3, a gate ruling is answered with a Note or on GitHub.

## Use case

Today the ticket stops at Review: the maintainer marks the draft ready on GitHub, watches CI, tells the worker what broke, rebases by hand and merges.

1. A worker's turn ends and kelpie opens the draft pull request. The ticket moves to Review.
2. Kelpie brings the branch up to the latest base branch and waits for CI on the head.
3. CI goes red. Kelpie hands the failing logs to the worker, which pushes a fix, and the gate starts again on the new head. A conflict on catch-up goes to the worker the same way.
4. CI goes green on the latest base. The ticket asks the maintainer for the merge.
5. The maintainer says yes. Kelpie marks the draft ready, merges the head it was asked about, closes the issue if GitHub has not, and removes the worktree and the branch. The ticket moves to Done.
6. Anything the worker will not fix on its own becomes a ruling on the ticket, with its reason.

Why it matters: this is the loop SK runs for kelpie today. With it, kelpie takes a ticket from Ready to Done itself, and SK can be archived once review, rulings and dispatch land too.

## Design

### Scope

| In this slice | Later |
|---|---|
| Catch-up and conflicts, the CI wait, red CI back to the worker, the merge yes, the merge and the merge queue, closing the issue, cleanup | The unread head and the review pass (note 2, SK ADRs 0006 to 0008). Rulings answered by the PM, `pm_decides` and merging unasked, dispatch, label writes, threads (note 3) |
| GitHub and GitHub Actions, behind the CI port and the merge queue trait | Other forges and CI systems, each through the same port |

### The order

Lifted as SK settled it (`runner/gate/mod.rs`, `Runner::check_ci`, SK ADR 0008). Each pass reads the pull request once and stops at the first step that has something to do:

```text
pull request read
  merged by hand                 counts as the maintainer's yes: Done
  closed                         ruling: closed
  new head                       the wait starts again
  base branch lagging            wait, and say so after 15 minutes (LAG_LIMIT)
  foreign change                 labels, ready or draft, or the head moved by someone else: ruling
  agents' own files              a harness's file in the diff: ruling
  catch-up                       behind the base branch: rebase and push
    conflict                     a fix turn for the worker, up to 3 (CONFLICT_TURNS), then a ruling
  CI not waited on               the project says its CI does not hold a merge: skip to the merge yes
  checks settling                120 s after a push (CHECKS_SETTLE)
  CI on the head
    none                         no pull request workflow on the base: skipped. Otherwise a ruling after the wait
    pending                      wait. A run stuck queued is rerun once, then a ruling
    failed                       the failure path below
    passed or skipped            the merge yes
```

The unread head and threads sit between CI passing and the merge yes. They land with note 2 and note 3, and until then a green head on the latest base goes straight to the yes.

### Red CI

As SK's `ci_failed` (`runner/gate/mod.rs`) and `back_to_worker` (`runner/rework/`):

1. Kelpie reads the failed runs' logs, at most three.
2. A head that was red before with nothing pushed since is a ruling: still red.
3. Logs that name no file the branch changed rerun the run once per head, since the failure is likely not the worker's.
4. Past the project's fix attempts, a ruling: still red.
5. Otherwise a fix turn: the worker's same session gets the failing check's name and its log, and starts from `origin`'s head, never a rewrite. The turn takes a slot like any other.

A pull request the merge queue removes goes back the same way, uncounted, with "merge queue" as the check's name. A second removal at the same head is a ruling: merge refused.

### The merge

As SK's `merge()` (`runner/merge/mod.rs`) and `queue.rs`:

1. A yes holds only for the head it was given on. A head that moved, CI no longer green, or a base that moved withdraws the yes, and the gate runs again.
2. A draft is marked ready first, then the merge waits out `CHECKS_SETTLE`, since marking ready can start a fresh run.
3. Kelpie merges as the maintainer's `gh` login, with a merge commit pinned to the head (`--match-head-commit`) and an empty body, so the merge commit's subject is the pull request's title.
4. Where the base branch has a merge queue, kelpie enqueues and records it first, then watches for the merge or a removal.
5. A merge GitHub refuses catches up once more. A second refusal is a ruling: merge refused.

### Who says yes

Red CI always blocks a merge (SK ADR 0020). In this slice the yes is always the maintainer's, as SK's default `Merging: Ask` has it. Merging unasked, and the PM answering the merge for a head it vouches for, come with note 3's rulings.

A ticket whose head is ready shows Merge on its card, and dragging it from Review to Done is the same yes (`docs/design/tickets.md`, the move table). A merge on GitHub by hand counts as the maintainer's yes.

### After the merge

As SK's `merged`, `cleanup` and `close_merged.rs`:

1. The ticket is Done. Nothing else is said: an OS notification fires only when work waits on the maintainer (`docs/design/ux.md`, Attention).
2. A pull request that names its issue is left to GitHub to close, for up to 600 s (`FORGE_CLOSE_WAIT`, never measured, the maintainer's call). Otherwise kelpie comments once and closes it through `Tickets`, with three tries.
3. The worker's session stops, and kelpie removes the worktree, deletes the remote branch and checks with `ls-remote` that it is gone (`worktree/remove`, already lifted).
4. An item that ends unmerged is handed back to the maintainer.

### Rulings in this slice

The gate's rulings use what Phase 2 already has (`GLOSSARY.md`, Ruling): the reason becomes the ticket's needs-you line, the worker holds no slot, and kelpie tells it no more. In this slice the maintainer answers with a Note, which sends the worker back with that text, or by acting on GitHub, which the next pass reads. Ruling cards, yes and no, and the PM's answers come with note 3.

| Ruling | Raised when |
|---|---|
| Closed | The pull request was closed |
| Foreign change | Its labels, draft state or head changed by someone else |
| Agents' files | The diff holds a harness's own file |
| Conflict | Catch-up still conflicts after 3 fix turns |
| No checks | No check on the head after the wait, with a workflow on the base |
| Check queued | A run still queued after one rerun |
| Still red | Red with nothing pushed, or past the fix attempts |
| Merge refused | Refused twice, or removed from the queue twice at one head |

### The Review column

A Review ticket shows one status line naming where the gate is: catching up, waiting for CI, fixing red CI, ready to merge, in the merge queue, or the ruling's reason. A red run's line links to the failing run on GitHub. While a fix turn runs, the ticket moves to Building, since a worker's session is live, and comes back to Review when the turn ends.

### Settings

SK's `[ci]` keys come over as project settings, facts and limits only (SK ADR 0020), with SK's defaults (`core/settings/ci.rs`):

| Setting | Default | What it does |
|---|---|---|
| CI holds the merge | On | Off reads no checks and asks for the merge once the branch has the latest base |
| Fix attempts | No cap | Fix turns on red runs for one ticket before a ruling. 0 rules on the first red run |
| CI wait | 60 minutes | How long a check may sit queued before a rerun, and a head with no check waits before a ruling |

### Ports

1. The CI port: `head_status`, `rerun` and `logs` (`docs/lift-plan.md`, The seams). SK's rollup reading, its rerun of a stuck run and `JOB_TIMEOUT` are GitHub Actions' and sit behind it (`docs/facts/ci-and-the-merge.md` in SK).
2. The merge queue trait, with `queue.rs` as GitHub's.
3. The writes come with their callers: `merge`, `mark_ready`, `disable_auto_merge`, `comment` and `close_issue` (`gh/mod.rs`, `gh/rules.rs`, `gh/queue.rs`, `gh/issues.rs`). `open_pull_requests` grows the checks, labels and head it left for this slice.
4. A fix turn is a prompt over ACP to the worker's session, so it reaches every harness kelpie has.

### Testing

`runner/several.rs` is ported first, as `docs/lift-plan.md` says: several tickets at once through stand-ins for the forge and the harness, asserting only what the outside world sees. This slice brings its gate and merge cases, and notes 2 and 3 add theirs.

## Synergy

| Reuses or connects to | How |
|---|---|
| Workers (`docs/design/workers.md`) | A fix turn is a turn of the same session in the same worktree, with a slot. Cleanup removes the worktree the worker started with |
| Tickets (`docs/design/tickets.md`) | The Review column, its status line, the Review to Done move, and the needs-you marker. `open_pull_requests` and the `gh` adapter grow, and nothing is read twice |
| The process tree (ADR 0001) | Every `gh` and `git` call is a child in the tree |
| Attention (`docs/design/ux.md`) | A ruling and a ticket ready to merge count toward the project's badge |
| Leases | A fix turn's `cargo test` waits its turn like any other |
| Review (note 2) and rulings (note 3) | Slot into the order at the unread head and the merge yes. Nothing in this slice is replaced by them |

It would duplicate GitHub's checks page and merge button. The status line links to the failing run, and a hand merge on GitHub is read as the yes rather than fought.
