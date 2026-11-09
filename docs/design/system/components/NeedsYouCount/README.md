# NeedsYouCount

The count of things waiting on the maintainer: sandbox requests, questions from the PM, tickets in Review, failed workers.

**The consumer provides** `count` (a number). At 0 render nothing: an empty badge is noise. Above 99 show `99+`.

**Where it appears** (one count, scoped to where it sits)
- The project rail: the project's total, on the project tile's top-right corner.
- A panel's tab or icon strip button: what is waiting inside that panel.
- The status bar: the window's total, with the needs-you glyph and the words "need you".
- The graph: an area that holds a waiting item gets a `status-needs` pip (see `GraphArea`), not a number.

**Rules**
- Solid `status-needs` with `status-needs-on` text. It is the only solid status in the app, so it is the first thing the eye finds; keep it that way by never using the tan for anything else.
- Counts are not clickable targets by themselves; the tile, tab or item they sit on is.
- An OS notification fires only when work is blocked on the maintainer, never for the count changing.
