# Kelpie

A desktop app where people and agents plan, run and follow work on a codebase. Vocabulary accepted on 2026-11-08, from the terms settled in `docs/process-and-sk.md` and ADRs 0001 to 0004.

shep's words (sheep, flock, lamb, dog, the shepherd) mean shep's things only. Kelpie's processes are not sheep. SK's own terms (project, work item, ruling, lease) come over with the code that uses them, when it is lifted.

## Who

**Maintainer**:
The person who owns kelpie and the codebases it works on, and decides what it does.
_Avoid_: user, owner

**PM**:
The maintainer's one long-lived conversation for a project, through which work is planned and followed. The chat box is its front end.
_Avoid_: project manager agent, PM chat (the first tracer bullet, not the PM)

**Worker**:
An agent session that does one piece of work and then ends.
_Avoid_: lamb, runner

**SK**:
shep-kelpie, the shep dog that builds kelpie until kelpie can build itself, and is then archived.
_Avoid_: kelpie (for SK), the backend

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

**Turn**:
One round of an agent's work, from a message to its reply.

**Paused**:
The state of a worker whose process stopped mid-turn. It resumes only when the maintainer says so, never on its own.
_Avoid_: stopped, suspended

**Terminal tab**:
A shell the maintainer types into, inside kelpie. It is gone after a quit and reopens in the same folder.
_Avoid_: pty session

**Drain**:
What a restart for an update does first: no new turns start, and the running ones finish.
_Avoid_: handover

## Building kelpie

**Lift**:
Bringing a chunk of SK or chelone into kelpie as kelpie's own code, with its tests, a note of what it handles and a row in `docs/lifted.md`.
_Avoid_: port, vendor (an unchanged copy, as with a skill)

**Slice**:
One end-to-end piece of kelpie that works on its own, built in phase order. A lift brings in only what the next slice needs.
_Avoid_: milestone, epic
