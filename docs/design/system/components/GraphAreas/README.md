# GraphAreas

The graph at its first zoom level: the project's areas, with the open ticket lighting what it links to.

- 5 to 9 boxes per level. Areas show their name (`title`), their rule and size (`code-sm`), and a foot row of ticket badges.
- **Unsorted** is an area like the others: dashed `border-strong` edge, no fill, a quiet count, and the PM's proposal waiting inside.
- **The open ticket** (chip bottom-left) lights its linked areas with a 2px `graph-lit` outline; edges between two lit areas turn `graph-lit`; everything else drops to `opacity-dim`.
- **A worker's branch** tints the areas it touches with `status-running-soft` and a 4px `status-running` band along the bottom.
- **Needs you** on an area is a `status-needs` diamond pip on its top edge.
- HUD: breadcrumb and base branch selector top-left, legend top-right, zoom bottom-right. All HUD boxes are `popover` with a `border`, never a shadow, because they sit on the canvas rather than float above content.
- 2D, node-and-edge, built with React Flow; the canvas is `background` with a `graph-grid` dot grid at 16px.
