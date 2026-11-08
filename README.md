# kelpie

Kelpie's visual side: an app where people and agents look at, plan and follow work on a codebase as pictures instead of chat.

Private for now. Kelpie itself lives in shep-pm/shep-kelpie.

## What's here

- `docs/goals.md`: start here. The two goals, starting a project A to Z, and what is parked.
- `docs/journey.md`: every step of starting this project, who did it, and whether SK could have.
- `docs/roles.md`: the architect role proposed for SK.
- `docs/gui-idea.md`: the idea, the pushback and prior art, as short numbered points.
- `docs/backend.md`: why kelpie exists, the backend, the repo split and the stack, decided 2026-10-23.
- `docs/gui-framework.md`: the shell and frontend re-checked on 2026-11-08: Tauri 2 + React stays, Solid judged and ranked fourth.
- `docs/process-and-sk.md`: decided 2026-11-08: kelpie owns its process tree, ships neither shep nor SK, takes shep-tree, and lifts SK's code as needed.
- `docs/session-notes-2026-11.md`: everything said about the app during the shep-kelpie sessions, Oct 26 to Nov 8.
- `map/build.py`: builds shep-kelpie's module map from its `src/` (which module uses which, and the loop they form).
- `map/page.html`: the page template `build.py` fills in.
- `map/module-map-c18cdb1b.html`: the first map, built from shep-kelpie main at c18cdb1b.

## Build the map

```sh
mkdir -p /tmp/kelpie-src && git -C ../shep-kelpie archive origin/main src | tar -x -C /tmp/kelpie-src
python3 -I map/build.py /tmp/kelpie-src/src out
open out/module-map.html
```

The page's headline numbers and prose are still written by hand for c18cdb1b. Only the grid and the panel follow the data.
