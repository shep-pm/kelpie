#!/bin/sh
# Build kelpie-reaper and put it where Tauri's sidecar rules say it must be.
#
# `bundle.externalBin` in src-tauri/tauri.conf.json names `binaries/kelpie-reaper`,
# and Tauri reads that as the file `src-tauri/binaries/kelpie-reaper-<target
# triple>`, the triple `rustc --print host-tuple` prints. `tauri-build` copies
# the file next to the app binary with the triple dropped from the name, which
# is where `state::reaper_path` looks for it, and each OS bundle puts a sidecar
# beside the binary it bundles.
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

# The triple Tauri appends to the sidecar's name, and the extension it builds
# with on Windows.
triple=$(rustc --print host-tuple)
exe=""
case "$(uname -s)" in
  *NT*) exe=.exe ;;
esac

# Where cargo puts the binary, which `CARGO_TARGET_DIR` moves when it is set.
target=${CARGO_TARGET_DIR:-target}

# `flag` is empty for a debug build, and empty is no argument at all here.
# shellcheck disable=SC2086
cargo build -p kelpie-reaper --bin kelpie-reaper $flag

mkdir -p src-tauri/binaries
cp "$target/$profile/kelpie-reaper$exe" "src-tauri/binaries/kelpie-reaper-$triple$exe"
echo "build-reaper.sh: src-tauri/binaries/kelpie-reaper-$triple$exe ($profile)"
