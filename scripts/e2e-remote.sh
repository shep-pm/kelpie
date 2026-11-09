#!/bin/sh
# Runs on the e2e host, in a copy of the worktree: `shep-kelpie lease e2e`
# copies the folder over, runs this and copies test-results/ back. It installs
# nothing system-wide, and everything stays in the foreground so a hangup ends it.
set -eu

# Outside the copied folder, so the next run builds incrementally.
CARGO_TARGET_DIR="$HOME/kelpie-e2e-target"
export CARGO_TARGET_DIR

# A failed run must not leave the last run's results to be copied back.
rm -rf test-results
mkdir test-results

pnpm install --frozen-lockfile
pnpm tauri build --debug --no-bundle

xvfb-run -a pnpm exec wdio run e2e/wdio.conf.ts
