# Composer

The message box at the bottom of the PM, a thread or a worker.

**The consumer provides** the session's state (`idle` or `running`), the harness and model label, and any context attached from the graph (a selection of areas or nodes).

**Idle**: one send button (icon, `default`). Context from the graph shows as a chip ("3 nodes from the graph") that can be removed.

**While a turn runs**
- A running line above the box: glyph, "Working", elapsed time and step count, in `status-running-fg`.
- **Note** (default, Enter): read at the agent's next tool boundary, or at the end of its turn on a harness that cannot take one mid-turn, and the chat says which. The note shows in the history marked "note".
- **Stop and tell** (destructive outline, Cmd+Enter): interrupts the turn, then sends the message.

**Rules**
- The placeholder names who you are talking to: "Message the PM", "Message the worker on #42".
- Never disable the box while a turn runs; that is what Note is for.
