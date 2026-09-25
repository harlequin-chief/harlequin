#!/bin/bash
# Build wrapper for the Harlequin node. Exports the env native build deps need so it reaches cargo's
# build scripts (background/nohup launches strip them):
#   - LIBCLANG_PATH / LLVM_CONFIG_PATH: clang-sys/bindgen (rocksdb).   apt: libclang-dev libclang1-19 llvm-dev
#   - PROTOC: prost-build/litep2p (protobuf compiler).                  apt: protobuf-compiler
# Paths below are Debian/LLVM-19 defaults; adjust for your distro.
#
# ⚠️  MAINNET vs TESTNET (root cause of the 2026-06-30 finality halt — DO NOT repeat):
#   The `mainnet` cargo feature selects PRODUCTION consensus/reputation cadence (epoch 600, decay
#   half-life ~2y, beta=12, tau=60). WITHOUT it the build uses fast TESTNET values → on a real launch
#   the founder committee decays to 0 in ~5 epochs → finality halts. PRODUCTION builds MUST pass
#   `--mainnet`. The default stays testnet (fast validation), but prints a loud warning so a testnet
#   binary is never shipped to production by accident.
#     ./build-node.sh            → TESTNET build (dev/validation)  [warns]
#     ./build-node.sh --mainnet  → MAINNET build (production)
set -uo pipefail
export LIBCLANG_PATH="${LIBCLANG_PATH:-/usr/lib/x86_64-linux-gnu}"
export LLVM_CONFIG_PATH="${LLVM_CONFIG_PATH:-/usr/bin/llvm-config}"
export PROTOC="${PROTOC:-/usr/bin/protoc}"
cd "$(dirname "$0")" || exit 1

FEATURES=()
MODE="TESTNET"
for arg in "$@"; do
  case "$arg" in
    --mainnet) FEATURES=(--features mainnet); MODE="MAINNET";;
    --testnet) FEATURES=(); MODE="TESTNET";;
    *) echo "build-node.sh: unknown arg '$arg' (use --mainnet | --testnet)" >&2; exit 2;;
  esac
done


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


if [ "$MODE" = "MAINNET" ]; then
  echo ">>> Building MAINNET binary (--features mainnet): PRODUCTION cadence." >&2
else
  echo ">>> Building TESTNET binary (fast dev values). For PRODUCTION pass --mainnet." >&2
  echo ">>> ⚠️  A testnet binary launched on mainnet WILL halt finality (founder decay). Do NOT ship it." >&2
fi
cargo build --release "${FEATURES[@]}" || exit $?
BIN="${CARGO_TARGET_DIR:-target}/release/harlequin-node"

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
