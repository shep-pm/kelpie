# Component sources

Code kelpie's UI can lift instead of writing from scratch. Both repos are the maintainer's, MIT, React 19 and shadcn, so they match kelpie's stack. Looked at on 2026-11-08: chelone `4fcdd4fe1`, ridiculous `c4c1f90`.

## chelone: lift components, skip the framework

chelone is a headless admin engine: resources, data providers, list and record contexts, its own router glue. Kelpie's data comes from its engine over Tauri commands and Channels, not from REST resources, so the engine would sit between kelpie and its data.

Its components are worth lifting. Each depends on chelone's hooks (`useListContext`, `useUpdate`, `useResourceContext`, its router and i18n), and a lift swaps those for kelpie's store and commands, the same way a lift from SK drops the shep coupling.

| Component (`packages/react/src/components/`) | Use in kelpie |
|---|---|
| `kanban/` (`@dnd-kit/react`, shadcn `Button`) | The work board |
| `monaco/` | Diff and file views |
| `charts/` | Usage, cost and run metrics |
| `inspector/` | A details side panel |
| `mdx-editor/`, `block-editor/`, `rich-text-input/` | Writing issues and notes, if chat isn't enough |
| `ui/` | The shadcn base set, already tuned |

`admin/` and `maps*/` stay behind.

## ridiculous: a shadcn registry

22 strictly typed components, installed with `npx shadcn add https://ridiculous.turtlesocks.dev/r/<name>.json` and needing no lift. Most edit CSS values (color, gradient, easing, shadow), which kelpie has little use for. Possible fits:

- `query-builder` and `filter-builder`: filtering the board and the process tree
- `color-picker`: project colors or a theme setting

## When a lift happens

Log it in `docs/lifted.md`, with the chelone path and commit in the source column.
