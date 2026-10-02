#!/bin/bash
# Build the browser wallet (wasm + JS glue) for the site, with NO build-host paths inside.
#
# Why (2026-10-01): the wasm the site served since July carried the cargo registry path and the folder of the
# machine that built it (panic locations of every crate). Same leak the node fixed on 2026-09-07 with
# --remap-path-prefix; the wallet has its own pipeline and never got it. This script remaps the three families
# (cargo registry, rustup, this repository) to fixed neutral prefixes and refuses to finish if any path remains.
#
# Needs: rustup target wasm32-unknown-unknown, wasm-bindgen-cli at the SAME version as Cargo.lock's
# wasm-bindgen (the JS glue and the wasm must come from the same version).
# Output: ./pkg/wallet_wasm.js + ./pkg/wallet_wasm_bg.wasm (+ .d.ts), then copied by hand to web/pkg and
# web/villa/entrar/wallet.
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$(cd .. && pwd)"
CARGO_HOME_DIR="${CARGO_HOME:-$HOME/.cargo}"
RUSTUP_HOME_DIR="${RUSTUP_HOME:-$HOME/.rustup}"
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=$CARGO_HOME_DIR=/hlq-build/cargo --remap-path-prefix=$RUSTUP_HOME_DIR=/hlq-build/rustup --remap-path-prefix=$ROOT=/hlq-build/harlequin"
want=$(awk '/^name = "wasm-bindgen"$/{getline; gsub(/version = |"/,""); print; exit}' Cargo.lock)
have=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')
[ "$want" = "$have" ] || { echo "wasm-bindgen-cli $have != Cargo.lock $want (cargo install wasm-bindgen-cli --version $want --locked)" >&2; exit 2; }
cargo build --release --locked --target wasm32-unknown-unknown
rm -rf pkg && mkdir pkg
wasm-bindgen --target web --out-dir pkg "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/wallet_wasm.wasm"
# The lock: no path of this machine may survive in what we publish.
# Every shipped file (wasm, JS glue, typings), fixed strings so a path with regex metacharacters still matches.
if cat pkg/wallet_wasm_bg.wasm pkg/wallet_wasm.js pkg/*.d.ts | strings -n 6 \
   | grep -F -e "$CARGO_HOME_DIR" -e "$RUSTUP_HOME_DIR" -e "$ROOT" -e "/home/" -e "/root/" ; then
  echo "❌ the wasm still carries build-host paths: NOT publishable" >&2; exit 4
fi
sha256sum pkg/wallet_wasm_bg.wasm pkg/wallet_wasm.js
