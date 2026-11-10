#!/bin/sh
# Run what CI runs, in CI's order, and stop at the first failure with the
# step's name. Run it before a push.
#
#   scripts/check.sh           # every step
#   scripts/check.sh --rust    # steps 1 to 7: the reaper sidecar, then cargo and the line cap
#   scripts/check.sh --web     # step 8: the frontend's typecheck, lint and tests
#
# Not here: `pnpm tauri build --no-bundle`, the release app build, which stays
# CI's alone. Iterate with `cargo test --workspace --lib --bins` instead.
#
# The 1.93 check (step 7) is skipped, with a line saying so, when `rustup` is
# missing or has no 1.93 toolchain: a worker's sandbox cannot install one.
# CI still runs it.
set -eu

# Every path below is relative to the repo's root, so run it from there.
if [ ! -f scripts/check.sh ]; then
  echo "check.sh: run it from the repo's root: scripts/check.sh" >&2
  exit 2
fi

if [ "$#" -gt 1 ]; then
  echo "check.sh: expected one argument at most" >&2
  exit 2
fi

rust=1
web=1
case "${1:-}" in
  "") ;;
  --rust) web=0 ;;
  --web) rust=0 ;;
  *)
    echo "check.sh: unknown argument: $1 (expected --rust or --web)" >&2
    exit 2
    ;;
esac

# `set -e` stops at the first failing command. The trap then names the step it
# stopped in, which each step sets in `step` before it runs. No function or
# variable stands in for a command, so every command below is written out.
step=""
code=0
trap 'code=$?; if [ "$code" -ne 0 ]; then echo "check.sh: FAILED at step: $step (exit $code)" >&2; fi' EXIT

if [ "$rust" -eq 1 ]; then
  step="1. build the reaper sidecar"; echo "==> $step"
  scripts/build-reaper.sh --debug

  step="2. cargo fmt"; echo "==> $step"
  cargo fmt --all --check

  step="3. cargo clippy"; echo "==> $step"
  cargo clippy --workspace --all-targets --locked -- -D warnings

  step="4. cargo test (doctests included)"; echo "==> $step"
  cargo test --workspace --locked

  step="5. cargo doc"; echo "==> $step"
  RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

  step="6. file size cap"; echo "==> $step"
  python3 -I scripts/file-size.py

  step="7. cargo check on 1.93"; echo "==> $step"
  if command -v rustup >/dev/null 2>&1 && rustup toolchain list | grep -q '^1\.93'; then
    cargo +1.93 check --workspace --all-targets --locked
  else
    echo "skipped: rustup has no 1.93 toolchain (CI runs it)"
  fi
fi

if [ "$web" -eq 1 ]; then
  step="8a. pnpm typecheck"; echo "==> $step"
  pnpm typecheck

  step="8b. pnpm lint"; echo "==> $step"
  pnpm lint

  step="8c. pnpm test"; echo "==> $step"
  pnpm test
fi

step="done"
echo "check.sh: all steps passed"
