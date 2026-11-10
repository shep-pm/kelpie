# The mechanical half of IR-48: no `.rs` file over 1000 lines.
#
# The one source of the cap: `.github/workflows/file-size.yml` runs this, and
# `scripts/check.sh` runs it before a push. Run it from the repo root as
# `python3 -I scripts/file-size.py`.
#
# Copied from shep-pm/rust-house-style's `.github/workflows/file-size.yml@v0.1.0`,
# where it is inline in a reusable workflow. Re-copy it when that changes.
#
# It counts untracked files too, so a new file is caught before it is added.
#
# Inputs, as environment variables with the reusable workflow's defaults:
#   EXCLUDE   globs of `.rs` files to skip, one per line (default: none)
#   BASELINE  the known-offenders list, `<lines> <path>` per line; absent is
#             empty (default: `.github/rust-file-size-baseline.txt`)
#   RATCHET   `true` makes a stale baseline entry an error, not a notice
#             (default: false)
import os
import subprocess
import sys

LIMIT = 1000
baseline_path = os.environ.get("BASELINE", ".github/rust-file-size-baseline.txt")
ratchet = os.environ.get("RATCHET") == "true"
exclude = [g.strip() for g in os.environ.get("EXCLUDE", "").splitlines() if g.strip()]
failed = False


def report(level, path, message):
    print(f"::{level} file={path},title=IR-48::{message}")


def fail(path, message):
    global failed
    failed = True
    report("error", path, message)


ceilings = {}
if baseline_path and os.path.isfile(baseline_path):
    with open(baseline_path, encoding="utf-8") as fh:
        for number, raw in enumerate(fh, 1):
            line = raw.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split(None, 1)
            if len(parts) != 2 or not parts[0].isdigit():
                fail(baseline_path, f"line {number}: expected `<lines> <path>`, got `{line}`")
            elif parts[1] in ceilings:
                fail(baseline_path, f"line {number}: `{parts[1]}` is listed twice")
            else:
                ceilings[parts[1]] = int(parts[0])
else:
    print(f"no baseline at `{baseline_path}`, treating it as empty")

pathspecs = ["*.rs"] + [f":(exclude,glob){g}" for g in exclude]
listed = subprocess.run(
    ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", *pathspecs],
    check=True,
    capture_output=True,
).stdout.decode("utf-8").split("\0")

counts = {}
for path in filter(None, listed):
    try:
        with open(path, "rb") as fh:
            data = fh.read()
    except OSError:
        continue
    counts[path] = data.count(b"\n") + (1 if data and not data.endswith(b"\n") else 0)

over = 0
for path, n in sorted(counts.items(), key=lambda item: -item[1]):
    ceiling = ceilings.get(path)
    if ceiling is None:
        if n > LIMIT:
            over += 1
            fail(path, f"{n} lines, over {LIMIT}. Split it (IR-48).")
        continue
    if n > LIMIT:
        over += 1
    if n > ceiling:
        fail(path, f"{n} lines, but {baseline_path} allows {ceiling}. A listed file may shrink but not grow.")
    elif n <= LIMIT:
        message = f"{n} lines, now within the limit. Remove it from {baseline_path}."
        fail(path, message) if ratchet else report("notice", path, message)
    elif n < ceiling:
        message = f"{n} lines, but {baseline_path} says {ceiling}. Lower the entry to {n}."
        fail(path, message) if ratchet else report("notice", path, message)

for path in ceilings:
    if path not in counts:
        message = f"{baseline_path} lists a file that is not a counted .rs file. Remove the entry."
        fail(path, message) if ratchet else report("notice", path, message)

print(f"checked {len(counts)} .rs files, {over} over {LIMIT}, {len(ceilings)} baselined")
sys.exit(1 if failed else 0)
