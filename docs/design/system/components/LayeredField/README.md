# LayeredField

A settings row that shows where its value comes from (kelpie's built-in default, the global settings, or this project) and resets to the layer below.

**Used by** the settings page and the agent editor; an agent file's `extends` is one more layer in the same component.

**The consumer provides**
- `label`, `help` (one line), and the control (shadcn `Select`, `Input`, `Switch`).
- `layers`: the ordered list of layers, e.g. `["kelpie", "global", "project"]`, or for an agent `["kelpie:pm", "global", "this agent"]`.
- `source`: the layer the current value comes from.
- `inherited`: the value the field would take after a reset, to show in the reset link.

**Anatomy**
- Layer meter: one 14px step per layer, filled up to the source layer, so depth reads at a glance.
- Source chip: "from kelpie", "from global", or "set in this project". Outline when inherited, `secondary` fill when set here.
- Reset: a link button, "Reset to global (main)", shown only when the value is set on the top layer.

**Rules**
- The chip says "from", never "inherited" or "override".
- Changing the control moves the source to the top layer immediately; reset moves it back.
- No setting is file-only: every field renders from the same schema the JSON file uses.
