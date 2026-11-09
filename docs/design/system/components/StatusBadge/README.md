# StatusBadge

Shows one of the six states a ticket, worker, thread or process can be in: idle, running, needs you, blocked, done, failed.

**Built on** shadcn `Badge`, with a `status` prop instead of `variant`.

**The consumer provides** `status` (one of `idle | running | needs | blocked | done | failed`) and, optionally, a shorter or more specific word as children ("Review", "CI failed"). The glyph and colour come from the status; never pass a colour.

**Forms**
- Badge (default): `status-*-soft` fill, `status-*-fg` text, glyph in `status-*`. In cards, rows and the PM's replies.
- Inline: glyph and word, no fill. In dense rows and node sublabels.
- Dot: an 8px `status-*` dot. Only beside a name that already explains itself (the rail, a process row); it is never the only signal in a list.

**Rules**
- Every status always carries its glyph: hollow ring (idle), spinning arc (running), filled diamond (needs you), ring with a bar (blocked), check (done), cross (failed). The glyph is what keeps it readable without colour.
- Use the words exactly: Idle, Running, Needs you, Blocked, Done, Failed.
- Needs you is the only status that is ever shown solid (see `NeedsYouCount`).
- Running's arc stops spinning under reduced motion and shows a still quarter arc.
