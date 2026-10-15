# kelpie

Kelpie's visual side: an app where people and agents look at, plan and follow work on a codebase as pictures instead of chat.

Private for now. Kelpie itself lives in shep-pm/shep-kelpie.

## What's here

- `docs/gui-idea.md`: the idea, the pushback and prior art, as short numbered points.
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
