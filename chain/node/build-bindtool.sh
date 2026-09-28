#!/bin/bash
# build-bindtool.sh — bind-tool (the NODE side of binding a node to its owner's mask) for /dist, x86_64 AND aarch64,
# with the SAME reproducibility rules as the node (build-node.sh / build-arm.sh): fixed --remap-path-prefix targets,
# stripped, no build-id, and the path guard (ops/hlq-build-paths.py) must say CLEAN or nothing is published.
# Tree must live at /hlq-build/harlequin for the sha to match anyone else's build.
# Output: target/dist/bind-tool-x86_64 and target/dist/bind-tool-aarch64 + their sha256.
# Needs: rustup target aarch64-unknown-linux-gnu + apt gcc-aarch64-linux-gnu binutils-aarch64-linux-gnu (as build-arm.sh).
set -euo pipefail
cd "$(dirname "$0")"
export RUSTFLAGS="${RUSTFLAGS:-} \
  --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/hlq-build/cargo \
  --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/hlq-build/rustup \
  --remap-path-prefix=$(cd .. && pwd)=/hlq-build/harlequin"
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc
export CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc
export AR_aarch64_unknown_linux_gnu=aarch64-linux-gnu-ar
cargo build --release --locked -p bind-tool
cargo build --release --locked -p bind-tool --target aarch64-unknown-linux-gnu
mkdir -p target/dist
T="${CARGO_TARGET_DIR:-target}"
objcopy --strip-all --remove-section=.note.gnu.build-id "$T/release/bind-tool" target/dist/bind-tool-x86_64
aarch64-linux-gnu-objcopy --strip-all --remove-section=.note.gnu.build-id \
  "$T/aarch64-unknown-linux-gnu/release/bind-tool" target/dist/bind-tool-aarch64
GUARD_PY="${HLQ_BUILD_PATHS_GUARD:-$(cd .. && cd .. && pwd)/ops/hlq-build-paths.py}"
[ -f "$GUARD_PY" ] || { echo ">>> missing $GUARD_PY: cannot check paths, NOT publishable" >&2; exit 3; }
for b in target/dist/bind-tool-x86_64 target/dist/bind-tool-aarch64; do
  # Same redaction as build-node.sh: on a hit the guard names the build account; never print that.
  RC=0; OUT=$(python3 "$GUARD_PY" "$b") || RC=$?
  printf '%s\n' "$OUT" | sed -E 's/accounts=.*/accounts=<redacted>/'
  [ "$RC" -eq 0 ] || { echo ">>> $b carries this machine's paths (rc=$RC): NOT publishable" >&2; exit 4; }
done
[ "$(cd .. && pwd)" = "/hlq-build/harlequin" ] || echo ">>> tree is not at /hlq-build/harlequin: these shas will NOT match a canonical build" >&2
echo ">>> reproducible bind-tool artifacts:"; sha256sum target/dist/bind-tool-x86_64 target/dist/bind-tool-aarch64
