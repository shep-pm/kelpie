# Kelpie owns its whole process tree

Decided on 2026-11-08 in `docs/process-and-sk.md`, which holds the measurements and the shep code this rests on.

Claude Desktop and Claude Code leave processes running after they quit, with nothing showing they are there. Kelpie exists partly to stop that. shep is built the other way round: its daemon detaches and its sheep outlive it, which is right for a supervisor and wrong for an app.

## Decision

- Every process kelpie starts is a descendant of the app, through the engine's tree. Nothing goes into launchd, systemd, a login item or any other init system, and nothing detaches (`setsid`, double fork, `nohup`, `disown`).
- When kelpie quits, crashes or is killed, every process it started stops, grandchildren included. The one known gap is on macOS (Consequences).
- Each child gets its own process group (a job object on Windows) and `KELPIE_RUN=<run id>` in its environment.
- A reaper sidecar holds a pipe from the app. When the pipe closes, it kills the recorded groups and sweeps for `KELPIE_RUN`. The next launch sweeps again before starting anything.
- Linux adds `PR_SET_CHILD_SUBREAPER` and `PR_SET_PDEATHSIG`. Windows uses a job with `KILL_ON_JOB_CLOSE` (shep#1616).
- Sessions outlive processes. A quit stops the PM's process and keeps its session id; the next message resumes it. A worker that was mid-turn comes back paused, never running.
- Updates drain and restart. `shep-handover` is not used.

## Considered options

- **Ship shep's daemon as kelpie's process layer.** Its daemon detaches, it can write init units, and a SIGKILLed daemon leaves its sheep under init. Embedding it as a library still brings its sheep, dogs and `SHEP_HOME`.
- **Keep work running while kelpie is closed.** That is the background process this rule is meant to remove. For overnight work, kelpie stays open in the tray.

## Consequences

- Code spawns a process only through the engine's tree. A bare `std::process::Command::spawn`, Tauri's shell plugin, or a spawn from the webview bypasses the reaper and the sweep.
- On macOS, a grandchild that double-forks and also scrubs its environment can escape before a sweep sees it. Nothing ordinary does both.
- The phone reaches kelpie only while the desktop app runs.
- `shep-tree` is the one shep crate kelpie takes.
