# Button

shadcn's `Button`, restyled: ink instead of a brand hue, 28px tall by default, 13px medium labels.

**The consumer provides** a label (a verb, sentence case: "Allow once", "Stop and tell") and optionally a leading Lucide icon at 14px. Icon-only buttons need an `aria-label` and a tooltip.

**Variants**
| Variant | Use |
|---|---|
| `default` | The one strong action in a view: Send, Allow once, Pick a folder. `primary` on `primary-foreground`. |
| `secondary` | A safe alternative beside the primary: Allow for this project. |
| `outline` | Standalone actions on a card or the graph: Retry, Comment. |
| `ghost` | Toolbars, panel headers, Deny. Hover fills `accent`. |
| `destructive` | Interrupting or deleting: Stop and tell, Delete project. |
| `link` | Inline expanders: Show 12 more lines. |

**Sizes**: `sm` 24px (inline in rows, the selection toolbar), `default` 28px, `lg` 32px (dialogs, onboarding), `icon` square at 28 or 24.

**Rules**
- One `default` button per view. Two strong actions side by side means one of them is `secondary`.
- Focus is a 2px solid `ring` outline with a 2px offset, on every variant.
- Labels say what happens, not "OK" or "Submit". No sheep puns.
