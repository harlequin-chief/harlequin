#!/bin/bash
# ci-build.sh — build the Harlequin node exactly as the canonical recipe does, inside a clean Debian 13.
#
# Runs INSIDE a debian:trixie container (pinned by digest) started by .github/workflows/build.yml, with this
# repository's chain/ tree at /hlq-build/harlequin and ops/ at /hlq-build/ops (the canonical path: local crates are
# identified by their absolute path, so the sha only matches when the tree lives there).
#
#   ci-build.sh check   → cargo check of the node workspace (fast compile feedback, no artifact)
#   ci-build.sh x86     → ./build-node.sh --mainnet, then the reproducible .dist artifact in /hlq-build/out
#   ci-build.sh test    → cargo test of the lean core workspace (chain/) and of every pallet workspace
#   ci-build.sh rogue   → LAB ONLY: the node built with `rogue-author` (seals out of turn), for refusal tests
#
# Anyone can run the same thing on their own machine:
#   docker run --rm -v "$PWD/chain:/hlq-build/harlequin" -v "$PWD/ops:/hlq-build/ops" \
#     -v /hlq-build/out:/hlq-build/out debian:trixie bash /hlq-build/ops/ci-build.sh x86
set -euo pipefail
MODE="${1:?usage: ci-build.sh check|x86|test}"
TREE=/hlq-build/harlequin
OUT=/hlq-build/out
mkdir -p "$OUT"

# --- toolchain: Debian 13 packages + the rustc pinned by rust-toolchain.toml ------------------------------
# rustup comes from Debian's own package (signed archive), not from a piped installer.
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl git build-essential clang libclang-dev \
  llvm protobuf-compiler pkg-config zstd file binutils python3 rustup >/dev/null
export HOME=/root CARGO_HOME=/root/.cargo RUSTUP_HOME=/root/.rustup
export PATH="$CARGO_HOME/bin:$PATH"
# rustup dispatches on its own name: cargo/rustc links to it are the toolchain proxies the recipe expects
mkdir -p "$CARGO_HOME/bin"
for t in cargo rustc rustdoc; do ln -sf "$(command -v rustup)" "$CARGO_HOME/bin/$t"; done
NODE_TC=$(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$TREE/node/rust-toolchain.toml")
# every channel some workspace pins (node and runtime pin their own), each with the wasm target
for tc in $(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$TREE"/*/rust-toolchain.toml | sort -u); do
  # rust-src: a polkavm test fixture in the dependency tree builds core/alloc from source (-Z build-std)
  rustup -q toolchain install --profile minimal "$tc" --target wasm32v1-none --component rust-src
done
rustup default "$NODE_TC" >/dev/null   # for the workspaces that pin nothing (the lean cores)
{ echo "debian: $(cat /etc/debian_version)"; rustc +"$NODE_TC" -V; cargo +"$NODE_TC" -V
  dpkg-query -W -f='${Package} ${Version}\n' clang libclang-dev llvm protobuf-compiler build-essential rustup; } \
  | tee "$OUT/environment.txt"

# Cargo build jobs: the runner has 4 cores and 16 GB; Substrate linking peaks at ~3 GB per job.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
export CARGO_TERM_COLOR=never

# The lock the build actually resolved, back to the caller: a lock that is not in git is a binary nobody can
# reproduce (it happened once). Compared with the committed one at the end of every mode.
keep_lock() {
  cp "$TREE/node/Cargo.lock" "$OUT/node-Cargo.lock" 2>/dev/null || return 0
  if cmp -s "$OUT/node-Cargo.lock" /hlq-build/out/.committed-node-Cargo.lock; then echo "lock unchanged" > "$OUT/lock.txt"
  else echo "LOCK CHANGED: commit node-Cargo.lock before calling the build reproducible" | tee "$OUT/lock.txt"; fi
}
cp "$TREE/node/Cargo.lock" /hlq-build/out/.committed-node-Cargo.lock 2>/dev/null || true
trap keep_lock EXIT

case "$MODE" in
  check)
    cd "$TREE/node"
    cargo check --release --features mainnet 2>&1 | tee "$OUT/check.log"
    ;;
  x86)
    cd "$TREE/node"
    start=$(date +%s)
    set +e; ./build-node.sh --mainnet 2>&1 | tee "$OUT/build.log"; rc=${PIPESTATUS[0]}; set -e
    echo "rc=$rc secs=$(( $(date +%s) - start ))" | tee -a "$OUT/build.log"
    [ "$rc" = 0 ] || exit "$rc"
    cp target/release/harlequin-node.dist "$OUT/harlequin-node.dist"
    # the runtime wasm the node embeds, for the spec ceremony and the on-chain upgrade path
    find target/release/wbuild/harlequin-runtime -maxdepth 1 -name 'harlequin_runtime.compact.compressed.wasm' \
      -exec cp {} "$OUT/" \;
    ( cd "$OUT" && sha256sum harlequin-node.dist harlequin_runtime.compact.compressed.wasm 2>/dev/null > SHA256SUMS )
    python3 /hlq-build/ops/hlq-build-paths.py "$OUT/harlequin-node.dist" | sed -E 's/accounts=.*/accounts=<redacted>/' \
      | tee "$OUT/paths.txt"
    cat "$OUT/SHA256SUMS"
    ;;
  rogue)
    # LAB ONLY: the node with `rogue-author` (seals every slot) to prove that peers refuse out-of-turn blocks.
    # Not a release artifact: it is named so, and the reproducibility rules do not apply to it.
    cd "$TREE/node"
    cargo build --release --features mainnet,rogue-author 2>&1 | tee "$OUT/build.log"
    cp target/release/harlequin-node "$OUT/harlequin-node-LAB-rogue-author"
    ( cd "$OUT" && sha256sum harlequin-node-LAB-rogue-author > SHA256SUMS )
    cat "$OUT/SHA256SUMS"
    ;;
  test)
    # one target dir for every workspace: the pallets share most of the FRAME tree, so it compiles once
    export CARGO_TARGET_DIR=/hlq-build/test-target
    set +e; rc=0
    : > "$OUT/test.log"
    ( cd "$TREE" && cargo test --workspace 2>&1 ) | tee -a "$OUT/test.log"; [ "${PIPESTATUS[0]}" = 0 ] || rc=1
    for d in "$TREE"/pallet-*/ "$TREE"/runtime/; do
      [ -f "$d/Cargo.toml" ] || continue
      echo "=== $(basename "$d")" | tee -a "$OUT/test.log"
      # the runtime builds its own wasm: with a target dir outside its tree the wasm builder links sp_io twice
      # (E0152), so it keeps its own; the pallets share one
      if [ "$(basename "$d")" = runtime ]; then
        ( cd "$d" && env -u CARGO_TARGET_DIR cargo test 2>&1 ) | tee -a "$OUT/test.log"
      else
        ( cd "$d" && cargo test 2>&1 ) | tee -a "$OUT/test.log"
      fi
      [ "${PIPESTATUS[0]}" = 0 ] || { rc=1; echo "FAILED: $(basename "$d")" | tee -a "$OUT/test.log"; }
    done
    # the node's own unit tests (authorship seal, finality proofs, participation record). Its target dir stays
    # inside node/ (the wasm builder needs it there) and the runtime wasm is not needed for these tests.
    echo "=== node" | tee -a "$OUT/test.log"
    ( cd "$TREE/node" && env -u CARGO_TARGET_DIR SKIP_WASM_BUILD=1 cargo test 2>&1 ) | tee -a "$OUT/test.log"
    [ "${PIPESTATUS[0]}" = 0 ] || { rc=1; echo "FAILED: node" | tee -a "$OUT/test.log"; }
    grep -E '^test result:|^FAILED:' "$OUT/test.log" | tee "$OUT/test-summary.txt"
    exit "$rc"
    ;;
  *) echo "unknown mode $MODE" >&2; exit 2 ;;
esac
