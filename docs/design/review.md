# Review

Phase 3's second slice (`docs/process-and-sk.md`, Order of work): once CI is green on a worker's head, reviewers read the pull request, each once and in order, and the worker gets one fix turn after each. It slots into the gate at the unread head (`docs/design/gate-and-merge.md`, The order). Rulings with the PM's review plan and dispatch (note 3) build on it. It places itself in `docs/design/tickets.md` (the Review column) and `docs/design/workers.md` (the fix turn, slots), and lifts from SK's `runner/review/`, `runner/gate/unread.rs`, `core/work_item/review.rs`, `core/ports/findings.rs` and `core/ports/reviewer.rs`, row by row from `docs/lift-plan.md` (Phase 3, The gate, review and merge). Terms are in `GLOSSARY.md`.

**Status:** confirmed by the maintainer on 2026-11-11: a reviewer's read is a tab beside the worker's, its findings are the worker's next message, and nothing goes to GitHub but disputes no reviewer read; bot reviewers are deferred to their own slice (Deferred: bot reviewers).

## Use case

After the gate slice, a green head goes straight to the maintainer's merge yes, so they read every diff themselves or merge it unread.

1. CI goes green on a worker's head. A reviewer agent reads the diff against the base, with the issue's acceptance criteria, and reports findings.
2. The worker gets the findings as one fix turn, fixes or disputes each, and pushes.
3. CI goes green on the fix. The next reviewer reads, if the pass owes another read.
4. When the pass has its reads, the gate goes on to the merge yes, naming anything no reviewer read.

Why it matters: in SK's experiments, one defect-hunting read found 27% of 52 known bugs at 82% precision, and a second look added 11 points at 95% (SK ADR 0006). That is a model call in place of the maintainer's own read.

## Design

### Scope

| In this slice | Later |
|---|---|
| Session reviewers over ACP on any harness, the pass, second look, findings, the fix turn, disputes, a reviewer's failing test, the unread head | The PM's review plan, one more read on a finding above a nit, the PM ruling on threads, `no <note>` and `rework <note>` on the merge (note 3) |
| | Bot reviewers such as CodeRabbit, deferred to their own slice (Deferred: bot reviewers). SK's command and endpoint reviewers, since a local model reviews through a harness that runs it |

### The pass

Lifted as SK settled it (SK ADRs 0006, 0007, 0008):

```text
CI green on the head (the gate)
  unread head                    no reviewer of a pass read it: a pass starts
  pass
    round
      worktree is the pushed head, or the worker gets one turn to push
      CI green on this head      waits, as the gate does; red is a CI fix turn first
      choose the next reviewer   one not run this pass, whose paths touch the diff
      read                       a fresh session, read-only on the worktree
      second look                where the reviewer has it: a fresh session, shown the first list
      findings                   both lists as one, worst first
      fix turn                   the worker fixes, defers or disputes each, and pushes
    the pass has its reads, or no reviewer is left
  back to the gate               the merge yes, naming anything unread
```

1. Each reviewer reads once a pass, in the project's order, and each read is followed by at most one fix turn. There is no loop and no judge (SK ADR 0006).
2. A pass owes 2 reads (`RULE_READS`, `review/lineup/weight.rs:26`) until note 3 lets the PM set the count at the pick. A reviewer whose call fails 3 times in a row (`ROUND_FAILURES`) is passed over for the pass.
3. A read holds a slot like any turn. A restart clears a read in flight and the round starts it again with the same reviewer.
4. A local round never marks the draft ready. The merge does (`gate-and-merge.md`, The merge).

### Reviewers

1. A reviewer is an agent file with `role: reviewer`, as SK's agent format has it. Kelpie ships `kelpie:reviewer`, SK's `defect-hunter` prompt with `second_look` on. Like `kelpie:worker`, it names no harness or model, and the project's settings choose them (ADR 0006).
2. A project lists its reviewers in the order they read, `kelpie:reviewer` by default. A reviewer's `paths` globs limit it to diffs that touch them.
3. A reviewer runs as an ACP session in its own sandbox: the worktree read-only, no commands, and one writable folder for tests when the project has a test command. Its prompt is SK's `reviewer_prompt`: the base, the diff, the issue's acceptance criteria (at most 4000 bytes), the pull request body's last 4000 bytes, and the disputes earlier readers were shown.
4. A project with no reviewer that can run, or whose reviewers are all passed over, goes to the merge yes with the head marked unread and the reason.

### Findings

As SK's `Finding` (`core/ports/findings.rs`): a severity of low (a nit), medium or high, a file and line, what and why, and the path of a failing test where the reviewer wrote one. Two findings are the same when their file, line and what match.

1. Every finding goes to the worker at the reviewer's own severity, nits included (SK ADR 0007).
2. A nit-only fix starts no new read.
3. The worker writes which findings it defers and which it disputes, with a reason, beside the findings file. A finding disputed once and raised again by a later reader is a ruling: two readers disagree, and the maintainer settles it.
4. A fix that pushes nothing for findings above a nit, and is not every one deferred or disputed, is a ruling: fix not pushed.

### A reviewer's failing test

With a project's Test command set, such as `cargo test --test {name}`, a reviewer may leave one failing test per finding. Kelpie runs it at the head the reviewer read, and again after the fix (SK ADR 0007, `review/checks/`). A test still failing sends the worker back once, then is a ruling: test still fails. Without a Test command no reviewer is asked for one.

### The unread head

As SK's `gate/unread.rs` for a project whose PM does not yet rule on threads: a head pushed after the pass's last read, or a pass that ended short, reaches the merge yes with a warning naming it, and the maintainer decides. Note 3 sends such a head back through a pass instead, once the PM rules.

### What the maintainer sees

A reviewer's read opens as a tab in the PM panel beside the worker's, in the same chat view, and its findings are the worker's next message, so the worker's tab shows what it was asked to fix. Nothing is posted to GitHub except disputes no reviewer read, as one comment at the gate, as SK does.

The Review card's status line (`gate-and-merge.md`, The Review column) gains `reviewing, read 1 of 2` and `fixing findings`. A fix turn moves the ticket to Building, as a CI fix turn does.

### Deferred: bot reviewers

A pull request bot, such as CodeRabbit, is not a reviewer in kelpie's pass yet. It comes as a slice of its own, with its own note, when a project wants one.

1. What it brings: SK's `runner/review_bot/` and `ReviewStage`'s `Summon`, `Summoned` and `Settling`. A bot sits in the same lineup as an agent reviewer. Kelpie marks the draft ready for a bot that skips drafts, summons it once its window and lease allow, waits for its review and settles its threads, and those threads become the round's findings. A bot passed over gets a late round of its own. SK's `review_bot/` constants (`HEARD_WAIT`, `ANSWER_WAIT`, `DONE_SETTLE`, `FAR`, `THREAD_SETTLE`, `SETTLE_LEAST`, `SETTLE_MOST`, `REVIEW_WAIT`, `LATE_READ`, `BOT_LEFT_OUT`) come with it.
2. Why it waits: no bot showed a measured gain in SK's experiments (SK ADR 0006), and its machinery is the heaviest part of SK's review code.
3. Until then: a bot the repo runs on its own still reviews the pull request on GitHub. Kelpie neither summons it nor waits for it, and its threads are not findings. Note 3 gives open threads to the PM at the merge, a bot's included.

### Settings

| Setting | Default | What it does |
|---|---|---|
| Reviewers | `kelpie:reviewer` | The reviewer agents a project uses, in the order they read |
| Test command | None | Runs one failing test a reviewer wrote, with `{test}` or `{name}` (SK's `ci.test_command`) |
| Reviewer harness, model, effort | Empty | As the worker's: kelpie names none (ADR 0006) |

SK's removed `reviews.*` keys stay removed (SK ADR 0020): how many reads an item gets becomes the PM's call in note 3.

### Testing

SK's `review/tests/` and the review cases of `runner/several.rs` come with the code, on the suite the gate slice ports first.

## Synergy

| Reuses or connects to | How |
|---|---|
| The gate (`docs/design/gate-and-merge.md`) | A pass starts at the unread head and ends back at the gate. CI waits, CI fix turns and the merge yes are the gate's |
| Workers (`docs/design/workers.md`) | A finding's fix is a fix turn of the same session. A read takes a slot |
| The PM chat (`docs/design/pm-chat.md`) | A reviewer is a session in the same chat view and harness interface |
| crabpot (ADR 0005) | A reviewer's sandbox is one more policy, read-only on the worktree |
| Settings and agents (`docs/design/settings.md`) | Reviewers are agent files, and the list, harness, model and test command are settings |
| Leases | A reviewer's failing test runs under `cargo-test` like any other |
| Rulings (note 3) | The PM's read count, its one more read, and threads |

It would duplicate a pull request bot's review, which is one more reason bots wait for their own slice (Deferred: bot reviewers).
