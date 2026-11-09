# Diff

A unified diff of one file, in a worker's chat (its diff so far), in a ticket, and later in the Git panel.

**Built on** a plain table today; Monaco's diff view (lifted from chelone) takes the same tokens when it arrives.

**The consumer provides** the file path, the add and remove counts, and hunks of lines (`context`, `add`, `del`) with old and new line numbers and, where known, the changed word ranges.

**Tokens**
| Part | Token |
|---|---|
| Added line | `diff-add-bg`, with `diff-add-word` behind the changed words |
| Removed line | `diff-del-bg`, with `diff-del-word` behind the changed words |
| `+`/`−` signs and line numbers on a changed line | `diff-add-fg`, `diff-del-fg` |
| 2px gutter bar, `+N`/`−N` blocks in the header | `diff-add-bar`, `diff-del-bar` (`status-done`, `status-failed`) |
| `@@` hunk header | `diff-hunk-bg`, `diff-hunk-fg` |
| Code text | `foreground` everywhere, in `code` mono at 12px |

**Rules**
- Every changed line keeps its `+` or `−` sign and its gutter bar: added and removed differ by sign and side of the line numbers, not by colour alone.
- Fold unchanged runs longer than 6 lines behind "Show N unchanged lines".
- Files fold to their header row by default when a diff has more than 3 files.
- Syntax highlighting, when added, stays quieter than the diff colours: it never uses the status hues.
