#!/bin/bash
# Build wrapper for the Harlequin node. Exports the env native build deps need so it reaches cargo's
# build scripts (background/nohup launches strip them):
#   - LIBCLANG_PATH / LLVM_CONFIG_PATH: clang-sys/bindgen (rocksdb).   apt: libclang-dev libclang1-19 llvm-dev
#   - PROTOC: prost-build/litep2p (protobuf compiler).                  apt: protobuf-compiler
# Paths below are Debian/LLVM-19 defaults; adjust for your distro.
#
# ⚠️  MAINNET vs TESTNET (root cause of the finality halt — DO NOT repeat):
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

if [ "$MODE" = "MAINNET" ]; then
  echo ">>> Building MAINNET binary (--features mainnet): PRODUCTION cadence." >&2
else
  echo ">>> Building TESTNET binary (fast dev values). For PRODUCTION pass --mainnet." >&2
  echo ">>> ⚠️  A testnet binary launched on mainnet WILL halt finality (founder decay). Do NOT ship it." >&2
fi
exec cargo build --release "${FEATURES[@]}"
