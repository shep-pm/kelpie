# kelpie's design system

The handoff from design to code: the tokens, the rules, and an HTML preview of every component and screen kelpie's UI is built from. Agents implement from this folder. Terms are in `GLOSSARY.md`; the layout and behaviour behind the screens are in `docs/design/ux.md` and `docs/design/pm-chat.md`.

**Status:** design system v1, from the maintainer's design brief, 2026-11-09. The screens are sketches of the design, not specs of any slice: each slice's own design note decides what it builds.

## What is here

| Path | What |
|---|---|
| `brand-book.md` | The rules: content and vocabulary, colour, status, graph, diffs, type, spacing, elevation, motion, layout, iconography, logo and mascot. Read it first. |
| `tokens.json` | Every token with its light and dark value and a usage note. The source of truth. |
| `index.html` | All previews on one page. Open it in a browser; `?theme=dark` or the switch link flips the theme. |
| `components/<Name>/preview.html` | One static HTML rendition per component or screen. |
| `components/<Name>/README.md` | What it is, what the consumer provides, which shadcn component it builds on, and its rules. |
| `components/bundle.css` | The previews' stylesheet. Reference for values and states, not code to ship. |
| `tokens.css`, `fonts/` | What the previews load. The app does not use them. Their licences (SIL OFL 1.1) are in `fonts/` too. |
| `assets/logos/` | The app icon and the mascot, with their usage rules. |

## Already in the app

- `src/styles/globals.css`: the tokens as shadcn's CSS variables on Tailwind v4, light (`:root`) and dark (`.dark`), plus kelpie's own (`status-*`, `graph-*`, `diff-*`, `brand-*`, `rail`), the type scale and radii. `src/main.tsx` imports it and sets `.dark` from the OS.
- `components.json`: shadcn's config. `@/` is `src/`, through `tsconfig.json` `paths`.
- `src/components/ui/`: shadcn's base set, as shadcn ships it (new-york, Radix): button, badge, input, textarea, label, select, switch, checkbox, tabs, tooltip, popover, dropdown-menu, dialog, command, separator, scroll-area, resizable, collapsible, skeleton, kbd. Add more with `pnpm dlx shadcn@4.22.2 add <name>`.
- Fonts come from `@fontsource-variable/atkinson-hyperlegible-next` and `-mono`, bundled, since the app runs offline. Icons are `lucide-react`.

## Implementing from it

1. Restyle shadcn's components to the brand book rather than writing new primitives. They still carry shadcn's own sizes and focus style: controls are 28px tall here (`h-7`, `h-6` small, `h-8` large) and focus is a 2px solid `ring` outline at a 2px offset.
2. Kelpie's own parts (StatusBadge, NeedsYouCount, LayeredField, ToolSteps, UsageFooter, Composer, SandboxRequest, TurnMarkers, ThreadList, GraphArea, Diff) are built from those primitives. Match the preview, and take every value from a token: `bg-status-needs-soft`, `text-diff-add-fg`, `outline-graph-lit`. Never a literal colour.
3. Use the labels in the brand book's vocabulary exactly.
4. Check both themes. The previews use `data-theme="dark"`; the app uses the `.dark` class. The values are the same.

## Changing it

`tokens.json` in this folder is the source of truth. A change edits it first, then mirrors the values into `src/styles/globals.css`, `tokens.css` and any preview that shows them, in one commit. The previews show the design and never lead it: a change to them follows a change to the tokens or the brand book.
