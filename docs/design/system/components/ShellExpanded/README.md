# ShellExpanded

The window: project rail, the graph in the middle, docked panels on the right, the drawer at the bottom, the status bar.

**Layout rules**
- The graph always takes the remaining space. Panels dock right like VS Code's secondary sidebar: resizable (min `panel-min`, first open `panel-default`), stacked (shown) or side by side, collapsible to an icon strip, maximisable to full width. They never float over the graph.
- The drawer (Processes, Terminal) spans the graph and the panels, opens to `drawer-default`, and closes to its tab bar.
- The rail is `rail-width` wide: a two-letter tile per project, the active one in `primary` with a bar on its left edge, a NeedsYouCount on any project with something waiting.
- The status bar (`control-sm` tall, `rail` fill): base branch, process count, running workers, needs-you count, the PM's harness and model.
- Panels are separated by `sidebar-border` hairlines. Nothing docked casts a shadow.
- Each project reopens with the layout it was left in.

**States**: expanded (this card), `ShellCollapsed`, `ShellMaximised`.
