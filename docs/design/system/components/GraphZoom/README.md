# GraphZoom

The two deeper zoom levels: an area's files, then a file's symbols.

- Files are `card` nodes in mono `code-sm` with a file icon, `radius-md`; symbols are smaller `radius-sm` nodes with their kind (`struct`, `fn`, `trait`, `range`) in `muted-foreground`.
- The breadcrumb always says where you are: Areas › Engine › Files.
- **Selection**: a `ring` border on each selected node, and a floating toolbar (`popover`, `shadow-md`, the graph's one floating surface) with the count and two actions: **Comment** (creates a draft ticket linked to the selection, and a thread where the PM researches it) and **Ask the PM** (sends the selection as context to the PM's composer).
- Lit and dim follow the open ticket at every level, down to a line range inside a symbol.
- Editing the graph never edits code: dragging a node to another area regroups it; anything else becomes a draft ticket.
