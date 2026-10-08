#!/usr/bin/env python3
"""Harlequin same-origin mediator v1 (#613/#696/#699) — stdlib only, no third-party deps.

Relays public chain state and signed bytes between the browser and the LOCAL node RPC; it can
censor at worst, never forge (every detail is client-verified against the on-chain hash and every
tx is signed on-device). Contract: chain/CONTRACT-MARKET-API-613.md (joint round CLOSED 2026-07-20).

Endpoints (served on loopback; Caddy fronts them same-origin under /api/*):
  GET  /api/market/offers?cursor=<id>&limit=<n>   offers newest-first + next_cursor + best_block
  GET  /api/tx/meta?ss58=<addr>|account=<0x hex>  nonce + genesis + versions (public chain data)
  GET  /api/consensus/vote_keys                   who has bound which node vote key (public, #1468)
  POST /api/tx        {"xt":"0x…"}                author_submitExtrinsic verbatim relay
  GET  /api/ipfs/<cid>                            detail from OUR store only (no third-party fetch since 27-sep)
  GET  /api/standing?account=<0x-hex pubkey>      reputation: aggregate + the four suits (docs/API-STANDING.md)
  GET  /api/balance?account=<0x-hex pubkey>       HLQ and SOV balances, base units as decimal strings (TokensApi)
  GET  /api/capabilities                         pallet/call indices read from the runtime metadata (deals on/off)
  GET  /api/market/deals?account=<0x hex>          the account's deals (proposed/closed/expired), newest first
  GET  /api/pacts/sealed?key=<0x hex32>            block at which a sealed pact key (mode B) was recorded, or null
  GET  /api/pacts/pact?id=<n>                      a mode-A pact (parties, who signed, completed) + next id
  GET  /api/transfers?account=<0x hex>&limit&offset  the account's Tokens transfers/fees, newest first (indexed cache)
  GET  /api/resolve?handle=hlq-<26 base32>         mask handle → account (only accounts the chain already knows)
  GET  /api/forum/posts                           forum index (author, body hash/cid, parent, block), newest first
  GET  /api/justice/marks?accounts=<hex,hex,…≤64> public guilty verdicts per account (moderation-B: label, never hide)
  POST /api/detail   <utf-8 text, ≤4 KiB>         store an offer's detail by its sha256 → {detail_hash, detail_cid}

The operator's conditions (review gate before exposure): same-origin under the web · rate limits from the
contract · ZERO new secrets on this box · the web host off the public path · no identity logs (sliding
window only, nothing retained).
"""

import json
import os
import re
import subprocess
import threading
import time
import urllib.request
from collections import deque
from hashlib import blake2b
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# Env overrides exist ONLY for a local lab (a dev node + this mediator); production leaves them unset.
RPC = os.environ.get("HLQ_MEDIATOR_RPC", "http://127.0.0.1:9944")   # node RPC (--rpc-methods safe)
LISTEN = ("127.0.0.1", int(os.environ.get("HLQ_MEDIATOR_PORT", "8137")))  # Caddy → /api/* (same-origin)
# NO upstream fetches (27-sep-2026). This list used to hold public IPFS gateways; details are now served from our own
# store only — fewer third parties learning which Harlequin CIDs are being read. A CID we never stored answers
# "detail unavailable". Re-enable with HLQ_MEDIATOR_IPFS_UPSTREAMS if ever needed.
IPFS_UPSTREAMS = [u for u in os.environ.get("HLQ_MEDIATOR_IPFS_UPSTREAMS", "").split(",") if u]
MAX_TX_BYTES = 16 * 1024
MAX_IPFS_BYTES = 256 * 1024
OFFER_CACHE_S = 6                       # ≤1 block of staleness, tagged by best_block anyway

# ── Villa login (#637): challenge/response, no secret on the server ──────────────────────────────
# The genesis hash is a KNOWN CONSTANT here (unlike the runtime, which prunes block 0): it anchors a
# login to THIS chain, so a signature made against another Harlequin instance never authenticates.
GENESIS_HASH = bytes.fromhex("ecdaa7d302af9f0e8a071c780460bf55faa164f8a9fb30df64b36aeab9c13aef")   # rehearsal chain 2 (8 Oct 2026, launch candidate 6); rehearsal 1 was 0x95cff149, the first chain 0xa1ab5e0b
LOGIN_LABEL = b"hlq-villa-login-v1"     # 18 bytes; folded into the signed message for domain separation
LOGIN_TTL_S = 120
# ABSOLUTE, fixed — never taken from the request. Env override exists ONLY for local testing; prod
# leaves it unset and uses the fixed deploy path.
SIGVERIFY_BIN = os.environ.get("HLQ_SIGVERIFY_BIN", "/opt/harlequin/hlq-sigverify")
_challenges: dict = {}                  # nonce_hex -> (msg_hex, expiry_monotonic, mask_ss58); RAM only
_challenges_lock = threading.Lock()

# Sliding-window rate limits (contract): path-class -> (max, per_seconds, burst_extra)
LIMITS = {"tx": (6, 60, 3), "txmeta": (30, 60, 5), "market": (60, 60, 0), "ipfs": (30, 60, 0), "login": (12, 60, 4), "standing": (30, 60, 0), "resolve": (10, 60, 0),
          "detail": (10, 3600, 2), "forum": (60, 60, 0), "justice": (60, 60, 0)}
# 2026-09-27 (web review): market, forum and justice reads used to share ONE "market" bucket, so a page
# that polls all three every 15 s spent it three times as fast. Each read has its own now.
# Every Tor visitor arrives from the loopback (the hidden service connects to Caddy locally), so on the
# .onion ALL visitors share one bucket. Read-only classes get a wider bucket there; writes stay strict.
LOOPBACK = {"127.0.0.1", "::1"}
ONION_READ_FACTOR = 8
READ_CLASSES = {"market", "forum", "justice", "ipfs", "standing", "txmeta", "resolve"}
_windows: dict = {}
_windows_lock = threading.Lock()


# LAB ONLY (8-oct-2026): the devnet bots all leave from one address, so a per-IP window built for strangers
# throttles the whole society. Production leaves it unset (1). Never a reason to raise it in production.
LAB_RATE_MULT = max(1, int(os.environ.get("HLQ_LAB_RATE_MULT", "1")))


def allow(ip: str, klass: str) -> bool:
    """Sliding window, in-memory only. Entries expire out of the deque — nothing is retained."""
    maxn, per, burst = LIMITS[klass]
    maxn *= LAB_RATE_MULT               # 1 in production; a lab with N bots behind ONE address sets it higher
    if ip in LOOPBACK and klass in READ_CLASSES:
        maxn *= ONION_READ_FACTOR
    now = time.monotonic()
    with _windows_lock:
        q = _windows.setdefault((ip, klass), deque())
        while q and now - q[0] > per:
            q.popleft()
        if len(q) >= maxn + burst:
            return False
        q.append(now)
        # opportunistic prune of dead peers so the map never grows unbounded
        if len(_windows) > 4096:
            for k in [k for k, v in _windows.items() if not v or now - v[-1] > per]:
                _windows.pop(k, None)
        return True


_B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58decode(s: str):
    n = 0
    for c in s:
        i = _B58.find(c)
        if i < 0:
            return None
        n = n * 58 + i
    body = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    pad = len(s) - len(s.lstrip("1"))  # leading '1's are leading zero bytes
    return b"\x00" * pad + body


HLQ_SS58_PREFIX = 1728  # Harlequin's registered SS58 network prefix (2-byte)


def ss58_to_pubkey(addr: str):
    """Decode an SS58 address to its 32-byte public key, verifying BOTH the blake2b checksum AND that
    the network prefix is Harlequin's 1728 — an address of another network (or a bad checksum) is
    rejected. Returns None on any malformation; the caller treats None as invalid (fail-closed)."""
    data = b58decode(addr)
    if not data or len(data) < 36:  # 2 prefix + 32 pubkey + 2 checksum
        return None
    if data[0] < 64:
        return None  # Harlequin uses a 2-byte prefix (1728); a 1-byte prefix cannot be it
    first, second = data[0], data[1]
    ident = ((first & 0x3F) << 2) | (second >> 6) | ((second & 0x3F) << 8)
    if ident != HLQ_SS58_PREFIX:
        return None
    body, checksum = data[:-2], data[-2:]
    pubkey = body[2:]
    if len(pubkey) != 32:
        return None
    digest = blake2b(b"SS58PRE" + body, digest_size=64).digest()
    if digest[:2] != checksum:
        return None
    return pubkey


def ss58_generic(pubkey: bytes) -> str:
    """The same key as an SS58 address with the GENERIC prefix 42. Why: the node never calls
    `set_default_ss58_version(1728)` at start-up, so its RPC rejects every Harlequin address
    ("Unknown SS58 address format `1728`", measured on the live node 2026-09-23) — /api/tx/meta had never
    answered for a real mask. Prefix 42 is always understood and names the SAME AccountId32. The node
    fix is one line at start-up (backlog, 27-nov); until then the mediator speaks the node's dialect."""
    body = bytes([42]) + pubkey
    data = body + blake2b(b"SS58PRE" + body, digest_size=64).digest()[:2]
    n = int.from_bytes(data, "big")
    out = ""
    while n:
        n, r = divmod(n, 58)
        out = _B58[r] + out
    return "1" * (len(data) - len(data.lstrip(b"\x00"))) + out


def sig_is_valid(pubkey: bytes, msg: bytes, sig: bytes) -> bool:
    """FAIL-CLOSED sr25519 check via the first-party hlq-sigverify binary. Valid ONLY on a clean
    exit code 0; a non-zero exit, a timeout, a missing binary, or any exception all mean 'invalid'."""
    if len(pubkey) != 32 or len(sig) != 64:
        return False
    payload = f"{pubkey.hex()}\n{msg.hex()}\n{sig.hex()}".encode()
    try:
        proc = subprocess.run(
            [SIGVERIFY_BIN], input=payload, capture_output=True, timeout=2
        )
        return proc.returncode == 0
    except Exception:
        return False


def rpc(method: str, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(RPC, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=15) as r:
        out = json.loads(r.read())
    if "error" in out:
        raise RuntimeError(out["error"].get("message", "rpc error"))
    return out["result"]


# ── Reputation (lever #1: "see your standing", 2026-09-23) ──────────────────────────────────────────
# Public chain data (Art. X): the runtime's ReputationApi, read at best. Fixed-point 1e9 → [0,1]. The
# four suits in canonical Suit::ALL order [♦ commerce, ♣ technical, ♠ judicial, ♥ governance], keyed
# with the panel's data-suit names. `committee_eligible` = the account is in the set the finality
# committee is DRAWN from (consensus_reputation > 0) — eligible for the draw, not a seat.
FP = 1_000_000_000
SUIT_KEYS = ("commerce", "contribution", "judgement", "governance")
HEX32_RE = re.compile(r"^(0x)?[0-9a-fA-F]{64}$")


def _i128s(h: str, n: int):
    b = bytes.fromhex(h[2:] if h.startswith("0x") else h)
    if len(b) != 16 * n:
        raise RuntimeError("unexpected runtime answer length")
    return [int.from_bytes(b[16 * k:16 * k + 16], "little", signed=True) for k in range(n)]


def standing_payload(pub_hex: str) -> dict:
    acc = "0x" + (pub_hex[2:] if pub_hex.startswith("0x") else pub_hex).lower()
    standing = _i128s(rpc("state_call", ["ReputationApi_standing_of", acc]), 1)[0]
    suits = _i128s(rpc("state_call", ["ReputationApi_suits_of", acc]), 4)
    unit = lambda v: max(0.0, min(1.0, v / FP))
    out = {"standing": f"{unit(standing):.3f}", "committee_eligible": standing > 0}
    if any(suits):
        out["suits"] = {k: round(unit(v), 4) for k, v in zip(SUIT_KEYS, suits)}
    return out


def twox128(data: bytes) -> bytes:
    # xxhash64 seeded 0 and 1, concatenated — substrate's twox_128. stdlib-only implementation.
    def xx64(seed: int, b: bytes) -> int:
        P1, P2, P3, P4, P5 = (
            11400714785074694791, 14029467366897019727, 1609587929392839161,
            9650029242287828579, 2870177450012600261)
        M = (1 << 64) - 1

        def rotl(x, r):
            return ((x << r) | (x >> (64 - r))) & M

        n = len(b)
        if n >= 32:
            v1, v2 = (seed + P1 + P2) & M, (seed + P2) & M
            v3, v4 = seed & M, (seed - P1) & M
            i = 0
            while i <= n - 32:
                for vi, off in ((0, 0), (1, 8), (2, 16), (3, 24)):
                    v = [v1, v2, v3, v4][vi]
                    lane = int.from_bytes(b[i + off:i + off + 8], "little")
                    v = (v + lane * P2) & M
                    v = rotl(v, 31)
                    v = (v * P1) & M
                    if vi == 0: v1 = v
                    elif vi == 1: v2 = v
                    elif vi == 2: v3 = v
                    else: v4 = v
                i += 32
            h = (rotl(v1, 1) + rotl(v2, 7) + rotl(v3, 12) + rotl(v4, 18)) & M
            for v in (v1, v2, v3, v4):
                v = (v * P2) & M
                v = rotl(v, 31)
                v = (v * P1) & M
                h = ((h ^ v) * P1 + P4) & M
        else:
            h = (seed + P5) & M
            i = 0
        h = (h + n) & M
        while i <= n - 8:
            lane = int.from_bytes(b[i:i + 8], "little")
            k = (lane * P2) & M
            k = rotl(k, 31)
            k = (k * P1) & M
            h = ((rotl(h ^ k, 27) * P1) + P4) & M
            i += 8
        if i <= n - 4:
            lane4 = (int.from_bytes(b[i:i + 4], "little") * P1) & M
            h = ((rotl(h ^ lane4, 23) * P2) + P3) & M
            i += 4
        while i < n:
            h = (rotl(h ^ ((b[i] * P5) & M), 11) * P1) & M
            i += 1
        h ^= h >> 33
        h = (h * P2) & M
        h ^= h >> 29
        h = (h * P3) & M
        h ^= h >> 32
        return h

    return xx64(0, data).to_bytes(8, "little") + xx64(1, data).to_bytes(8, "little")


def blake2_128_concat(data: bytes) -> bytes:
    return blake2b(data, digest_size=16).digest() + data


OFFERS_PREFIX = (twox128(b"Market") + twox128(b"Offers")).hex()
POSTS_PREFIX = (twox128(b"Forum") + twox128(b"Posts")).hex()
MARKS_PREFIX = twox128(b"Justice") + twox128(b"GuiltyMarks")

B32 = "abcdefghijklmnopqrstuvwxyz234567"


def cid_v1(digest32: bytes) -> str:
    # CIDv1 raw+sha256, multibase base32-lower no padding: 'b' + b32(0x01 0x55 0x12 0x20 + digest)
    raw = bytes([0x01, 0x55, 0x12, 0x20]) + digest32
    bits = 0
    acc = 0
    out = []
    for byte in raw:
        acc = (acc << 8) | byte
        bits += 8
        while bits >= 5:
            bits -= 5
            out.append(B32[(acc >> bits) & 31])
    if bits:
        out.append(B32[(acc << (5 - bits)) & 31])
    return "b" + "".join(out)


def decode_compact(b: bytes, i: int):
    f = b[i] & 0b11
    if f == 0:
        return b[i] >> 2, i + 1
    if f == 1:
        return int.from_bytes(b[i:i + 2], "little") >> 2, i + 2
    if f == 2:
        return int.from_bytes(b[i:i + 4], "little") >> 2, i + 4
    n = (b[i] >> 2) + 4
    return int.from_bytes(b[i + 1:i + 1 + n], "little"), i + 1 + n


def _stored(digest: bytes) -> bool:
    """Is the content behind this sha256 digest in our own detail store (what /api/ipfs/<cid> serves)?"""
    return os.path.isfile(os.path.join(DETAIL_STORE, digest.hex()))


# ── Handle → account (7-oct-2026, the wallet's «send to a mask») ─────────────────────────────────────
# A handle is one-way: hlq- + base32(sha256(pubkey)[:16]) (wallet-core `address_of`). The only way back is to
# hash every account the chain knows and look it up. Public data in, public data out; an account that has never
# touched the chain (a fresh mask that only did the rite) is not findable — the wallet then asks for the address.
SYSTEM_ACCOUNT_PREFIX = (twox128(b"System") + twox128(b"Account")).hex()
TOKENS_ACCOUNTS_PREFIX = (twox128(b"Tokens") + twox128(b"Accounts")).hex()
HANDLE_RE = re.compile(r"^hlq-[a-z2-7]{26}$")
HANDLE_INDEX_S = 30
_handles = {"at": 0.0, "map": {}}
_handles_lock = threading.Lock()
_handles_build = threading.Lock()


def handle_of(pubkey: bytes) -> str:
    import base64, hashlib
    return "hlq-" + base64.b32encode(hashlib.sha256(pubkey).digest()[:16]).decode().rstrip("=").lower()


def _build_handle_index() -> dict:
    m = {}
    for prefix in (SYSTEM_ACCOUNT_PREFIX, TOKENS_ACCOUNTS_PREFIX):
        start = None
        while True:   # full paging: one page would silently drop accounts past the first thousand
            page = rpc("state_getKeysPaged", ["0x" + prefix, 1000, start])
            for k in page:
                pub = bytes.fromhex(k[2:])[-32:]   # Blake2_128Concat(AccountId32): the id is the tail
                m[handle_of(pub)] = pub
            if len(page) < 1000:
                break
            start = page[-1]
    return m


def resolve_handle(handle: str):
    """Review 7-oct: the scan used to run WITH the lock held, so every caller queued behind one chain-wide scan.
    Now one caller at a time rebuilds (non-blocking try), the rest answer from the last index, and the lock only
    guards the swap. A first-ever call with no index yet waits for the build in progress."""
    stale = time.monotonic() - _handles["at"] >= HANDLE_INDEX_S
    if stale and _handles_build.acquire(blocking=not _handles["map"]):
        try:
            if time.monotonic() - _handles["at"] >= HANDLE_INDEX_S:
                m = _build_handle_index()
                with _handles_lock:
                    _handles["map"], _handles["at"] = m, time.monotonic()
        finally:
            _handles_build.release()
    with _handles_lock:
        return _handles["map"].get(handle)


def decode_offer(v: bytes) -> dict:
    # Offer { offerer:[u8;32], detail:[u8;32], category:BoundedVec<u8>, at:u32, withdrawn:bool }
    i = 0
    offerer, i = v[i:i + 32], i + 32
    detail, i = v[i:i + 32], i + 32
    catlen, i = decode_compact(v, i)
    category, i = v[i:i + catlen], i + catlen
    at, i = int.from_bytes(v[i:i + 4], "little"), i + 4
    withdrawn = bool(v[i])
    return {
        "pubkey": offerer.hex(),
        "detail_hash": detail.hex(),
        "detail_cid": cid_v1(detail),
        # 7-oct-2026: whether WE hold the detail. The page then does not ask for a body nobody can serve (each
        # visitor paid two failed requests per missing body, red in the console, for the rehearsal's lab entries).
        "detail_stored": _stored(detail),
        "category": category.decode("utf-8", "replace"),
        "at_block": at,
        "withdrawn": withdrawn,
        "hidden": False,  # jury-driven hide arrives with the #614 justice hook
    }


_offers_cache = {"at": 0.0, "payload": None}
_offers_lock = threading.Lock()


def offers_payload() -> dict:
    with _offers_lock:
        if _offers_cache["payload"] and time.monotonic() - _offers_cache["at"] < OFFER_CACHE_S:
            return _offers_cache["payload"]
        head = rpc("chain_getHeader", [])
        best = int(head["number"], 16)
        offers = []
        for oid, raw in _read_map(OFFERS_PREFIX):   # full paging + batched values (see _read_map)
            o = decode_offer(raw)
            o["id"] = oid
            offers.append(o)
        offers.sort(key=lambda o: -o["id"])
        payload = {
            "offers": offers,
            "next_cursor": None,
            "best_block": best,
            "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }
        _offers_cache.update(at=time.monotonic(), payload=payload)
        return payload


# ── Offer details: a tiny content-addressed store (2026-09-23) ─────────────────────────────────────
# An offer on chain is only a 32-byte sha256; the text a buyer reads has to live somewhere. Nobody pins
# it on IPFS, so the board could only ever say "detail unavailable". This store keeps the text under its
# own hash — the SAME raw-sha256 CIDv1 the board already verifies, so the bytes can be pinned on the real
# IPFS network later without changing anything on chain or in the page. The server cannot alter a
# detail: the browser re-hashes every body against the on-chain hash and refuses a mismatch.
# Spam bounds: 4 KiB per text, a rate limit per IP, a total cap, and text that no offer on chain points
# to is deleted after an hour (a detail is uploaded moments before its `publish` is signed).
DETAIL_STORE = os.environ.get("HLQ_DETAIL_STORE", "/var/lib/hlq-mediator/details")
MAX_DETAIL_BYTES = 4096
DETAIL_ORPHAN_S = 3600
DETAIL_STORE_CAP = 64 * 1024 * 1024
_detail_lock = threading.Lock()


def cid_to_digest(cid: str):
    """Inverse of cid_v1: 'b' + base32(0x01 0x55 0x12 0x20 ‖ digest32) → digest32, or None."""
    acc = bits = 0
    raw = bytearray()
    for ch in cid[1:]:
        v = B32.find(ch)
        if v < 0:
            return None
        acc = (acc << 5) | v
        bits += 5
        if bits >= 8:
            bits -= 8
            raw.append((acc >> bits) & 0xFF)
    if len(raw) != 36 or bytes(raw[:4]) != bytes([0x01, 0x55, 0x12, 0x20]):
        return None
    return bytes(raw[4:])


def _detail_gc():
    """Drop stored texts older than DETAIL_ORPHAN_S that no offer on chain references."""
    try:
        names = os.listdir(DETAIL_STORE)
    except FileNotFoundError:
        return
    now = time.time()
    old = [n for n in names if now - os.path.getmtime(os.path.join(DETAIL_STORE, n)) > DETAIL_ORPHAN_S]
    if not old:
        return
    referenced = {o["detail_hash"] for o in offers_payload()["offers"]}
    referenced |= {p["body_hash"] for p in posts_payload()["posts"]}   # forum bodies share the store
    for n in old:
        if n not in referenced:
            try:
                os.remove(os.path.join(DETAIL_STORE, n))
            except OSError:
                pass


def store_detail(body: bytes) -> dict:
    from hashlib import sha256
    if not body or len(body) > MAX_DETAIL_BYTES:
        raise ValueError("detail must be 1..4096 bytes")
    text = body.decode("utf-8")          # raises on invalid UTF-8: the board renders it as text
    if not text.strip():
        raise ValueError("empty detail")
    digest = sha256(body).digest()
    name = digest.hex()
    with _detail_lock:
        os.makedirs(DETAIL_STORE, mode=0o755, exist_ok=True)
        path = os.path.join(DETAIL_STORE, name)
        if not os.path.exists(path):
            _detail_gc()
            used = sum(os.path.getsize(os.path.join(DETAIL_STORE, n)) for n in os.listdir(DETAIL_STORE))
            if used + len(body) > DETAIL_STORE_CAP:
                raise OverflowError("detail store full")
            tmp = path + ".tmp"
            with open(tmp, "wb") as f:
                f.write(body)
            os.replace(tmp, path)        # write-once, atomic: a half-written detail is never served
    return {"detail_hash": "0x" + name, "detail_cid": cid_v1(digest)}


def _read_map(prefix_hex: str):
    """Every (id, raw value) of a Blake2_128Concat(u64)-keyed map, with the full paging loop (a single
    page would silently drop entries past the first thousand — the operator's n2 on the offers feed)."""
    keys, start = [], None
    while True:
        page = rpc("state_getKeysPaged", ["0x" + prefix_hex, 1000, start])
        keys.extend(page)
        if len(page) < 1000:
            break
        start = page[-1]
    # Values in batches of 256 keys per call (state_queryStorageAt at the best block) instead of one
    # state_getStorage per entry: with thousands of posts the old loop was thousands of RPCs per refresh
    # (the reviewer's note, 2026-09-24). Same block for every batch, so the snapshot is consistent.
    at = rpc("chain_getBlockHash", []) if keys else None
    for i in range(0, len(keys), 256):
        chunk = keys[i:i + 256]
        res = rpc("state_queryStorageAt", [chunk, at])
        values = {k: v for k, v in (res[0]["changes"] if res else [])}
        for k in chunk:
            raw = values.get(k)
            if raw:
                yield int.from_bytes(bytes.fromhex(k[2:])[-8:], "little"), bytes.fromhex(raw[2:])


def decode_post(v: bytes) -> dict:
    # Post { author:[u8;32], body:[u8;32], parent:Option<u64>, at:u32 }
    author, body, i = v[0:32], v[32:64], 64
    parent = None
    if v[i] == 1:
        parent = int.from_bytes(v[i + 1:i + 9], "little")
        i += 9
    else:
        i += 1
    at = int.from_bytes(v[i:i + 4], "little")
    return {"pubkey": author.hex(), "body_hash": body.hex(), "body_cid": cid_v1(body), "body_stored": _stored(body),
            "parent": parent, "at_block": at}


def decode_u64_vec(v: bytes):
    """SCALE Vec<u64>: compact length, then little-endian u64s."""
    n, i = decode_compact(v, 0)
    return [int.from_bytes(v[i + 8 * k:i + 8 * k + 8], "little") for k in range(n)]


def guilty_marks(accounts):
    """account hex → case ids it was found GUILTY in (pallet-justice GuiltyMarks). Moderation-B (the
    maintainer, 2026-07-15): nothing is hidden — the verdict is published and each reader weighs it. One batched read
    at one block; an account with no mark is simply absent from the storage and comes back as []."""
    keys = {("0x" + (MARKS_PREFIX + blake2_128_concat(bytes.fromhex(a))).hex()): a for a in accounts}
    out = {a: [] for a in accounts}
    if not keys:
        return out
    res = rpc("state_queryStorageAt", [list(keys), rpc("chain_getBlockHash", [])])
    for k, v in (res[0]["changes"] if res else []):
        if v:
            out[keys[k]] = decode_u64_vec(bytes.fromhex(v[2:]))
    return out


_posts_cache = {"at": 0.0, "payload": None}
_vk_cache = {"at": 0.0, "hex": None}
_vk_lock = threading.Lock()
_posts_lock = threading.Lock()


def posts_payload() -> dict:
    with _posts_lock:
        if _posts_cache["payload"] and time.monotonic() - _posts_cache["at"] < OFFER_CACHE_S:
            return _posts_cache["payload"]
        best = int(rpc("chain_getHeader", [])["number"], 16)
        posts = []
        for pid, raw in _read_map(POSTS_PREFIX):
            p = decode_post(raw)
            p["id"] = pid
            posts.append(p)
        posts.sort(key=lambda p: -p["id"])
        payload = {"posts": posts, "best_block": best,
                   "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
        _posts_cache.update(at=time.monotonic(), payload=payload)
        return payload


SS58_RE = re.compile(r"^[1-9A-HJ-NP-Za-km-z]{40,60}$")
CID_RE = re.compile(r"^b[a-z2-7]{20,80}$")
HEX_RE = re.compile(r"^0x[0-9a-fA-F]+$")


# ── Movimientos de la cartera (#1531, 7-oct-2026) ───────────────────────────────────────────────────
# The chain keeps no per-account history; the node (archive) keeps every block's System.Events. A background
# thread walks FINALIZED blocks once, decodes the events with the runtime's own metadata (hlq_scale, stdlib)
# and keeps, per account, its Tokens.Transferred and fee lines. A regenerable CACHE (delete it and it rebuilds
# from block 1): public chain data only, nothing about who asks. On a runtime upgrade (System.CodeUpdated) the
# metadata is re-read at the next block, so old and new layouts both decode.
import hlq_scale  # noqa: E402  (same directory)

TRANSFERS_STATE = os.environ.get("HLQ_TRANSFERS_STATE", "/var/lib/hlq-mediator/transfers.json")
TRANSFERS_PER_ACCOUNT = 500             # newest kept per account
# LAB ONLY: a --dev instant-seal node never finalizes; =1 indexes up to BEST (reorgs not handled). Prod: unset.
TRANSFERS_UNFINALIZED = os.environ.get("HLQ_TRANSFERS_UNFINALIZED") == "1"
_tx_lock = threading.Lock()
_tx = {"genesis": None, "next": 1, "head": 0, "by": {}, "error": None}


def _tx_load():
    try:
        with open(TRANSFERS_STATE) as f:
            st = json.load(f)
        if isinstance(st.get("by"), dict) and isinstance(st.get("next"), int):
            _tx.update(genesis=st.get("genesis"), next=st["next"], by=st["by"])
    except (OSError, ValueError):
        pass


def _tx_save():
    try:
        os.makedirs(os.path.dirname(TRANSFERS_STATE), mode=0o755, exist_ok=True)
        tmp = TRANSFERS_STATE + ".tmp"
        with _tx_lock:
            blob = json.dumps({"genesis": _tx["genesis"], "next": _tx["next"], "by": _tx["by"]}, separators=(",", ":"))
        with open(tmp, "w") as f:
            f.write(blob)
        os.replace(tmp, TRANSFERS_STATE)
    except OSError:
        pass


def _tx_add(acc, line):
    lst = _tx["by"].setdefault(acc, [])
    lst.append(line)
    if len(lst) > TRANSFERS_PER_ACCOUNT:
        del lst[: len(lst) - TRANSFERS_PER_ACCOUNT]


def _tx_block(meta, n, h):
    raw = rpc("state_getStorage", [hlq_scale.SYSTEM_EVENTS_KEY, h])
    if not raw:
        return False
    upgraded = False
    lines = []
    for pallet, name, body, xi in meta.events(bytes.fromhex(raw[2:])):
        if pallet == "System" and name == "CodeUpdated":
            upgraded = True
        if pallet != "Tokens" or not isinstance(body, dict):
            continue
        if name == "Transferred":
            coin = str(body["coin"]).lower()
            a, f, t = str(body["amount"]), body["from"], body["to"]
            lines.append((f, [n, coin, "out", t, a, "transfer"]))
            lines.append((t, [n, coin, "in", f, a, "transfer"]))
        elif name == "FeeCharged":
            lines.append((body["payer"], [n, "hlq", "out", body["node"], str(body["burned"] + body["to_node"]), "fee"]))
            if body["to_node"]:
                lines.append((body["node"], [n, "hlq", "in", body["payer"], str(body["to_node"]), "fee"]))
    with _tx_lock:
        for acc, line in lines:
            _tx_add(acc.lower(), line)
    return upgraded


def _tx_indexer():
    _tx_load()
    meta, meta_at = None, None
    last_save = time.monotonic()
    while True:
        try:
            gen = rpc("chain_getBlockHash", [0])
            if _tx["genesis"] != gen:            # new chain (or first run): start over
                with _tx_lock:
                    _tx.update(genesis=gen, next=1, by={})
                meta = None
            fin = [] if TRANSFERS_UNFINALIZED else [rpc("chain_getFinalizedHead", [])]
            head = int(rpc("chain_getHeader", fin)["number"], 16)
            _tx["head"] = head
            done = 0
            while _tx["next"] <= head and done < 400:
                n = _tx["next"]
                h = rpc("chain_getBlockHash", [n])
                if meta is None or meta_at is None:
                    meta, meta_at = hlq_scale.Metadata(bytes.fromhex(rpc("state_getMetadata", [h])[2:])), n
                if _tx_block(meta, n, h):
                    meta = None                    # runtime changed: re-read metadata at the next block
                with _tx_lock:
                    _tx["next"] = n + 1
                done += 1
            _tx["error"] = None
            if done == 0 or time.monotonic() - last_save > 30:
                _tx_save()
                last_save = time.monotonic()
            time.sleep(0.2 if done >= 400 else 6)
        except Exception as e:                     # node down, bad bytes: say so, retry; never kill the mediator
            _tx["error"] = str(e)[:200]
            meta = None
            time.sleep(15)


# ── Market deals and runtime capabilities (D3, 7-oct-2026) ──────────────────────────────────────────
# A deal = two masks both saying "this happened" over an offer (pallet-market propose/confirm/cancel/reap_deal,
# genesis binary). The web must not guess pallet/call indices: /api/capabilities reads them from the runtime's own
# metadata (cached per spec version), so the page arms «Cerrar trato» only where the chain can take it.
DEALS_PREFIX = (twox128(b"Market") + twox128(b"Deals")).hex()
_caps = {"spec": None, "payload": None}
_caps_lock = threading.Lock()


def capabilities_payload() -> dict:
    spec = rpc("state_getRuntimeVersion", [])["specVersion"]
    with _caps_lock:
        if _caps["spec"] == spec and _caps["payload"]:
            return _caps["payload"]
    meta = hlq_scale.Metadata(bytes.fromhex(rpc("state_getMetadata", [])[2:]))
    pallets = {}
    for name in ("Market", "Pacts", "Tokens", "Forum", "Directory", "Reputation", "Justice", "Hosting"):
        p = meta.pallets.get(name)
        if not p:
            continue
        calls = {}
        if p["calls"] is not None:
            calls = {v[0]: k for k, v in meta.types[p["calls"]]["def"][1].items()}
        pallets[name] = {"index": p["index"], "calls": calls}
    m = pallets.get("Market", {}).get("calls", {})
    out = {"spec_version": spec, "pallets": pallets,
           "deals": all(c in m for c in ("propose_deal", "confirm_deal", "cancel_deal"))}
    with _caps_lock:
        _caps.update(spec=spec, payload=out)
    return out


def decode_deal(v: bytes) -> dict:
    return {"offer": int.from_bytes(v[0:8], "little"), "proposer": "0x" + v[8:40].hex(),
            "counterparty": "0x" + v[40:72].hex(), "at": int.from_bytes(v[72:76], "little"),
            "expires": int.from_bytes(v[76:80], "little"), "closed": bool(v[80])}


def deals_payload(acc: str) -> dict:
    best = int(rpc("chain_getHeader", [])["number"], 16)
    mine = []
    for did, raw in _read_map(DEALS_PREFIX):
        if len(raw) < 81:
            continue
        d = decode_deal(raw)
        if acc in (d["proposer"], d["counterparty"]):
            d["id"] = did
            d["expired"] = not d["closed"] and best > d["expires"]
            mine.append(d)
    mine.sort(key=lambda d: -d["id"])
    return {"deals": mine, "best_block": best}


PACTS_SEALED_PREFIX = twox128(b"Pacts") + twox128(b"Sealed")


def sealed_at(key: bytes):
    """Block at which a sealed-pact key (mode B) was recorded, or None. Public; reveals nothing but the key's age."""
    raw = rpc("state_getStorage", ["0x" + (PACTS_SEALED_PREFIX + blake2_128_concat(key)).hex()])
    return int.from_bytes(bytes.fromhex(raw[2:])[:4], "little") if raw else None


PACTS_MAP_PREFIX = twox128(b"Pacts") + twox128(b"Pacts")
PACTS_NEXT_KEY = "0x" + (twox128(b"Pacts") + twox128(b"NextPactId")).hex()


def pact_payload(pid: int) -> dict:
    """A mode-A pact by id (public: commitment, parties, who signed, window, completed). 8-oct-2026, for the villa and
    the devnet bots. Layout: commitment[32] · proposer[32] · parties Vec<32> · signed Vec<32> · at u32 · expires u32 · bool."""
    nxt = rpc("state_getStorage", [PACTS_NEXT_KEY])
    out = {"id": pid, "next_id": int.from_bytes(bytes.fromhex(nxt[2:]), "little") if nxt else 0, "pact": None}
    k = pid.to_bytes(8, "little")
    raw = rpc("state_getStorage", ["0x" + (PACTS_MAP_PREFIX + blake2_128_concat(k)).hex()])
    if not raw:
        return out
    v = bytes.fromhex(raw[2:]); r = hlq_scale.R(v)
    c = r.take(32).hex(); prop = r.take(32).hex()
    parties = ["0x" + r.take(32).hex() for _ in range(r.compact())]
    signed = ["0x" + r.take(32).hex() for _ in range(r.compact())]
    at = r.uint(4); exp = r.uint(4); done = bool(r.u8())
    out["pact"] = {"commitment": "0x" + c, "proposer": "0x" + prop, "parties": parties, "signed": signed,
                   "at": at, "expires": exp, "completed": done}
    return out


def transfers_payload(acc: str, limit: int, offset: int) -> dict:
    with _tx_lock:
        lst = list(_tx["by"].get(acc, []))
        nxt, head, err = _tx["next"], _tx["head"], _tx["error"]
    page = lst[::-1][offset:offset + limit]
    items = [{"block": b, "coin": c, "dir": d, "other": o, "amount": a, "kind": k} for b, c, d, o, a, k in page]
    return {"items": items, "indexed_to": nxt - 1, "head": head, "complete": nxt - 1 >= head and not err,
            "next_offset": offset + limit if offset + limit < len(lst) else None}


class Handler(BaseHTTPRequestHandler):
    server_version = "hlq-mediator/1"
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):  # no identity logs — condition (5)
        pass

    def _json(self, code: int, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Cache-Control", "no-store")
        # DESYNC FIX (2026-09-23, found by an oversize POST): error paths answer WITHOUT reading the body
        # (too big, rate-limited, bad size). On a kept-alive connection those unread bytes stayed in the
        # pipe and Caddy — which reuses upstream connections across visitors — glued the NEXT visitor's
        # request behind them (measured: a 5000-byte body turned the following request into a 501 whose
        # request line began with the leftover bytes). Request smuggling between proxy and backend. Every
        # error now closes the connection, so nothing unread can outlive the response it belonged to.
        if code >= 400:
            self.close_connection = True
            self.send_header("Connection", "close")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _ip(self) -> str:
        # R3 (operator review): trust ONLY the proxy-added value — the LAST element. A client-sent
        # X-Forwarded-For gets APPENDED to by Caddy, so the first element is attacker-controlled
        # (rate-limit bypass by rotating fake IPs). The Caddy patch additionally REPLACES the header
        # (header_up X-Forwarded-For {remote_host}); taking the last value is correct in both setups.
        return self.headers.get("X-Forwarded-For", self.client_address[0]).split(",")[-1].strip()

    def do_GET(self):
        try:
            path, _, query = self.path.partition("?")
            q = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
            if path == "/api/market/offers":
                if not allow(self._ip(), "market"):
                    return self._json(429, {"error": "rate limited"})
                p = offers_payload()
                limit = max(1, min(50, int(q.get("limit", "50"))))
                cursor = q.get("cursor")
                offers = p["offers"]
                if cursor is not None:
                    offers = [o for o in offers if o["id"] < int(cursor)]
                page = offers[:limit]
                nxt = page[-1]["id"] if len(offers) > limit else None
                # "total" (2026-09-25, the operator): the board says how many there are even though it only paints the first page.
                return self._json(200, {**p, "offers": page, "next_cursor": nxt, "total": len(p["offers"])})
            if path == "/api/justice/marks":
                if not allow(self._ip(), "justice"):
                    return self._json(429, {"error": "rate limited"})
                accs = [a.lower().removeprefix("0x") for a in q.get("accounts", "").split(",") if a]
                if not accs or len(accs) > 64 or not all(re.fullmatch(r"[0-9a-f]{64}", a) for a in accs):
                    return self._json(400, {"error": "accounts: 1..64 hex public keys"})
                return self._json(200, {"marks": guilty_marks(sorted(set(accs)))})
            if path == "/api/forum/posts":
                if not allow(self._ip(), "forum"):
                    return self._json(429, {"error": "rate limited"})
                return self._json(200, posts_payload())
            if path == "/api/standing":
                # No identity log (the handler's log_message is silent): account↔IP never written.
                if not allow(self._ip(), "standing"):
                    return self._json(429, {"error": "rate limited"})
                acc = q.get("account", "")
                if not HEX32_RE.match(acc):
                    return self._json(400, {"error": "bad account"})
                return self._json(200, standing_payload(acc))
            if path == "/api/balance":
                # 7-oct-2026: the wallet showed «0» to everyone — it had no endpoint to read. Public chain data.
                if not allow(self._ip(), "standing"):
                    return self._json(429, {"error": "rate limited"})
                acc = q.get("account", "")
                if not HEX32_RE.match(acc):
                    return self._json(400, {"error": "bad account"})
                out = {}
                for name, coin in (("hlq", "00"), ("sov", "01")):
                    raw = rpc("state_call", ["TokensApi_balance_of", "0x" + acc.lower().removeprefix("0x") + coin])
                    out[name] = str(int.from_bytes(bytes.fromhex(raw[2:])[:16], "little"))
                out["best_block"] = int(rpc("chain_getHeader", [])["number"], 16)
                return self._json(200, out)
            if path == "/api/capabilities":
                if not allow(self._ip(), "standing"):
                    return self._json(429, {"error": "rate limited"})
                return self._json(200, capabilities_payload())
            if path == "/api/market/deals":
                if not allow(self._ip(), "standing"):
                    return self._json(429, {"error": "rate limited"})
                acc = q.get("account", "").lower()
                if not HEX32_RE.match(acc):
                    return self._json(400, {"error": "bad account"})
                return self._json(200, deals_payload("0x" + acc.removeprefix("0x")))
            if path == "/api/pacts/pact":
                if not allow(self._ip(), "standing"):
                    return self._json(429, {"error": "rate limited"})
                try:
                    pid = int(q.get("id", ""))
                except ValueError:
                    return self._json(400, {"error": "bad id"})
                return self._json(200, pact_payload(pid))
            if path == "/api/pacts/sealed":
                if not allow(self._ip(), "standing"):
                    return self._json(429, {"error": "rate limited"})
                k = q.get("key", "")
                if not HEX32_RE.match(k):
                    return self._json(400, {"error": "bad key"})
                return self._json(200, {"key": "0x" + k.lower().removeprefix("0x"), "sealed_at": sealed_at(bytes.fromhex(k.lower().removeprefix("0x")))})
            if path == "/api/transfers":
                if not allow(self._ip(), "standing"):
                    return self._json(429, {"error": "rate limited"})
                acc = q.get("account", "").lower()
                if not HEX32_RE.match(acc):
                    return self._json(400, {"error": "bad account"})
                acc = "0x" + acc.removeprefix("0x")
                try:
                    limit = max(1, min(100, int(q.get("limit", "30"))))
                    offset = max(0, int(q.get("offset", "0")))
                except ValueError:
                    return self._json(400, {"error": "bad paging"})
                return self._json(200, transfers_payload(acc, limit, offset))
            if path == "/api/resolve":
                if not allow(self._ip(), "resolve"):
                    return self._json(429, {"error": "rate limited"})
                h = q.get("handle", "").strip().lower()
                if not HANDLE_RE.match(h):
                    return self._json(400, {"error": "bad handle"})
                pub = resolve_handle(h)
                if pub is None:
                    return self._json(404, {"error": "unknown handle"})
                return self._json(200, {"handle": h, "account": "0x" + pub.hex()})
            if path == "/api/tx/meta":
                # 28-sep (#1468): reading what to sign is PUBLIC data and costs a few reads; it used to share
                # the "tx" bucket with submissions, so binding a node and then replacing it in the same minute
                # (8 calls) hit the limit — measured by the stranger's walk. Submissions stay at 6/min.
                if not allow(self._ip(), "txmeta"):
                    return self._json(429, {"error": "rate limited"})
                ss58 = q.get("ss58", "")
                acc = q.get("account", "")
                # #1468: the node-binding page holds the mask's PUBLIC KEY, not its address; both name
                # the same account, so either is accepted (hex = exactly 32 bytes, nothing else).
                if acc and re.fullmatch(r"0x[0-9a-fA-F]{64}", acc):
                    pub = bytes.fromhex(acc[2:])
                else:
                    pub = ss58_to_pubkey(ss58) if SS58_RE.match(ss58) else None
                if pub is None:
                    return self._json(400, {"error": "bad ss58 (a Harlequin 1728 address) or account (0x + 32 bytes)"})
                nonce = rpc("system_accountNextIndex", [ss58_generic(pub)])
                ver = rpc("state_getRuntimeVersion", [])
                genesis = rpc("chain_getBlockHash", [0])
                head = rpc("chain_getHeader", [])
                best_number = int(head["number"], 16)
                out = {
                    "nonce": nonce,
                    "genesis_hash": genesis,
                    "spec_version": ver["specVersion"],
                    "tx_version": ver["transactionVersion"],
                    "best_number": best_number,
                }
                # A MORTAL signature commits to the hash of its era's birth block; the browser computes
                # the birth height (bind.js eraFor) and asks for that one public hash. Bounded to recent
                # blocks so this cannot be used to walk the chain.
                birth = q.get("birth")
                if birth is not None:
                    b = int(birth)
                    if not (0 <= best_number - b <= 4096):
                        return self._json(400, {"error": "birth out of range"})
                    out["birth_hash"] = rpc("chain_getBlockHash", [b])
                return self._json(200, out)
            if path == "/api/consensus/vote_keys":
                # #1468: the binding page must know whether this mask ALREADY has a node bound (claiming
                # again replaces it). Public chain data, the same bytes any node returns; cached briefly
                # so a burst of page loads is one runtime call.
                if not allow(self._ip(), "standing"):
                    return self._json(429, {"error": "rate limited"})
                with _vk_lock:
                    if _vk_cache["hex"] is None or time.monotonic() - _vk_cache["at"] > 6:
                        _vk_cache.update(at=time.monotonic(), hex=rpc("state_call", ["HarlequinConsensusApi_vote_keys", "0x"]))
                    return self._json(200, {"hex": _vk_cache["hex"]})
            if path == "/api/villa/challenge":
                # #637: hand the client a fresh single-use challenge. We build the EXACT bytes to sign
                # and return them (msg_hex) so client and server never rebuild the message separately.
                if not allow(self._ip(), "login"):
                    return self._json(429, {"error": "rate limited"})
                mask = q.get("mask", "")
                if not SS58_RE.match(mask) or ss58_to_pubkey(mask) is None:
                    return self._json(400, {"error": "bad mask (not a Harlequin address)"})
                nonce = os.urandom(32)
                issued_at = int(time.time())
                msg = LOGIN_LABEL + nonce + GENESIS_HASH + issued_at.to_bytes(8, "little")
                with _challenges_lock:
                    now = time.monotonic()
                    # prune expired while we are here (RAM only, nothing retained past TTL)
                    for k in [k for k, v in _challenges.items() if v[1] <= now]:
                        _challenges.pop(k, None)
                    _challenges[nonce.hex()] = (msg.hex(), now + LOGIN_TTL_S, mask)
                return self._json(200, {
                    "nonce": nonce.hex(),
                    "msg_hex": msg.hex(),
                    # 2026-08-03: exposed as its own field too — the UI contract (deploy notes 21-jul)
                    # expected it and only msg_hex carried it; the skew cost the founder his first login.
                    "genesis_hash": GENESIS_HASH.hex(),
                    "issued_at": issued_at,
                    "ttl": LOGIN_TTL_S,
                })
            m = re.match(r"^/api/ipfs/([^/]+)$", path)
            if m:
                if not allow(self._ip(), "ipfs"):
                    return self._json(429, {"error": "rate limited"})
                cid = m.group(1)
                if not CID_RE.match(cid):
                    return self._json(400, {"error": "bad cid"})
                digest = cid_to_digest(cid)
                local = os.path.join(DETAIL_STORE, digest.hex()) if digest else None
                if local and os.path.isfile(local):
                    with open(local, "rb") as f:
                        body = f.read(MAX_IPFS_BYTES + 1)
                    self.send_response(200)
                    self.send_header("Content-Type", "application/octet-stream")
                    # content-addressed: the bytes behind a CID never change, so the browser may keep them
                    self.send_header("Cache-Control", "public, max-age=31536000, immutable")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body)
                    return
                for base in IPFS_UPSTREAMS:
                    try:
                        with urllib.request.urlopen(base + cid, timeout=20) as r:
                            body = r.read(MAX_IPFS_BYTES + 1)
                        if len(body) > MAX_IPFS_BYTES:
                            return self._json(502, {"error": "detail too large"})
                        self.send_response(200)
                        self.send_header("Content-Type", "application/octet-stream")
                        self.send_header("Cache-Control", "no-store")
                        self.send_header("Content-Length", str(len(body)))
                        self.end_headers()
                        self.wfile.write(body)
                        return
                    except Exception:
                        continue
                return self._json(502, {"error": "detail unavailable"})
            return self._json(404, {"error": "not found"})
        except ValueError:
            # int() of a malformed limit/cursor/birth: the client's mistake, not ours (27-sep).
            return self._json(400, {"error": "bad parameter"})
        except Exception:
            # Never echo internals (node RPC URL, paths) to the internet (27-sep audit).
            return self._json(500, {"error": "internal error"})

    def do_POST(self):
        try:
            # A chunked body has no Content-Length: every handler here would read 0 bytes and leave the
            # whole body in the pipe (the desync above). Refuse it; Caddy sends Content-Length anyway.
            if "chunked" in (self.headers.get("Transfer-Encoding") or "").lower():
                return self._json(411, {"error": "Content-Length required"})
            if self.path == "/api/villa/login":
                # #637: verify a challenge response. Prove control of the mask, nothing more — no
                # server session, no token; the client flips the logged-in UI, and every sensitive
                # op re-signs on-device (coherent with #680 and the sovereign-key rule).
                if not allow(self._ip(), "login"):
                    return self._json(429, {"error": "rate limited"})
                n = int(self.headers.get("Content-Length", "0"))
                if n <= 0 or n > 4096:
                    return self._json(400, {"error": "bad size"})
                body = json.loads(self.rfile.read(n))
                mask = body.get("mask_ss58", "")
                nonce = body.get("nonce", "")
                sig_hex = body.get("sig", "")
                if not (SS58_RE.match(mask) and re.fullmatch(r"[0-9a-fA-F]{64}", nonce)
                        and re.fullmatch(r"[0-9a-fA-F]{128}", sig_hex)):
                    return self._json(400, {"error": "bad login payload"})
                # Single use: pop the challenge (so a replay finds nothing), then check freshness.
                with _challenges_lock:
                    entry = _challenges.pop(nonce, None)
                if entry is None:
                    return self._json(401, {"ok": False, "error": "unknown or used challenge"})
                msg_hex, expiry, mask_bound = entry
                if time.monotonic() > expiry:
                    return self._json(401, {"ok": False, "error": "challenge expired"})
                if mask_bound != mask:  # the response must be for the mask the challenge was issued to
                    return self._json(401, {"ok": False, "error": "mask mismatch"})
                pubkey = ss58_to_pubkey(mask)
                if pubkey is None:
                    return self._json(400, {"error": "bad mask"})
                # FAIL-CLOSED: valid only if hlq-sigverify exits 0 over the SAME bytes we handed out.
                if sig_is_valid(pubkey, bytes.fromhex(msg_hex), bytes.fromhex(sig_hex)):
                    return self._json(200, {"ok": True, "mask_ss58": mask})
                return self._json(401, {"ok": False, "error": "bad signature"})
            if self.path == "/api/detail":
                if not allow(self._ip(), "detail"):
                    return self._json(429, {"error": "rate limited"})
                n = int(self.headers.get("Content-Length", "0"))
                if n <= 0 or n > MAX_DETAIL_BYTES:
                    return self._json(400, {"error": "detail must be 1..4096 bytes"})
                try:
                    return self._json(200, store_detail(self.rfile.read(n)))
                except UnicodeDecodeError:
                    return self._json(400, {"error": "detail must be UTF-8 text"})
                except ValueError as e:
                    return self._json(400, {"error": str(e)})
                except OverflowError as e:
                    return self._json(503, {"error": str(e)})
            if self.path != "/api/tx":
                return self._json(404, {"error": "not found"})
            if not allow(self._ip(), "tx"):
                return self._json(429, {"error": "rate limited"})
            n = int(self.headers.get("Content-Length", "0"))
            if n <= 0 or n > MAX_TX_BYTES * 2 + 64:
                return self._json(400, {"error": "bad size"})
            body = json.loads(self.rfile.read(n))
            xt = body.get("xt", "")
            if not HEX_RE.match(xt) or len(xt) // 2 > MAX_TX_BYTES:
                return self._json(400, {"error": "bad xt"})
            try:
                h = rpc("author_submitExtrinsic", [xt])
                return self._json(200, {"hash": h})
            except RuntimeError as e:
                return self._json(400, {"error": str(e)[:300]})  # node's rejection, verbatim
        except ValueError:
            return self._json(400, {"error": "bad request"})
        except Exception:
            return self._json(500, {"error": "internal error"})


if __name__ == "__main__":
    print(f"hlq-mediator on {LISTEN[0]}:{LISTEN[1]} → {RPC}")
    threading.Thread(target=_tx_indexer, name="tx-indexer", daemon=True).start()
    ThreadingHTTPServer(LISTEN, Handler).serve_forever()
