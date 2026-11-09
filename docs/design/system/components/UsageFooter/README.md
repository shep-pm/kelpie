# UsageFooter

A one-line record of what a turn cost, under each PM, thread or worker reply.

**The consumer provides** the turn's usage: tokens in, tokens out, cache read, cache written, and the cache lifetime that applied (`5m`, `1h`).

**Rules**
- `code-sm` in `muted-foreground`, with `·` separators at half strength. No colour, no bars: it is a record, not an alarm.
- Numbers abbreviate to one decimal with `k` or `M` and use tabular figures, so turns line up when scanned.
- Collapsible to a single total; the choice is remembered per project.
- A field the harness does not report is left out, never shown as 0: harnesses differ in what usage they report (ADR 0007).
- After a harness switch the first footer shows `cache read 0`: say nothing more, the divider above already explains it.
