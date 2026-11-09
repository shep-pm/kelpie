# ToolSteps

The PM's (or a worker's) tool calls, folded into one line each between the parts of a reply.

**The consumer provides** a list of steps: `kind` (read, search, run, fetch, edit), a one-line summary, an optional result (count, pass/fail), and the detail to show when expanded.

**Rules**
- Folded by default. Consecutive calls of one kind merge: "Read 4 files", not four rows.
- Summary in `ui` with the target in `code`: "Searched for `ProcessTree`", "Ran `cargo test -p engine`".
- Steps sit in `muted-foreground`, so the reply's prose stays the loudest thing in the chat. Hover lifts a row to `foreground` on `accent`.
- Expanded detail is mono `code-sm` on `muted`, at most 4 lines, then "N more".
- A step waiting on the maintainer carries the needs-you inline badge, and its request card follows it.
