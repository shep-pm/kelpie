"""Module map of shep-kelpie from `crate::<module>` references (exact, unlike the code graph's name-matched calls).

Usage: python3 -I map/build.py <src dir> <out dir>
Writes <out dir>/modmap.json and <out dir>/module-map.html (page.html with the data filled in).
The page's headline numbers and prose are still hand-written for main c18cdb1b.
"""
import json
import os
import re
import sys
from collections import defaultdict

src, outdir = sys.argv[1], sys.argv[2]
os.makedirs(outdir, exist_ok=True)
out = os.path.join(outdir, "modmap.json")
mods = sorted({n[:-3] if n.endswith(".rs") else n for n in os.listdir(src)
               if n.endswith(".rs") or os.path.isdir(os.path.join(src, n))} - {"lib", "main"})
ref = re.compile(r"\b(?:crate|shep_kelpie)::(\w+)")
TEST = re.compile(r"(_tests?\.rs$|/tests?/|/tests?\.rs$|^test\.rs$|^test/)")

lines = defaultdict(lambda: [0, 0])          # module -> [code lines, test lines]
files = defaultdict(int)
edges = defaultdict(lambda: [0, set()])      # (a, b) -> [refs, files]
for root, _, names in os.walk(src):
    for n in names:
        if not n.endswith(".rs"):
            continue
        path = os.path.join(root, n)
        rel = os.path.relpath(path, src)
        top = rel.split(os.sep)[0]
        top = top[:-3] if top.endswith(".rs") else top
        if top in ("lib",):
            continue
        text = open(path, errors="replace").read()
        # Inline `#[cfg(test)] mod tests { ... }` blocks count as test lines from that marker on.
        cut = text.find("#[cfg(test)]")
        is_test = bool(TEST.search(rel)) or top == "test"
        code, test = (text, "") if cut < 0 else (text[:cut], text[cut:])
        if is_test:
            code, test = "", text
        lines[top][0] += code.count("\n")
        lines[top][1] += test.count("\n")
        files[top] += 1
        for m in ref.finditer(code):
            b = m[1]
            if b in mods and b != top:
                e = edges[(top, b)]
                e[0] += 1
                e[1].add(rel)

# Tarjan's strongly connected components over code (non-test) references.
graph = defaultdict(set)
for a, b in edges:
    graph[a].add(b)
index, low, stack, on, sccs, i = {}, {}, [], set(), [], [0]

def visit(v):
    index[v] = low[v] = i[0]; i[0] += 1
    stack.append(v); on.add(v)
    for w in graph[v]:
        if w not in index:
            visit(w); low[v] = min(low[v], low[w])
        elif w in on:
            low[v] = min(low[v], index[w])
    if low[v] == index[v]:
        comp = []
        while True:
            w = stack.pop(); on.discard(w); comp.append(w)
            if w == v:
                break
        sccs.append(sorted(comp))

sys.setrecursionlimit(10000)
for v in list(lines):
    if v not in index:
        visit(v)

fan_in = defaultdict(int)
fan_out = defaultdict(int)
for a, b in edges:
    fan_out[a] += 1
    fan_in[b] += 1
data = {
    "modules": [{"name": m, "code": lines[m][0], "test": lines[m][1], "files": files[m],
                 "fan_in": fan_in[m], "fan_out": fan_out[m]} for m in sorted(lines)],
    "edges": [{"from": a, "to": b, "refs": r, "files": len(f)} for (a, b), (r, f) in sorted(edges.items())],
    "cycles": [c for c in sccs if len(c) > 1],
}
json.dump(data, open(out, "w"), indent=1)
print(f"modules {len(data['modules'])}, edges {len(data['edges'])}, "
      f"cycle groups {len(data['cycles'])}: {[len(c) for c in data['cycles']]}")
for c in data["cycles"]:
    print("  cycle:", " ".join(c))
top = sorted(data["modules"], key=lambda m: -m["code"])[:12]
print("biggest:", ", ".join(f"{m['name']} {m['code']}+{m['test']}t in{m['fan_in']} out{m['fan_out']}" for m in top))

here = os.path.dirname(os.path.abspath(__file__))
mods_js = json.dumps([[m["name"], m["code"], m["test"], m["files"]] for m in data["modules"]], separators=(",", ":"))
edges_js = json.dumps([[e["from"], e["to"], e["refs"]] for e in data["edges"]], separators=(",", ":"))
page = open(os.path.join(here, "page.html")).read().replace("/*DATA*/", f"const MODS={mods_js};\nconst EDGES={edges_js};")
open(os.path.join(outdir, "module-map.html"), "w").write(page)
