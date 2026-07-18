#!/usr/bin/env bash
# install-harlequin-node.sh — ONE installer for a Harlequin follower node. Any supported box, one line.
#
# Served at https://harlequinproject.org/install-harlequin-node.sh and run as:
#
#   servers / PCs (systemd):   curl -fsSL https://harlequinproject.org/install-harlequin-node.sh | sudo bash
#   Android tablet/phone:      (Termux)  pkg install -y proot-distro && proot-distro install debian \
#                                        && proot-distro login debian
#                              (inside Debian, with your VPN on)
#                                        curl -fsSL https://harlequinproject.org/install-harlequin-node.sh | bash
#
# It detects your CPU (x86_64 / aarch64) and your init system, then does ONLY this:
#   1. downloads the right node binary + the launch chain spec from the published distribution.
#   2. VERIFIES the sha256 of BOTH against the pinned values below — ABORTS on any mismatch.
#   3. systemd host  → installs an unprivileged system service (survives reboots).
#      no systemd (Android proot, containers) → PORTABLE mode: installs under ~/harlequin and runs the
#      node in the foreground (Ctrl+C stops it; re-run `~/harlequin/run-node.sh` to start again).
#   Force portable mode on a systemd host with:  HLQ_PORTABLE=1
#
# OPSEC / sovereignty: this script NEVER asks for, captures, or generates your ACCOUNT secret. Your
# account (your mask in the society) is created by YOU in the browser (/rito.html) or with
# `harlequin-node key generate`, and stored in YOUR password manager. A node needs no account to sync
# and serve. Run it behind your VPN: a node announces its IP to peers.
#
# Idempotent: safe to re-run. Re-running re-verifies and restarts; it never overwrites an existing node-key.
set -euo pipefail

# ─────────────────────────────────────────────────────────────────────────────
# Pinned distribution (re-pinned on every release; sha256 is the security boundary of this script).
DIST_BASE="https://harlequinproject.org"
SPEC_URL="$DIST_BASE/dist/mainnet-raw.json"
SPEC_SHA256="ba1b25f7179d24c89aabd0a5f924d06f15365e1040cff3be2811a771e42086a6"  # SEALED launch chainspec (genesis 2026-07-18)

BIN_URL_x86_64="$DIST_BASE/dist/harlequin-node"
BIN_SHA_x86_64="fa79dda97e0f335eed274ec89c7a80c51ef7ff60eb979f11a76df7336e6666a6" # FINAL launch mainnet binary (3-band closed)
BIN_URL_aarch64="$DIST_BASE/dist/harlequin-node-arm64"
BIN_SHA_aarch64="91205a74e8379051e0da9769506c859f290555a96b269334c149399774802f4b" # launch aarch64 (cross-built; wasm runtime byte-identical to x86_64 fa79dda9)

# Baked into the spec too; passed explicitly in portable mode for first-dial robustness.
BOOTNODE="/ip4/95.133.166.93/tcp/30333/p2p/12D3KooWBLjMD2oEZvNVZXFSHdRRS62gbZsgfYcR6rkkjygJ2emR"
SVC="harlequin-node"

# Weak-subjectivity checkpoint (M1) — re-pinned on EVERY release, next to the sha256 pins above.
# The pinned block must be FINALIZED. After install the running node is checked against it: if the
# chain your peers serve does not contain exactly this block hash at this height, the node is STOPPED
# (fail-closed). A long-range attacker can grow a longer fork from old keys; they cannot forge this pin.
# Before trusting a fresh copy of this script, verify these values against AT LEAST TWO independent
# sources: (1) this script over HTTPS, (2) https://harlequinproject.org/network.html, (3) a node
# operator you already trust. If the sources disagree — STOP, do not join.
# F4 NOTE: values below pin the LAUNCH chain at genesis. At the relaunch ceremony (F4) they are
# re-pinned to a fresh finalized checkpoint, and on every release thereafter.
CHECKPOINT_HEIGHT="235"
CHECKPOINT_HASH="0x5551222d8fda1625486444f5392b9055b7ffc2a1033eb947bae25694d486d9e8"
CHECKPOINT_CHAIN="Harlequin Launch (cold-start)"  # system_chain name (spec id: hlq_launch)
CHECKPOINT_PINNED_AT="2026-07-18"  # re-pin to a fresher finalized block before each public release
RPC_URL="http://127.0.0.1:9944"   # node RPC is local-only by default; the check runs on YOUR box
# ─────────────────────────────────────────────────────────────────────────────

die() { echo "  ✗ $*" >&2; exit 1; }
ok()  { echo "  ✓ $*"; }
info(){ echo "    $*"; }

echo
echo "  HARLEQUIN — node installer"
echo "  ────────────────────────────────────────────"

# 1. CPU architecture → binary + sha
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64)  BIN_URL="$BIN_URL_x86_64";  BIN_SHA256="$BIN_SHA_x86_64" ;;
  aarch64) BIN_URL="$BIN_URL_aarch64"; BIN_SHA256="$BIN_SHA_aarch64" ;;
  *) die "unsupported CPU '$ARCH' (supported: x86_64, aarch64)." ;;
esac
ok "cpu: $ARCH"

# 2. install mode: systemd service vs portable foreground
MODE="portable"
if [ -z "${HLQ_PORTABLE:-}" ] && [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
  MODE="service"
fi
ok "mode: $MODE"

command -v curl >/dev/null 2>&1 || {
  # bootstrap curl where we can (fresh proot Debian has apt and we are root there)
  if command -v apt-get >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
    apt-get update -y >/dev/null 2>&1 || true
    DEBIAN_FRONTEND=noninteractive apt-get install -y curl ca-certificates >/dev/null 2>&1 || true
  fi
  command -v curl >/dev/null 2>&1 || die "curl is required."
}
command -v sha256sum >/dev/null 2>&1 || die "sha256sum is required (coreutils)."

if [ "$MODE" = "service" ]; then
  [ "$(id -u)" -eq 0 ] || die "service install needs root (pipe to 'sudo bash'). Or force HLQ_PORTABLE=1."
  PREFIX="/opt/harlequin"
else
  PREFIX="${HOME:-/root}/harlequin"
  # runtime libs the binary links against (readelf: libstdc++6 + libgcc-s1); root+apt only, else assume present
  if command -v apt-get >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
    ok "installing runtime libs…"
    apt-get update -y >/dev/null 2>&1 || true
    DEBIAN_FRONTEND=noninteractive apt-get install -y libstdc++6 libgcc-s1 ca-certificates >/dev/null 2>&1 \
      || die "could not install libstdc++6/libgcc-s1 (apt)."
  fi
fi

NODE_NAME="${HLQ_NODE_NAME:-hlq-$(printf '%04x' $((RANDOM)))}"   # pseudonymous by default (no identity link)
NODE_NAME="$(printf '%s' "$NODE_NAME" | tr -cd 'A-Za-z0-9-' | cut -c1-32)"; [ -n "$NODE_NAME" ] || NODE_NAME="hlq-node"

# 3. download to a temp dir that is always cleaned.
#    HTTPS-only, TLS>=1.2, no redirects: the URLs are fixed and same-origin, so a redirect would mean
#    someone is steering us elsewhere. (sha256 below is the real backstop.)
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
DL='curl -fsS --proto =https --tlsv1.2 --connect-timeout 15 --max-time 900'
[ "${BIN_URL#https://}"  != "$BIN_URL"  ] || die "binary URL is not https."
[ "${SPEC_URL#https://}" != "$SPEC_URL" ] || die "spec URL is not https."
ok "downloading node binary ($ARCH)…"; $DL "$BIN_URL"  -o "$TMP/harlequin-node"   || die "download failed: $BIN_URL"
ok "downloading launch chain spec…";   $DL "$SPEC_URL" -o "$TMP/mainnet-raw.json" || die "download failed: $SPEC_URL"

# 4. verify sha256 — ABORT on mismatch (the security boundary of this script)
verify() { # <file> <expected>
  local got; got="$(sha256sum "$1" | cut -d' ' -f1)"
  [ "$got" = "$2" ] || die "sha256 MISMATCH for $(basename "$1")
      expected: $2
      got:      $got
    Aborting — do NOT trust this file."
}
verify "$TMP/harlequin-node"   "$BIN_SHA256";  ok "binary sha256 verified."
verify "$TMP/mainnet-raw.json" "$SPEC_SHA256"; ok "chain spec sha256 verified."

# 4b. weak-subjectivity verifier (M1) — installed next to the node, safe to re-run any time.
#     Exit codes: 0 = checkpoint verified · 1 = MISMATCH (wrong chain) · 2 = local RPC unreachable.
#     Both non-zero outcomes are treated FAIL-CLOSED by this installer: the node is stopped.
cat > "$TMP/verify-checkpoint.sh" <<VCEOF
#!/usr/bin/env bash
# verify-checkpoint.sh — Harlequin weak-subjectivity check (M1). Re-run any time.
# Pinned at install time ($CHECKPOINT_PINNED_AT); re-pinned on every release.
# Verify these values against >=2 independent sources (installer over HTTPS,
# https://harlequinproject.org/network.html, an operator you trust) before trusting them.
CHECKPOINT_HEIGHT="$CHECKPOINT_HEIGHT"
CHECKPOINT_HASH="$CHECKPOINT_HASH"
CHECKPOINT_CHAIN="$CHECKPOINT_CHAIN"
CHECKPOINT_PINNED_AT="$CHECKPOINT_PINNED_AT"
RPC_URL="$RPC_URL"
VCEOF
cat >> "$TMP/verify-checkpoint.sh" <<'VCEOF'
set -euo pipefail
rpc() { curl -fsS -m 10 -H 'Content-Type: application/json' -d "$1" "$RPC_URL" 2>/dev/null || true; }
res() { printf '%s' "$1" | grep -o '"result":"[^"]*"' | head -1 | cut -d'"' -f4 || true; }

# 1. wait for the LOCAL node RPC (up to ~60s)
CH=""
for _ in $(seq 1 30); do
  CH="$(res "$(rpc '{"id":1,"jsonrpc":"2.0","method":"system_chain","params":[]}')")"
  [ -n "$CH" ] && break
  sleep 2
done
if [ -z "$CH" ]; then
  echo "  ✗ node RPC not reachable at $RPC_URL — checkpoint NOT verified." >&2
  exit 2
fi

# 2. chain identity
if [ "$CH" != "$CHECKPOINT_CHAIN" ]; then
  echo "  ✗ CHECKPOINT FAIL: node chain is '$CH', pinned '$CHECKPOINT_CHAIN'." >&2
  exit 1
fi

# 3. wait until the pinned height exists locally, then compare. Waiting is safe;
#    joining the wrong chain is not — so this loop never gives up on its own.
GOT=""
while :; do
  GOT="$(res "$(rpc "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"chain_getBlockHash\",\"params\":[$CHECKPOINT_HEIGHT]}")")"
  [ -n "$GOT" ] && break
  BESTHEX="$(printf '%s' "$(rpc '{"id":1,"jsonrpc":"2.0","method":"chain_getHeader","params":[]}')" | grep -o '"number":"0x[0-9a-fA-F]*"' | head -1 | cut -d'"' -f4 || true)"
  echo "    syncing… best block $(( ${BESTHEX:-0} )) / checkpoint $CHECKPOINT_HEIGHT — waiting."
  sleep 15
done

if [ "$GOT" = "$CHECKPOINT_HASH" ]; then
  echo "  ✓ weak-subjectivity checkpoint verified: block $CHECKPOINT_HEIGHT = $CHECKPOINT_HASH (pinned $CHECKPOINT_PINNED_AT)."
  exit 0
fi
echo "  ✗ CHECKPOINT MISMATCH at block $CHECKPOINT_HEIGHT" >&2
echo "      pinned: $CHECKPOINT_HASH" >&2
echo "      got:    $GOT" >&2
echo "    Your peers are serving a DIFFERENT chain (possible long-range attack or wrong network)." >&2
echo "    Do NOT trust this node. Check https://harlequinproject.org/network.html and ask an operator you trust." >&2
exit 1
VCEOF
chmod +x "$TMP/verify-checkpoint.sh"

# A fresh node is a FOLLOWER (observer): it syncs, verifies, serves and relays, but does NOT validate —
# validating requires earned reputation (committee membership), by design. So no --validator anywhere here.
# --state/blocks-pruning archive: the finality committee for each block is read from on-chain state at that
# block's EPOCH-START; default pruning keeps only the last 256 states, so the read would return EMPTY and
# finality would stall (the #270 stall). Archive keeps all state so the read always resolves.

if [ "$MODE" = "service" ]; then
  # ── service install (unprivileged system user + hardened unit) ─────────────
  id harlequin >/dev/null 2>&1 || { useradd --system --home-dir "$PREFIX" --shell /usr/sbin/nologin harlequin; ok "created system user 'harlequin'."; }
  install -d -o harlequin -g harlequin -m 0750 "$PREFIX" "$PREFIX/data"
  install -o root -g root -m 0755 "$TMP/harlequin-node"   "$PREFIX/harlequin-node"
  install -o harlequin -g harlequin -m 0644 "$TMP/mainnet-raw.json" "$PREFIX/mainnet-raw.json"
  install -o root -g root -m 0755 "$TMP/verify-checkpoint.sh" "$PREFIX/verify-checkpoint.sh"
  ok "installed binary + spec under $PREFIX."

  # node-key (network identity, NOT an account; never overwritten)
  if [ ! -f "$PREFIX/node-key" ]; then
    runuser -u harlequin -- "$PREFIX/harlequin-node" key generate-node-key --file "$PREFIX/node-key" >/dev/null 2>&1 \
      || die "could not generate node-key."
    chown harlequin:harlequin "$PREFIX/node-key"; chmod 600 "$PREFIX/node-key"
    ok "generated network node-key (0600)."
  else
    ok "node-key already present (left untouched)."
  fi

  cat > "/etc/systemd/system/${SVC}.service" <<UNIT
[Unit]
Description=Harlequin node
After=network-online.target
Wants=network-online.target

[Service]
User=harlequin
Group=harlequin
ExecStart=$PREFIX/harlequin-node \\
  --base-path $PREFIX/data \\
  --chain $PREFIX/mainnet-raw.json \\
  --node-key-file $PREFIX/node-key \\
  --name "$NODE_NAME" \\
  --port 30333 \\
  --consensus woven-trust-12000 \\
  --network-backend libp2p \\
  --state-pruning archive \\
  --blocks-pruning archive
Restart=on-failure
RestartSec=5
LimitNOFILE=65536
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=$PREFIX/data
PrivateTmp=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictNamespaces=true

[Install]
WantedBy=multi-user.target
UNIT
  systemctl daemon-reload
  systemctl enable "$SVC" >/dev/null 2>&1 || true
  systemctl restart "$SVC"
  ok "service '${SVC}' enabled and started."

  # weak-subjectivity check (M1). Two distinct failures (review 2026-07-04, H3):
  #   exit 1 = checkpoint MISMATCH → hostile/wrong chain → stop AND disable (fail-closed).
  #   exit 2 = local RPC unreachable (transient/slow hardware) → NOT an attack signal: leave the
  #            node running UNVERIFIED and demand a manual re-run — disabling here would be a
  #            self-inflicted DoS on slow boxes.
  ok "verifying weak-subjectivity checkpoint (block $CHECKPOINT_HEIGHT)…"
  VC_RC=0; "$PREFIX/verify-checkpoint.sh" || VC_RC=$?
  if [ "$VC_RC" -eq 1 ]; then
    systemctl stop "$SVC" || true
    systemctl disable "$SVC" >/dev/null 2>&1 || true
    die "checkpoint MISMATCH — node stopped and disabled (fail-closed).
    Re-check your network and the pinned values against >=2 independent sources, then re-run this installer."
  elif [ "$VC_RC" -ne 0 ]; then
    echo "  ⚠ could not reach the node's local RPC — checkpoint NOT verified yet." >&2
    echo "    The node keeps running, but do NOT trust it until this passes:" >&2
    echo "        $PREFIX/verify-checkpoint.sh" >&2
  fi

  echo "  ────────────────────────────────────────────"
  ok "Node up. Open p2p port 30333/tcp in your firewall to accept peers."
  info "logs:    journalctl -u ${SVC} -f"
  info "status:  systemctl status ${SVC}"
  info "re-verify checkpoint any time: $PREFIX/verify-checkpoint.sh"
  info "account: $PREFIX/harlequin-node key generate   (store the phrase in YOUR password manager)"
  echo "  Your node, your keys, no master."
  echo

else
  # ── portable install (no systemd: Android proot, containers) ────────────────
  mkdir -p "$PREFIX/data"
  install -m 0755 "$TMP/harlequin-node"   "$PREFIX/harlequin-node"
  install -m 0644 "$TMP/mainnet-raw.json" "$PREFIX/mainnet-raw.json"
  install -m 0755 "$TMP/verify-checkpoint.sh" "$PREFIX/verify-checkpoint.sh"
  ok "installed binary + spec under $PREFIX."

  # --wasmtime-instantiation-strategy recreate-instance-copy-on-write: avoids wasmtime's pooling
  # allocator (reserves a huge mmap that dies inside proot/Android).
  # The node runs in the background of this script so the weak-subjectivity check (M1) can stop it
  # FAIL-CLOSED on mismatch; on success the script stays attached to the node (Ctrl+C stops it).
  cat > "$PREFIX/run-node.sh" <<RUN
#!/usr/bin/env bash
set -euo pipefail
cd "\$(dirname "\$0")"
./harlequin-node \\
  --base-path ./data \\
  --chain ./mainnet-raw.json \\
  --name "${NODE_NAME}" \\
  --port 30333 \\
  --consensus woven-trust-12000 \\
  --network-backend libp2p \\
  --wasmtime-instantiation-strategy recreate-instance-copy-on-write \\
  --bootnodes "${BOOTNODE}" \\
  --state-pruning archive \\
  --blocks-pruning archive &
NODE_PID=\$!
trap 'kill "\$NODE_PID" 2>/dev/null || true' INT TERM
# exit 1 = checkpoint MISMATCH → kill the node (fail-closed). exit 2 = local RPC unreachable
# (transient/slow hardware, e.g. a tablet) → keep the node running, demand a manual re-check.
VC_RC=0; ./verify-checkpoint.sh || VC_RC=\$?
if [ "\$VC_RC" -eq 1 ]; then
  kill "\$NODE_PID" 2>/dev/null || true
  echo "  ✗ node stopped: checkpoint MISMATCH (fail-closed)." >&2
  echo "    Re-check the pinned values against >=2 independent sources, then run ./run-node.sh again." >&2
  exit 1
elif [ "\$VC_RC" -ne 0 ]; then
  echo "  ⚠ could not reach the node's local RPC — checkpoint NOT verified yet." >&2
  echo "    The node keeps running, but do NOT trust it until this passes:  ./verify-checkpoint.sh" >&2
fi
wait "\$NODE_PID"
RUN
  chmod +x "$PREFIX/run-node.sh"

  echo "  ────────────────────────────────────────────"
  ok "All set. Starting your follower node '$NODE_NAME'."
  info "· First start runs the weak-subjectivity check: your node must contain the pinned block or it stops."
  info "· You'll see technical lines (Imported #… / Idle). That IS the live node — the engine, not your mask."
  info "· Your MASK (seed phrase) is created in the BROWSER: $DIST_BASE/rito.html"
  info "· Stop any time: Ctrl+C. Start again: $PREFIX/run-node.sh"
  info "· Keep your VPN on — a node announces its IP to peers."
  echo "  Your node, your keys, no master."
  echo "  ────────────────────────────────────────────"
  echo
  exec "$PREFIX/run-node.sh"
fi
