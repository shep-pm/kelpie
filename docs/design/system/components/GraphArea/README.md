# GraphArea

An area on the graph (a named group of code: folder rules plus hand-picked symbols), and the file and symbol nodes inside it.

**The consumer provides** `name`, `rule` (the path pattern, shown in `code-sm`), a size (file count), and state flags: `lit`, `dim`, `selected`, `worker` (a running branch touches it), `needsYou`, and the ticket badges for its foot row. `unsorted` marks the one area that holds code matching no rule.

**States, and the one token each uses**
| State | Look |
|---|---|
| Default | `graph-area` fill, `graph-area-border` edge, `radius-lg` |
| Lit | 2px `graph-lit` outline at 2px offset |
| Not lit | `opacity-dim` while a ticket is open |
| Selected | 2px `ring` halo |
| Worker branch | `status-running-soft` fill, `status-running` edge and a 4px bottom band |
| Needs you | `status-needs` diamond pip on the top edge, right |
| Unsorted | no fill, 1.5px dashed `border-strong`, a quiet count |

**Rules**
- States combine; when they do, lit beats dim and the worker band stays visible under dim.
- An area's label carries its identity, so its edge stays quiet; only lit, selected and worker states strengthen it.
- Never colour an area by anything but these states. Areas are not categories.
