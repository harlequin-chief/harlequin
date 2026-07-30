#!/usr/bin/env bash
# install-harlequin-node.sh — ONE installer for a Harlequin follower node. Any supported box, one line.
#
# Served at https://harlequinproject.org/install-harlequin-node.sh and run as:
#
#   servers / PCs, as root:    curl -fsSL https://harlequinproject.org/install-harlequin-node.sh | bash
#   servers / PCs, with sudo:  curl -fsSL https://harlequinproject.org/install-harlequin-node.sh | sudo bash
#   only wget on the box:      wget -qO- https://harlequinproject.org/install-harlequin-node.sh | bash
#   (a bare Debian ships with NEITHER curl nor sudo: apt update && apt install -y curl, as root)
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
# account (your mask in the society) is created by YOU in the browser (/rito) or with
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
BIN_SHA_x86_64="fb50ba6a3b48c45e9d38f70c440ea3362eb724af308882da4c6479a8e5111ad3" # spec-3 rollout node (door upgrade; gate v2 SYNC-vs-TIP; marca puerta-spec3-2026-07-26)
BIN_URL_aarch64="$DIST_BASE/dist/harlequin-node-arm64"
BIN_SHA_aarch64="1b65c660043a9d9efded6314f77f46a3c4f6cec501cb6c2dd310058fe65fa54e" # spec-3 rollout aarch64 (cross-built from the same marca puerta-spec3-2026-07-26)

# Baked into the spec too; passed explicitly in portable mode for first-dial robustness.
# THREE DOORS, NOT ONE (2026-07-26). Until today the installer handed out a single address, so every
# newcomer entered through the same machine — and when that machine choked (as it did today: 15-20 s to
# serve 5 KB, one peer left, falling behind the chain) the door was the network. A newcomer whose first
# contact times out does not try again. These are tried in order; any one of them is enough to join.
#
# THE HOME NODE IS DELIBERATELY ABSENT AND MUST STAY ABSENT. It reaches the network outbound-only over
# WireGuard and its residential address is never published. Listing it here would print someone's home
# IP in a script that anyone can download — the project's second golden rule. If you are tempted to
# "complete the list" some day: that is the reason it is incomplete on purpose.
BOOTNODE="/ip4/95.133.166.93/tcp/30333/p2p/12D3KooWBLjMD2oEZvNVZXFSHdRRS62gbZsgfYcR6rkkjygJ2emR"
BOOTNODE2="/ip4/148.116.86.24/tcp/30333/p2p/12D3KooWBiZXWDXuXKzKw8f6Wpmo3Mx81oAgFu2VG1fsZihs3BHC"
BOOTNODE3="/ip4/151.145.42.146/tcp/30333/p2p/12D3KooWLinJp4ZZnrcGsqXprk7snpdC64KnnTPenN2CP356z37X"
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
# Re-pinned 2026-07-29 to a POST-APPLY finalized block: #79830 is past the runtime upgrade at #79501,
# so this pin alone proves the chain you joined is the one running spec 3. Cross-verified on three
# independent nodes before pinning (bootnode RPC, oracle04 RPC, ct103 finality log: "finalised #79830
# … committee 4, alpha 3/4").
CHECKPOINT_HEIGHT="79830"
CHECKPOINT_HASH="0xda9180c4276481a76c00125272029b715fe924c535a2aec4d6556100401deb44"
CHECKPOINT_CHAIN="Harlequin Launch (cold-start)"  # system_chain name (spec id: hlq_launch)
CHECKPOINT_PINNED_AT="2026-07-29"  # re-pin to a fresher finalized block before each public release
RPC_URL="http://127.0.0.1:9944"   # node RPC is local-only by default; the check runs on YOUR box
# ─────────────────────────────────────────────────────────────────────────────

die() { echo "  ✗ $*" >&2; exit 1; }
ok()  { echo "  ✓ $*"; }
info(){ echo "    $*"; }

echo
echo "  ──────────────── ♦ ♥ ♠ ♣ ────────────────"
echo
echo "         H  A  R  L  E  Q  U  I  N"
echo "            node installer · mainnet"
echo
echo "  ─────────────────────────────────────────"
echo
echo "  A node is NOT your mask. Running a node sustains the network;"
echo "  it does not create your identity in the society. Your mask is"
echo "  made by you alone, in your browser, at:"
echo "      https://harlequinproject.org/rito   (EN)"
echo "      https://proyectoharlequin.org/rito  (ES)"
echo "  This script never asks for it and never touches it."
echo

# 1. CPU architecture → binary + sha
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64)  BIN_URL="$BIN_URL_x86_64";  BIN_SHA256="$BIN_SHA_x86_64" ;;
  aarch64) BIN_URL="$BIN_URL_aarch64"; BIN_SHA256="$BIN_SHA_aarch64" ;;
  *) die "unsupported CPU '$ARCH' (supported: x86_64, aarch64)." ;;
esac
ok "cpu: $ARCH"

# 1b. Memory and disk preflight. MEASURED on 2026-07-30, same box, same path, only the RAM changed:
#     1 GB → 0.0 blocks/s, stuck at block 47,152, ~10 days to finish, and NO error: the node crawls in
#     silence and the person concludes the project is broken. 2 GB → 840-1,066 blocks/s, whole chain in
#     ~2 minutes — BUT the first sync peaked at 2,031 MB and only survived because that box had swap;
#     at rest it settles around 1,997 MB + 260 MB swap. A 2 GB VPS with no swap (most cheap ones) does
#     not slow down: the kernel KILLS the node. And the overnight watch closed the case: 2 GB WITH
#     512 MB of swap synced fine and then ate 509 of those 512 MB in ninety minutes, stalling on
#     ~1,800 allocations. So the honest floor is 4 GB — on a machine with room the node settles near
#     230 MB and never touches swap.
MEM_MB=0; SWAP_MB=0
if [ -r /proc/meminfo ]; then
  MEM_MB="$(awk '/^MemTotal:/{printf "%d", $2/1024}' /proc/meminfo 2>/dev/null || echo 0)"
  SWAP_MB="$(awk '/^SwapTotal:/{printf "%d", $2/1024}' /proc/meminfo 2>/dev/null || echo 0)"
fi
if [ "${MEM_MB:-0}" -gt 0 ]; then
  if [ "$MEM_MB" -lt 1800 ]; then
    echo
    echo "  ⚠ THIS MACHINE HAS ${MEM_MB} MB OF MEMORY. The node needs 4 GB."
    echo "    With 1 GB it does not merely go slow: it stalls part-way and NEVER finishes, printing"
    echo "    no error at all (measured: stuck at block 47,152, ~10 days remaining)."
    echo "    Add memory or swap and run this again. Continuing anyway in 15s — Ctrl+C to stop."
    echo
    sleep 15
  elif [ "$MEM_MB" -lt 3800 ]; then
    # Overnight watch, 2026-07-30: a 2 GB box with 512 MB of swap DID sync and kept up with the tip —
    # and then quietly ate 509 of its 512 MB of swap in ninety minutes, with ~1,000 failed page-outs
    # and ~1,800 allocation stalls. It had not died yet; it was next in line. A node that behaves all
    # afternoon and starts failing at 3 a.m. is the worst failure there is: nobody can reproduce it.
    echo
    echo "  ⚠ ${MEM_MB} MB of memory (swap: ${SWAP_MB:-0} MB). This node wants 4 GB."
    echo "    Under that it can look fine for hours and then choke: measured on 2 GB + 512 MB swap,"
    echo "    it filled the swap in 90 minutes and started stalling. If you go ahead anyway, give it"
    echo "    at least 2 GB of swap:"
    echo "        fallocate -l 2G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile"
    echo "    Continuing anyway in 15s — Ctrl+C to stop."
    echo
    sleep 15
  else
    ok "memory: ${MEM_MB} MB (first sync peaks at ~2,031 MB; steady state settles near 230 MB)"
  fi
fi
DISK_MB="$(df -Pm "${HOME:-/}" 2>/dev/null | awk 'NR==2{print $4}' || echo 0)"
if [ "${DISK_MB:-0}" -gt 0 ] && [ "$DISK_MB" -lt 3000 ]; then
  echo "  ⚠ only ${DISK_MB} MB free on this filesystem. The chain is ~600 MB today and grows every day."
fi

# 2. install mode: systemd service vs portable foreground
MODE="portable"
if [ -z "${HLQ_PORTABLE:-}" ] && [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then
  MODE="service"
fi
ok "mode: $MODE"

# Downloader: curl if present, wget if not. A freshly installed Debian ships with NEITHER, and the
# published one-liner starts with curl — measured on a bare container: the stranger's very first
# command answers "curl: command not found" and explains nothing. Bootstrap what we can, and when we
# cannot, say what is missing and the exact command that fixes it.
command -v curl >/dev/null 2>&1 || {
  # bootstrap curl where we can (fresh proot Debian has apt and we are root there). SAY IT OUT LOUD:
  # installing packages in silence on the machine of someone who has just arrived and does not trust
  # us yet is the wrong first impression. If wget is here, we do not touch their system at all.
  if command -v wget >/dev/null 2>&1; then
    info "no curl here — using wget instead (nothing will be installed on your system)."
  elif command -v apt-get >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
    info "no curl and no wget here — installing 'curl' and 'ca-certificates' with apt (say no by pressing Ctrl+C now)."
    sleep 3
    apt-get update -y >/dev/null 2>&1 || true
    DEBIAN_FRONTEND=noninteractive apt-get install -y curl ca-certificates >/dev/null 2>&1 || true
  fi
}
if command -v curl >/dev/null 2>&1; then
  HAVE_DL="curl"
elif command -v wget >/dev/null 2>&1; then
  HAVE_DL="wget"
else
  die "no downloader found: this needs curl (or wget). On Debian/Ubuntu as root:  apt update && apt install -y curl ca-certificates"
fi
ok "downloader: $HAVE_DL"
command -v sha256sum >/dev/null 2>&1 || \
  die "sha256sum is missing — it is what proves the binary is the right one, so we do not go on without it. On Debian/Ubuntu as root:  apt update && apt install -y coreutils"

if [ "$MODE" = "service" ]; then
  [ "$(id -u)" -eq 0 ] || die "service install needs root (pipe to 'sudo bash'). Or force HLQ_PORTABLE=1."
  PREFIX="/opt/harlequin"
else
  PREFIX="${HOME:-/root}/harlequin"
  # Runtime libs the binary links against (readelf NEEDED: libstdc++.so.6, libgcc_s.so.1, libm/libc)
  # PLUS the dynamic loader (ld-linux-aarch64) and zlib. A fresh Termux `proot-distro install debian`
  # ships pared to the bone — without these the binary fails to exec with "required file not found"
  # (missing loader) or "error while loading shared libraries". Install them BEFORE running the binary.
  # `apt-get update` MUST succeed first (a bare rootfs has empty package lists); if it can't, say so
  # instead of silently proceeding to a binary that then won't start.
  if command -v apt-get >/dev/null 2>&1 && [ "$(id -u)" -eq 0 ]; then
    ok "installing runtime libraries (a fresh proot Debian ships without them)…"
    apt-get update -y >/dev/null 2>&1 || apt-get update -y >/dev/null 2>&1 \
      || die "apt-get update failed — the node's libraries can't be installed. Check the network/VPN inside the proot and re-run."
    DEBIAN_FRONTEND=noninteractive apt-get install -y \
        libc6 libstdc++6 libgcc-s1 zlib1g ca-certificates >/dev/null 2>&1 \
      || die "could not install runtime libraries (apt). Try: apt-get install -y libc6 libstdc++6 libgcc-s1 zlib1g"
  fi
fi

NODE_NAME="${HLQ_NODE_NAME:-hlq-$(printf '%04x' $((RANDOM)))}"   # pseudonymous by default (no identity link)
NODE_NAME="$(printf '%s' "$NODE_NAME" | tr -cd 'A-Za-z0-9-' | cut -c1-32)"; [ -n "$NODE_NAME" ] || NODE_NAME="hlq-node"

# 3. download to a temp dir that is always cleaned.
#    HTTPS-only, TLS>=1.2, no redirects: the URLs are fixed and same-origin, so a redirect would mean
#    someone is steering us elsewhere. (sha256 below is the real backstop.)
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
dl() { # <url> <dest> — same guarantees on either tool: HTTPS-only, TLS>=1.2, no redirect following.
  if [ "$HAVE_DL" = "curl" ]; then
    curl -fsS --proto '=https' --tlsv1.2 --connect-timeout 15 --max-time 900 "$1" -o "$2"
  else
    wget -q --https-only --secure-protocol=TLSv1_2 --max-redirect=0 --timeout=15 -O "$2" "$1"
  fi
}
[ "${BIN_URL#https://}"  != "$BIN_URL"  ] || die "binary URL is not https."
[ "${SPEC_URL#https://}" != "$SPEC_URL" ] || die "spec URL is not https."
ok "downloading node binary ($ARCH)…"; dl "$BIN_URL"  "$TMP/harlequin-node"   || die "download failed: $BIN_URL"
ok "downloading launch chain spec…";   dl "$SPEC_URL" "$TMP/mainnet-raw.json" || die "download failed: $SPEC_URL"

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
rpc() { # curl if present, wget otherwise — a bare server may have only one of the two.
  if command -v curl >/dev/null 2>&1; then
    curl -fsS -m 10 -H 'Content-Type: application/json' -d "$1" "$RPC_URL" 2>/dev/null || true
  else
    wget -q -O- --timeout=10 --header='Content-Type: application/json' --post-data="$1" "$RPC_URL" 2>/dev/null || true
  fi
}
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
# Honesty of the waiting line (measured on a 1 GB box, 2026-07-30): a node short on memory does not
# die — it CRAWLS, and while it thrashes its RPC stops answering. The old line then printed
# "best block 0", which reads like "it started over" and sends people away thinking it is broken.
# So: say when the node is not answering, say when it IS answering but not advancing, and say the
# rate when it moves — a person who can see progress waits; one staring at a frozen 0 does not.
GOT=""; PREV=""; SAME=0
while :; do
  GOT="$(res "$(rpc "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"chain_getBlockHash\",\"params\":[$CHECKPOINT_HEIGHT]}")")"
  [ -n "$GOT" ] && break
  HDR="$(rpc '{"id":1,"jsonrpc":"2.0","method":"chain_getHeader","params":[]}')"
  BESTHEX="$(printf '%s' "$HDR" | grep -o '"number":"0x[0-9a-fA-F]*"' | head -1 | cut -d'"' -f4 || true)"
  if [ -z "$BESTHEX" ]; then
    echo "    the node is not answering right now (it may be busy syncing, or short on memory) — waiting."
  else
    BEST=$(( BESTHEX ))
    if [ "$BEST" = "${PREV:-}" ]; then
      SAME=$((SAME+1))
      if [ "$SAME" -ge 4 ]; then
        echo "    stuck at block $BEST / $CHECKPOINT_HEIGHT for ~$((SAME*15))s — this usually means too little"
        echo "    memory (see the requirements on https://harlequinproject.org/join). The node is alive; it is crawling."
      else
        echo "    syncing… block $BEST / $CHECKPOINT_HEIGHT — no progress this round, waiting."
      fi
    else
      [ -n "${PREV:-}" ] && echo "    syncing… block $BEST / $CHECKPOINT_HEIGHT (+$((BEST-PREV)) in 15s)." \
                        || echo "    syncing… block $BEST / $CHECKPOINT_HEIGHT."
      SAME=0
    fi
    PREV="$BEST"
  fi
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
# --pool-type single-state: root fix for the intermittent "Essential task txpool-background failed"
# (no backticks in this heredoc: it is unquoted, so the shell would try to RUN whatever they wrap —
#  measured on a clean box as 'bash: line 252: Essential: command not found' right before start)
# crash of the fork-aware pool. It bit twice on 2026-07-25, BOTH times while a node was catching up at
# ~1000 blocks/s — exactly what every newcomer does. A node that dies mid-sync is a person who does not
# try again, so the flag ships with the installer rather than waiting for a deployment round.
ExecStart=$PREFIX/harlequin-node \\
  --base-path $PREFIX/data \\
  --chain $PREFIX/mainnet-raw.json \\
  --node-key-file $PREFIX/node-key \\
  --name "$NODE_NAME" \\
  --port 30333 \\
  --consensus woven-trust-12000 \\
  --network-backend libp2p \\
  --no-mdns \\
  --bootnodes "${BOOTNODE}" "${BOOTNODE2}" "${BOOTNODE3}" \\
  --state-pruning archive \\
  --blocks-pruning archive \\
  --pool-type single-state
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
  # v4.1 UX: the node runs DETACHED from the terminal (nohup + logfile + pidfile). Closing the
  # terminal or locking the screen no longer kills it. The weak-subjectivity check (M1) still stops
  # it FAIL-CLOSED on mismatch before the script hands control back.
  cat > "$PREFIX/run-node.sh" <<RUN
#!/usr/bin/env bash
set -euo pipefail
cd "\$(dirname "\$0")"
# singleton: refuse a second copy (stale pidfile is fine — we check the process)
if [ -f node.pid ] && kill -0 "\$(cat node.pid)" 2>/dev/null; then
  echo "  ✓ node already running (pid \$(cat node.pid)). Status: ./node-status.sh" >&2
  exit 0
fi
nohup ./harlequin-node \\
  --base-path ./data \\
  --chain ./mainnet-raw.json \\
  --name "${NODE_NAME}" \\
  --port 30333 \\
  --consensus woven-trust-12000 \\
  --network-backend libp2p \\
  --no-mdns \\
  --wasmtime-instantiation-strategy recreate-instance-copy-on-write \\
  --bootnodes "${BOOTNODE}" "${BOOTNODE2}" "${BOOTNODE3}" \\
  --state-pruning archive \\
  --blocks-pruning archive \\
  --pool-type single-state > node.log 2>&1 &
NODE_PID=\$!
echo "\$NODE_PID" > node.pid
# exit 1 = checkpoint MISMATCH → kill the node (fail-closed). exit 2 = local RPC unreachable
# (transient/slow hardware, e.g. a tablet) → keep the node running, demand a manual re-check.
VC_RC=0; ./verify-checkpoint.sh || VC_RC=\$?
if [ "\$VC_RC" -eq 1 ]; then
  kill "\$NODE_PID" 2>/dev/null || true
  rm -f node.pid
  echo "  ✗ node stopped: checkpoint MISMATCH (fail-closed)." >&2
  echo "    Re-check the pinned values against >=2 independent sources, then run ./run-node.sh again." >&2
  exit 1
elif [ "\$VC_RC" -ne 0 ]; then
  echo "  ⚠ could not reach the node's local RPC — checkpoint NOT verified yet." >&2
  echo "    The node keeps running, but do NOT trust it until this passes:  ./verify-checkpoint.sh" >&2
fi
echo "  ✓ node running detached (pid \$NODE_PID). It survives closing this terminal."
echo "    peek at it:   ./node-status.sh      follow live:  tail -f node.log      stop: ./stop-node.sh"
RUN
  chmod +x "$PREFIX/run-node.sh"

  # One-line health view: process state + the node's own last sync line.
  cat > "$PREFIX/node-status.sh" <<'ST'
#!/usr/bin/env bash
cd "$(dirname "$0")"
if [ -f node.pid ] && kill -0 "$(cat node.pid)" 2>/dev/null; then
  echo "  ✓ node RUNNING (pid $(cat node.pid))"
else
  echo "  ✗ node NOT running. Start it: ./run-node.sh"
fi
grep -oE "(Syncing|Idle).*" node.log 2>/dev/null | tail -1
grep -oE "finalized #[0-9]+" node.log 2>/dev/null | tail -1
ST
  chmod +x "$PREFIX/node-status.sh"

  cat > "$PREFIX/stop-node.sh" <<'SP'
#!/usr/bin/env bash
cd "$(dirname "$0")"
if [ -f node.pid ] && kill -0 "$(cat node.pid)" 2>/dev/null; then
  kill "$(cat node.pid)" && rm -f node.pid && echo "  ✓ node stopped."
else
  rm -f node.pid; echo "  · node was not running."
fi
SP
  chmod +x "$PREFIX/stop-node.sh"

  # Foreground variant for OUTER supervisors. Android/proot gotcha (field-tested 2026-07-25):
  # proot is ptrace-based, so a nohup INSIDE the proot login dies when the login exits — detaching
  # must happen OUTSIDE. From Termux (not inside debian) run:
  #   setsid nohup proot-distro login debian -- bash ~/harlequin/run-node-fg.sh > ~/node.log 2>&1 &
  cat > "$PREFIX/run-node-fg.sh" <<RUNFG
#!/usr/bin/env bash
set -euo pipefail
cd "\$(dirname "\$0")"
exec ./harlequin-node \\
  --base-path ./data \\
  --chain ./mainnet-raw.json \\
  --name "${NODE_NAME}" \\
  --port 30333 \\
  --consensus woven-trust-12000 \\
  --network-backend libp2p \\
  --no-mdns \\
  --wasmtime-instantiation-strategy recreate-instance-copy-on-write \\
  --bootnodes "${BOOTNODE}" "${BOOTNODE2}" "${BOOTNODE3}" \\
  --state-pruning archive \\
  --blocks-pruning archive \\
  --pool-type single-state
RUNFG
  chmod +x "$PREFIX/run-node-fg.sh"

  echo "  ────────────────────────────────────────────"
  ok "All set. Starting your follower node '$NODE_NAME' (detached — it survives closing the terminal)."
  info "· Android/Termux (proot): to survive closing the APP, start it from Termux itself instead:"
  info "    setsid nohup proot-distro login debian -- bash ~/harlequin/run-node-fg.sh > ~/node.log 2>&1 &"
  info "· First start runs the weak-subjectivity check: your node must contain the pinned block or it stops."
  info "· Peek any time: $PREFIX/node-status.sh · live view: tail -f $PREFIX/node.log"
  info "· Your MASK (seed phrase) is created in the BROWSER: $DIST_BASE/rito"
  info "· Stop: $PREFIX/stop-node.sh · start again: $PREFIX/run-node.sh"
  info "· Keep your VPN on — a node announces its IP to peers."
  echo "  Your node, your keys, no master."
  echo "  ────────────────────────────────────────────"
  echo
  exec "$PREFIX/run-node.sh"
fi
