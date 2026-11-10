#!/bin/sh
# Build kelpie-reaper and put it where Tauri's sidecar rules say it must be.
#
# `bundle.externalBin` in src-tauri/tauri.conf.json names
# `binaries/kelpie-reaper-sidecar`, and Tauri reads that as the file
# `src-tauri/binaries/kelpie-reaper-sidecar-<target triple>`. The triple is the
# one Tauri is building for, which it passes as `TAURI_ENV_TARGET_TRIPLE`, and
# `rustc --print host-tuple` when that is unset. `tauri-build` copies the file
# next to the app binary with the triple dropped from the name, which is where
# `state::reaper_path` looks for it, and each OS bundle puts a sidecar beside
# the binary it bundles.
#
# The name is not `kelpie-reaper`, the cargo bin's: `tauri-build` deletes the
# file at the name it copies to, so a sidecar named like the bin would replace
# the one cargo built, and `cargo test` would run whichever copy came last.
#
# Nothing that builds src-tauri works without the file: src-tauri/build.rs stops
# when it is missing. So run this before `pnpm tauri dev`, `pnpm tauri build`
# and any cargo command that compiles the app crate.
# `.github/workflows/test.yml` runs it in every job that builds src-tauri.
#
#   scripts/build-reaper.sh            # the profile Tauri is building: release,
#                                      # or debug when TAURI_ENV_DEBUG is set, as
#                                      # `tauri dev` and `tauri build --debug` set it
#   scripts/build-reaper.sh --debug    # a debug build
#   scripts/build-reaper.sh --release  # a release build
#
# Windows: this is a POSIX shell script, so a Windows build needs the same two
# steps in a form a POSIX shell cannot run.
set -eu

cd "$(dirname "$0")/.."

profile=release
flag=--release
if [ -n "${TAURI_ENV_DEBUG:-}" ]; then
  profile=debug
  flag=
fi
case "${1:-}" in
  "") ;;
  --debug)
    profile=debug
    flag=
    ;;
  --release)
    profile=release
    flag=--release
    ;;
  *)
    echo "build-reaper.sh: unknown argument: $1" >&2
    exit 2
    ;;
esac

# Where cargo puts the binary, which `CARGO_TARGET_DIR` moves when it is set.
target=${CARGO_TARGET_DIR:-target}

host=$(rustc --print host-tuple)
want=${TAURI_ENV_TARGET_TRIPLE:-$host}

# A universal macOS build needs one sidecar per architecture.
case "$want" in
  universal-apple-darwin) triples="aarch64-apple-darwin x86_64-apple-darwin" ;;
  *) triples=$want ;;
esac

mkdir -p src-tauri/binaries
for triple in $triples; do
  # The extension Tauri builds with on Windows.
  exe=""
  case "$triple" in
    *windows*) exe=.exe ;;
  esac

  # `--target` moves the binary into a folder named for the triple, so it is
  # passed only when the build is not for the host.
  if [ "$triple" = "$host" ]; then
    out=$target/$profile
    set -- --bin kelpie-reaper
  else
    out=$target/$triple/$profile
    set -- --bin kelpie-reaper --target "$triple"
  fi

  # `flag` is empty for a debug build, and empty is no argument at all here.
  # shellcheck disable=SC2086
  cargo build -p kelpie-reaper "$@" $flag

  cp "$out/kelpie-reaper$exe" "src-tauri/binaries/kelpie-reaper-sidecar-$triple$exe"
  echo "build-reaper.sh: src-tauri/binaries/kelpie-reaper-sidecar-$triple$exe ($profile)"
done
