# TurnMarkers

Marks in the chat history that are not messages: an interrupted turn, and dividers.

**Interrupted turn**: the last user message keeps its place; under it, a `status-failed` inline badge "Interrupted when kelpie quit" (or "Interrupted") and an outline **Retry** that sends it again.

**Divider**: a centred `label`-size line between two hairlines, with one icon. Use the same style for every event kelpie records that no harness does:
- "Switched from Claude Code (Opus 5.5) to pi (qwen)"
- "Couldn't resume, started a new session"
- Day breaks: "Yesterday", "Mon 9 Nov"

**The consumer provides** the event text. Keep it to one line, past tense, plain.
