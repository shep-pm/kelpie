# ThreadList

The PM's threads, listed at the top of the PM panel like Discord threads under a channel.

**The consumer provides** threads: `name` (a short noun phrase), `tie` (a ticket number like `#42`, or `topic`), `status`, and for finished ones the one-line outcome.

**Rules**
- One row per thread, 28px, name truncates first. The tie is mono `code-sm`; the status is a glyph only (with an aria-label), since the column is narrow.
- Threads that need you sort to the top; done threads drop below a "Done" fold after a day.
- One level deep: a thread cannot fork a thread.
- When a thread ends, its outcome returns to the PM as one line in the PM's history (the outcome row shown here), not as its transcript.
