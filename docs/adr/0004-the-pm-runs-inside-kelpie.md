# The PM runs inside kelpie

Decided on 2026-11-08 in `docs/process-and-sk.md`.

The PM is the maintainer's one conversation per project. It runs in kelpie's process tree, and the app's chat box is its front end, so the maintainer pays for one Opus session rather than a PM in SK plus a chat in the app. That made the PM chat the first tracer bullet: one session streamed into the chat box, stopped by a quit and resumed on the next launch. It proves the tree rule, the stream and the chat UI at once, and needs nothing from SK's runner.

## Considered options

- **SK's board as the first slice.** It needs SK's runner and keeps two PMs alive. A read-only board view can come later.
