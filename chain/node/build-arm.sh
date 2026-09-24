#!/bin/bash
# build-arm.sh — cross-compile the Harlequin node for aarch64 (glibc) = Chief's tablet (proot Debian).
# Recipe recovered from bitacora 2026-07-01c (the env-based cross setup that was never persisted to a
# config file → the 07-21 rollout hit "linking with cc failed" because plain `cargo --target aarch64`
# used the HOST linker). This script is the durable home for it. Needs: rustup target aarch64-…-gnu +
# apt gcc/g++-aarch64-linux-gnu. ALWAYS --features mainnet for a live-chain binary (golden rule).
set -euo pipefail
cd "$(dirname "$0")"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
export CXX_aarch64_unknown_linux_gnu=aarch64-linux-gnu-g++
export AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_unknown_linux_gnu="--sysroot=/usr/aarch64-linux-gnu"
cargo build --release --features mainnet --target aarch64-unknown-linux-gnu
BIN=target/aarch64-unknown-linux-gnu/release/harlequin-node
sha256sum "$BIN"
file "$BIN"
