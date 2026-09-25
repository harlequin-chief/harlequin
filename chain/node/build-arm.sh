#!/bin/bash
# build-arm.sh — cross-compile the Harlequin node for aarch64 (glibc) = small ARM devices (e.g. proot Debian on a tablet or phone).
# Recipe recovered from bitacora 2026-07-01c (the env-based cross setup that was never persisted to a
# config file → the 07-21 rollout hit "linking with cc failed" because plain `cargo --target aarch64`
# used the HOST linker). This script is the durable home for it. Needs: rustup target aarch64-…-gnu +
# apt gcc/g++-aarch64-linux-gnu. ALWAYS --features mainnet for a live-chain binary (golden rule).
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

# --- REMAPPED PATHS (2026-09-07) ----------------------------------------------------------------
# WHY. The published binary carried its build path inside: `/home/<account>/…` appeared 3,550 times
# in the x86 build on /dist and 3,896 in the ARM one. Two consequences, both bad:
#   1. A LEAK. In a project whose whole point is taking part without giving an identity, we were
#      handing out the account name of the build machine. Not a real name, but a stable identifier,
#      which is exactly what ties things together.
#   2. IRREPRODUCIBILITY. The sha only came out the same when building in the SAME absolute path —
#      measured: the same tag in another folder gives another sha; in the same folder, the same one,
#      bit for bit. So the number we published for anyone to verify us could NOT be reproduced by
#      anyone outside, and their only possible conclusion was that we were lying.
# One fix covers both. NOTE: all THREE families must be remapped — most occurrences (3,457 of
# 3,550) came from cargo's registry, NOT from our code, so moving the project folder would not help.
# The targets are fixed and neutral ON PURPOSE: whoever wants to verify us must use these same ones.
export RUSTFLAGS="${RUSTFLAGS:-} \
  --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/hlq-build/cargo \
  --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/hlq-build/rustup \
  --remap-path-prefix=$(cd .. && pwd)=/hlq-build/harlequin"
# ...AND THE RUNTIME WASM TOO (2026-09-17). substrate-wasm-builder compiles the runtime in ANOTHER cargo
# invocation and OVERWRITES RUSTFLAGS with its own (wasm_project.rs: `.env("RUSTFLAGS", rustflags)`), so
# the remap above never reached it. Measured: the spec-3 runtime LIVE ON CHAIN carries 96
# `/home/<account>/…` paths (cargo registry, rustup and the project folder) and no `/hlq-build/`. The
# only door the builder leaves open is WASM_BUILD_RUSTFLAGS. Check with ops/hlq-build-paths.py.
export WASM_BUILD_RUSTFLAGS="${WASM_BUILD_RUSTFLAGS:-} --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/hlq-build/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/hlq-build/rustup --remap-path-prefix=$(cd .. && pwd)=/hlq-build/harlequin"


export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
export AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_gnu="--sysroot=/usr/aarch64-linux-gnu"
cargo build --release --features mainnet --target aarch64-unknown-linux-gnu || exit $?
BIN="${CARGO_TARGET_DIR:-target}/aarch64-unknown-linux-gnu/release/harlequin-node"

# LOCK (2026-09-17): the build FAILS if the binary — or the compressed runtime wasm inside it — still
# carries paths of this machine. Warning is not enough: the remap existed since 2026-09-07 and the wasm
# silently lost it, because nobody looked inside. Looks for the lock next to the project; fails if absent.
GUARD_PY="${HLQ_BUILD_PATHS_GUARD:-$(cd .. && cd .. && pwd)/ops/hlq-build-paths.py}"
[ -f "$GUARD_PY" ] || { echo ">>> ❌ falta $GUARD_PY: cannot check paths, the binary is NOT valid" >&2; exit 3; }
# The exit code is kept BEFORE filtering the output: in a pipeline without pipefail the last command
# wins, and a `| sed` would always give 0 — a lock that never closes.
GUARD_OUT=$(python3 "$GUARD_PY" "$BIN"); GUARD_RC=$?
printf '%s\n' "$GUARD_OUT" | sed -E 's/accounts=.*/accounts=<redacted>/'
if [ "$GUARD_RC" -ne 0 ]; then
  echo ">>> ❌ the binary carries paths of this machine (or could not be inspected, rc=$GUARD_RC): NOT publishable" >&2
  exit 4
fi
sha256sum "$BIN"
file "$BIN"
