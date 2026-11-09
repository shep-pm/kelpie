# kelpie brand book

kelpie is a desktop app where people and coding agents plan, run and follow work on a codebase. Its reader is a developer who learns by seeing: long text makes them agree to everything. So the project is a picture they can point at, and every piece of text is short, folded and scannable.

**Priorities, in order:** calm and scannable · the graph is the centre, not the chat · "what needs me" visible at a glance, never noisy · dense enough for a developer tool, never cramped.

## Content

**Tone.** A serious, calm tool with a wink. Labels, statuses, the graph, the chat and every error stay plain. Charm lives only where it costs no attention: the app icon, empty states, loading and onboarding, where a kelpie (the sheepdog) may appear. No sheep puns in labels.

**Writing rules**
- Sentence case everywhere, including buttons and headings. Never all caps.
- Buttons are verbs that say what happens: "Allow once", "Stop and tell", "Retry". Never "OK" or "Submit".
- Name the actor: "PM wants to reach docs.rs", "Message the worker on #42".
- One line where one line will do. Past tense for things that happened: "Couldn't resume, started a new session".
- Lists, tables and before/after over paragraphs. Fold anything longer than a screenful.
- No emoji in the UI. Status is a glyph and a word, not an emoji.
- Numbers abbreviate with one decimal (`18.4k`) and use tabular figures.

**Vocabulary.** Use these labels exactly.

| Term | Meaning | Avoid |
|---|---|---|
| Project | One codebase kelpie works on | workspace, repo (in UI) |
| Graph | The picture of the project's code | map, diagram |
| Area | A named group of code on the graph (folder rules plus hand-picked symbols). Code matching no area sits in **Unsorted** | module, region, cluster |
| PM | The project's one long-lived agent conversation | assistant, bot |
| Thread | A side conversation forked from the PM, tied to a ticket or topic. Its outcome returns to the PM in one line | sub-chat, branch |
| Worker | An agent session building one ticket | runner, bot |
| Ticket | A unit of planned work on the board | card, work item |
| Draft ticket | A ticket the PM is still researching | |
| Link | What ties a ticket to an area, symbol, file or lines | anchor, tag |
| Sandbox | What confines an agent's reads, writes and network | jail |
| Harness | The agent program a session runs on (Claude Code, Codex, pi, goose) | provider |

**Real copy to match:** "Read 4 files" · "Searched for `ProcessTree`" · "PM wants to reach docs.rs: Allow once · Allow for this project · Deny" · "Switched from Claude Code (Opus 5.5) to goose (qwen)" · "Couldn't resume, started a new session" · "Interrupted when kelpie quit" · "3 need you".

## Colour

Neutral first. The window is warm stone greys and ink; colour is spent on status and nowhere else.

- Ground: `background` for the graph canvas, `sidebar` for docked panels and the drawer, `rail` for the project rail and status bar. Text is `foreground`; secondary text `muted-foreground`.
- Cards and nodes on the canvas are `card`; anything floating (menus, tooltips, the selection toolbar) is `popover`.
- The strong action is ink: `primary` on `primary-foreground`. There is no brand hue on buttons.
- Hairlines are `border`. Anything that must be seen to be used (inputs, checkboxes, outline buttons, the Unsorted edge) is `border-strong`, which holds 3:1.
- Focus is a 2px solid `ring` outline at a 2px offset, on everything. `ring` holds 3:1 on every surface in both themes.
- `destructive` is for destructive actions only. A failed run is `status-failed`.
- `brand-teal`, `brand-coat`, `brand-cream` and `brand-outline` are the mascot's colours. They belong to the app icon, onboarding, empty states and loading, never to chrome or status. The UI's one warm hue, `status-needs`, is the same kelpie coat tuned for contrast.
- Every token name used by shadcn (`background`, `card`, `primary`, `muted`, `accent`, `border`, `input`, `ring`, `sidebar-*`, `chart-1`…`chart-5`) is the shadcn variable of the same name, so shadcn components pick kelpie up unchanged. The Tailwind v4 theme built from these tokens is `src/styles/globals.css`.

## Status

Six states for tickets, workers, threads and processes. Each is a word, a glyph and a colour; never colour alone.

| Status | Glyph | Mark / soft / text tokens | Means |
|---|---|---|---|
| Idle | hollow ring | `status-idle` · `-soft` · `-fg` | Nothing happening |
| Running | spinning arc | `status-running` · … | An agent or process is working |
| Needs you | filled diamond | `status-needs` · … · `status-needs-on` | Blocked on the maintainer |
| Blocked | ring with a bar | `status-blocked` · … | Waiting on something else (CI, a dependency) |
| Done | check | `status-done` · … | Finished |
| Failed | cross | `status-failed` · … | Crashed or failed a check |

- `status-*` is the mark: glyphs, dots, graph overlay strokes (3:1 on `background`, `card`, `sidebar`).
- `status-*-soft` is a badge fill or an area tint; `status-*-fg` is text on that fill and on the plain grounds (4.5:1, both themes).
- **Needs you is the kelpie's tan,** the only warm hue in the app and the only status ever drawn solid (the count badge, `status-needs` with `status-needs-on`). Never use it for decoration, highlights or links, or it stops meaning "you".
- Done is a blue-leaning teal, not a green, so done and failed differ by more than red against green.
- On the graph, the same tokens become overlays: a worker's branch tints the areas it touches with `status-running-soft` and a 4px `status-running` band; a waiting item puts a `status-needs` pip on its area; blocked can hatch.

## Graph

- Canvas `background` with a 1px `graph-grid` dot every 16px. Areas `graph-area` with a `graph-area-border` edge at `radius-lg`; files and symbols `graph-node`; edges 1.25px `graph-edge` with small arrowheads.
- The open ticket lights its links with a 2px `graph-lit` outline (offset 2px) and dims everything else to `opacity-dim`. Lit is ink, not a hue, so it never collides with a status.
- Selection is a `ring` halo. Selecting nodes raises the one floating toolbar: the count, **Comment**, **Ask the PM**.
- 5 to 9 boxes per zoom level: areas, then files, then symbols. Unsorted is an area with a dashed `border-strong` edge, no fill and a count.
- HUD boxes (breadcrumb and branch selector, legend, zoom) are `popover` with a `border` and no shadow.

## Diffs

- Added lines `diff-add-bg`, removed lines `diff-del-bg`; the changed words inside them sit on `diff-add-word` and `diff-del-word`. Code text stays `foreground`.
- Signs and line numbers on changed lines take `diff-add-fg` and `diff-del-fg`; a 2px gutter bar and the header's `+N`/`−N` blocks take `diff-add-bar` and `diff-del-bar` (aliases of `status-done` and `status-failed`).
- `@@` hunk headers are `diff-hunk-bg` with `diff-hunk-fg`.
- Added and removed differ by their `+`/`−` sign and gutter, never by colour alone. Diffs are mono `code` at 12px; unchanged runs fold.

## Type

Atkinson Hyperlegible Next for the interface and Atkinson Hyperlegible Mono for paths, symbols, branches, diffs and numbers in columns. Both are designed for legibility, which is the point of this app; both are variable (200–800).

- `ui` (13/20) is the default for controls, rows and menus; shadcn's `text-sm` lands on it.
- `prose` (14/22) for chat replies and anything read as sentences.
- `label` (12/16, 500) for section and field labels; `caption` (11/16, 500) for badges and counts.
- `title` (15/22, 600) for ticket titles and area names; `heading` (18/24) for page titles; `display` (28/32) only in empty states and onboarding.
- `code` (12.5/20) inline with text; `code-sm` (11/16) for the usage footer, file nodes and the process tree.
- Weights: 400 body, 500 labels and buttons, 600 titles, 700 only in count badges and the wordmark.

## Space, size and shape

- 4px grid: `space-1` (4) icon to label · `space-2` (8) row padding and control gaps · `space-3` (12) card and panel padding · `space-4` (16) section gaps and the graph grid · `space-6` (24) between areas · `space-8` (32) empty states.
- Controls are 28px (`control`), 24px small (`control-sm`), 32px large (`control-lg`). Rows match controls.
- The rail and the collapsed panel strip are `rail-width` (48). Panels open at `panel-default` (400) and resize down to `panel-min` (320); the drawer opens at `drawer-default` (240).
- Radii: `radius-sm` (4) badges and symbol nodes · `radius-md` (6) buttons, inputs, rows, file nodes · `radius-lg` (8) cards, popovers, areas · `radius-xl` (12) dialogs · `radius-full` counts and dots.

## Elevation

Docked things are separated by `border` hairlines and one step of surface (`background` → `sidebar` → `rail`), never by shadows. Shadows belong only to what floats: `shadow-xs` lifts inputs and outline buttons by a pixel, `shadow-md` for popovers, menus and the graph's selection toolbar, `shadow-lg` for dialogs and the command palette.

## Motion

Quiet and short. 120ms ease-out for hover, fold and dim; 200ms ease-out for panel collapse and drawer open; the running arc turns once a second. Folds expand in place, nothing slides in from off-screen. Under reduced motion everything is instant and the running arc stands still.

## Layout

```
┌──┬──────────────────────────────┬───────────┐
│P │                              │ PM        │
│r │          Graph               │ threads   │
│o │   (areas, overlays, links    ├───────────┤
│j │    lit by the open ticket)   │ Board     │
│  ├──────────────────────────────┴───────────┤
│  │ Drawer: Processes · Terminal             │
└──┴──────────────────────────────────────────┘
```

- The graph takes all remaining space. Panels (PM, Board, Git) dock right: resizable, stacked or side by side, collapsible to an icon strip, maximisable. They never float over the graph.
- The drawer is shared by every panel. The status bar shows the base branch, process count, running workers, the needs-you count, and the PM's harness and model.
- One needs-you count appears wherever it applies: the project in the rail, the panel it lives in, the area on the graph, the status bar.
- Each project reopens with the layout it was left in.

## Iconography

Lucide, shadcn's default set, through `lucide-react`: 16px in panels and toolbars, 14px in buttons and rows, 12px inline with `caption` text, stroke 1.75. Status glyphs are kelpie's own six (`StatusBadge`), not Lucide icons. No emoji.

## Logo and mascot

Two marks, in `assets/logos/`:

- **App icon** (`kelpie-app-icon.png`): the kelpie's face on a `brand-teal` rounded square. The dock, the window, the installer, the About dialog. Use it whole; never crop the face out of the square or recolour the ground.
- **Mascot** (`kelpie-mascot.png`): the kelpie sitting, in a `brand-teal` bandana with a sheep. Onboarding, empty states and loading only, at 96–160px, centred above a `display` or `title` line. Never in chrome, beside a status, or in an error.

Both are full-colour illustrations with `brand-outline` ink; they sit on `background`, `card` or `brand-cream`, never on a status fill. In text the name is plain type, lower case: "kelpie". The sheep stays on the bandana: no sheep in labels.

## Components

Start from the shadcn set and restyle through these tokens; do not invent new primitives. The cards in this system: `StatusBadge`, `NeedsYouCount`, `Button`, `LayeredField`, `ToolSteps`, `UsageFooter`, `Composer`, `SandboxRequest`, `TurnMarkers`, `ThreadList`, `GraphArea`, `Diff`, and the screens `ShellExpanded`, `ShellCollapsed`, `ShellMaximised`, `PMPanel`, `GraphAreas`, `GraphZoom`. Their previews are static HTML renditions styled by `components/bundle.css`; each README says which shadcn component it builds on and what the consumer provides.
