# Kelpie

A desktop app where people and agents plan, run and follow work on a codebase. Vocabulary accepted on 2026-11-08, from the terms settled in `docs/process-and-sk.md` and ADRs 0001 to 0004.

shep's words (sheep, flock, lamb, dog, the shepherd) mean shep's things only. Kelpie's processes are not sheep. SK's own terms (ruling, lease) come over with the code that uses them, when it is lifted. SK's work item is kelpie's ticket.

## Who

**Maintainer**:
The person who owns kelpie and the codebases it works on, and decides what it does.
_Avoid_: user, owner

**PM**:
The maintainer's one long-lived conversation for a project, through which work is planned and followed. The chat box is its front end.
_Avoid_: project manager agent, PM chat (the first tracer bullet, not the PM)

**Thread**:
A side conversation forked from the PM, tied to a ticket or a topic. Its outcome returns to the PM in one line, and its transcript stays with it.
_Avoid_: sub-chat, branch, fork (the mechanism, not the thing)

**Worker**:
An agent session that does one piece of work and then ends.
_Avoid_: lamb, runner

**SK**:
shep-kelpie, the shep dog that builds kelpie until kelpie can build itself, and is then archived.
_Avoid_: kelpie (for SK), the backend

## Projects

**Project**:
One codebase kelpie works on, with its own PM, graph, board and git history. Projects sit in a rail down the side of the window.
_Avoid_: repo (the GitHub side), workspace

**Pillar**:
One of a project's four parts: the graph, which fills the window, and the PM, Board and Git panels docked beside it. Each project reopens with the layout it was left in.
_Avoid_: tab, view, page, mode

**Graph**:
The picture of a project built from its code, which grows as the code changes without anyone asking.
_Avoid_: map, code map, diagram

**Area**:
A named part of a project on the graph, drawn by the maintainer or the PM, with the code under it. An area for a planned feature may hold no code yet.
_Avoid_: module, region, cluster, component

**Ticket**:
An item on a project's board: one piece of work and everything known about it, including its conversation with the PM.
_Avoid_: work item, card, issue (the GitHub side)

**Draft ticket**:
A ticket not yet ready for a worker, while the maintainer and the PM shape it: not yet filed, or filed without the ready label.
_Avoid_: idea, proposal

**Board**:
A project's tickets in columns, Draft, Ready, Building, Review and Done. A ticket's column is a fact about it, read from its source and the engine, not a place it is put.
_Avoid_: kanban, queue

**Ticket source**:
Where a project's tickets live and are true: GitHub issues, or local files. A project has one.
_Avoid_: tracker, backend

**Pick up**:
An issue becoming a ticket on the board, by who opened it or by the ready label.
_Avoid_: import, sync

**Issue**:
GitHub's record of a ticket, once filed. The ticket holds more than its issue does.
_Avoid_: ticket (for the GitHub side)

**Link**:
A ticket's tie to the code: an area, a symbol, a file or lines within a symbol, kept pointing at the right place as the code moves.
_Avoid_: anchor, reference, tag

## Processes

**Run**:
One launch of kelpie, from start until it quits, crashes or is killed. Every process it starts is marked with the run's id.

**Engine**:
The part of kelpie that starts, watches and stops every process, and holds the sessions.
_Avoid_: daemon, supervisor, backend

**Process tree**:
Every process a run has started, down to grandchildren. All of it stops when the run ends.
_Avoid_: flock

**Reaper**:
A small process that stops the process tree when kelpie dies without quitting.

**Sweep**:
The search for leftover processes marked with a run's id, which stops any it finds.

**Stop ladder**:
How a process tree is stopped: ask, wait a grace period, then kill.

## Sessions

**Session**:
A conversation with an agent. It outlives the processes that serve it: a quit stops the process, and the next message resumes the session.
_Avoid_: process, chat

**Sandbox**:
What confines an agent session's reads, writes and network. Every agent runs in one, the PM included.
_Avoid_: fence, jail, container

**Sandbox setup**:
The one-time change a machine needs before any sandbox can start on it: an AppArmor profile on Ubuntu 24.04 and later, crabpot's install on Windows.
_Avoid_: sandbox install, permissions

**Sandbox request**:
An agent asking the maintainer to let its sandbox reach something it refused. Allowed once, for the project, or denied.
_Avoid_: permission prompt (the harness's own, which is off)

**Kelpie tool**:
A tool kelpie itself serves to an agent session, the same on every harness. The PM changes anything outside its own folder through one.
_Avoid_: MCP tool (the mechanism), command

**Turn**:
One round of an agent's work, from a message to its reply.

**Seed**:
What a new session starts from when the old one cannot carry on: the standing-decisions file, then the newest exchanges of kelpie's own log, up to 10 or 8,000 characters. No model writes it.
_Avoid_: summary, handoff, brief (a thread's starting text)

**Divider**:
A line in the history where a new session began, such as a switch of harness or a resume the harness refused. The history above it stays.
_Avoid_: separator, marker

**Harness**:
The agent program a session runs on, such as Claude Code, Codex, pi or goose. The model is chosen separately.
_Avoid_: CLI, backend, provider

**Bridge**:
A program from the ACP registry that lets kelpie reach a harness that does not speak ACP itself, such as Claude Code. Kelpie installs and updates the bridge, never the harness behind it.
_Avoid_: adapter (kelpie's side), shim, wrapper

**Adapter**:
What lets kelpie use one outside tool through an interface shared by every tool of its kind: a harness, a forge such as GitHub, an indexer such as CMM. Kelpie is a hub of adapters and favours none.
_Avoid_: plugin, integration, driver

**Agent**:
A named way to run a session: its harness, model, effort and instructions, kept as a file that may extend another, such as kelpie's built-in `kelpie:pm`.
_Avoid_: profile, persona, bot

**Layer**:
One level a setting's value can come from: kelpie's built-in default, then global, then project. An agent's `extends` chain adds layers of its own. The nearest layer that sets a value wins.
_Avoid_: scope, level, override

**Paused**:
The state of a worker whose process stopped mid-turn. It resumes only when the maintainer says so, never on its own.
_Avoid_: stopped, suspended

**Terminal tab**:
A shell the maintainer types into, inside kelpie. It is gone after a quit and reopens in the same folder.
_Avoid_: pty session

**Drain**:
What a restart for an update does first: no new turns start, and the running ones finish.
_Avoid_: handover

## Workers

**Worktree**:
A worker's own copy of the project, on a branch of its own, where it does its one piece of work.
_Avoid_: checkout (the maintainer's own), clone (the mechanism)

**Note**:
A message from the maintainer to a running worker, read at its next step without stopping it. It goes into the ticket's record.
_Avoid_: tell, nudge

**Stop and tell**:
A message that interrupts the worker's turn and becomes its next prompt.
_Avoid_: interrupt, cancel

**Ruling**:
What kelpie makes of something a worker will not fix on its own: the worker waits on the maintainer with what it found as its reason, holds no slot, and is told no more until the file or the maintainer changes.
_Avoid_: error, blocker, escalation

**Fence**:
The check, when a worker's turn ends, that its branch changed only what a worker may change. It sits on top of the sandbox and is not the sandbox.
_Avoid_: guard (the check on each command)

**Slot**:
One of the places a project has for a running worker. A worker that is Paused or waiting on the maintainer gives its slot back.
_Avoid_: seat, lane

**Lease**:
A turn at something only a few may use at once, such as a test run, held while the command runs and handed back when it ends.
_Avoid_: lock, mutex

**Gate**:
The checks a worker's pull request passes between its draft and the merge: on the latest base branch, CI green on its head, then the maintainer's yes.
_Avoid_: pipeline, checks (CI's own)

**Catch-up**:
Bringing a pull request's branch up to the latest base branch, so the head CI passed on is the head that merges.
_Avoid_: sync, update branch

**Fix turn**:
A worker's turn spent on what the gate sent back: a red CI run, a conflict from catch-up, or a reviewer's findings.
_Avoid_: retry, rework turn

**Reviewer**:
An agent that reads a worker's pull request once CI is green on it and reports findings, without changing the code.
_Avoid_: judge, critic, review bot (a pull request bot such as CodeRabbit, which is not one yet)

**Finding**:
One problem a reviewer reports at a place in the diff, with how bad it is: a nit, medium or high.
_Avoid_: comment, issue (a ticket's)

**Dispute**:
A worker's reasoned refusal to fix a finding. A finding disputed once and raised again by another reviewer goes to the maintainer.
_Avoid_: rejection, pushback

## Building kelpie

**Lift**:
Bringing a chunk of SK or chelone into kelpie as kelpie's own code, with its tests, a note of what it handles and a row in `docs/lifted.md`.
_Avoid_: port, vendor (an unchanged copy, as with a skill)

**Slice**:
One end-to-end piece of kelpie that works on its own, built in phase order. A lift brings in only what the next slice needs.
_Avoid_: milestone, epic
