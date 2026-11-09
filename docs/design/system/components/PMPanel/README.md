# PMPanel

The project's PM: one long-lived conversation, with its threads on top.

**Top to bottom**
1. Header: "PM", its NeedsYouCount, the harness and model, collapse and maximise.
2. Threads (`ThreadList`), three rows visible, "+2" for the rest.
3. The conversation. Replies stream as Markdown in `prose`; tool calls fold into `ToolSteps`; a reply longer than a screenful folds behind "Show the rest · N lines". Each reply ends with a `UsageFooter`.
4. Inline cards: `SandboxRequest`, interrupted turns and dividers (`TurnMarkers`).
5. The `Composer`, pinned to the bottom, with Note and Stop and tell while a turn runs.

**Reading rules**
- The PM's own reply style is short: lists, tables and before/after comparisons over paragraphs. The panel backs it up by folding everything else.
- User messages are `secondary` bubbles on the right; the PM's replies are unboxed on the left, so the replies (the thing being read) get the full width.
- The same view serves threads and workers; only the header and placeholder change.
