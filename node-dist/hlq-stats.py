#!/usr/bin/env python3
"""hlq-stats.py — publish a small, safe network summary for the public Network page.

Polls a LOCAL Harlequin node's JSON-RPC and writes `network.json` atomically. The web server (Caddy)
serves that file same-origin; the Network page (network.js) fetches it. Read-only: it never controls the
node, never writes anything but the output file.

What it publishes (deliberately minimal — aggregate only, nothing that deanonymises a peer):
  chain name, best block, finalized block, peer count, syncing flag, updated timestamp (UTC).
What it NEVER publishes: peer IPs, peer-ids, node names, RPC internals, machine info.

Usage:
  hlq-stats.py [--rpc http://127.0.0.1:9944] [--out /var/www/harlequin/network.json] [--interval 15]
  --interval 0 = run once and exit (good for a systemd timer / cron); >0 = loop every N seconds.
"""
import argparse
import json
import os
import sys
import tempfile
import time
import urllib.request

def rpc(url, method, params=None):
    body = json.dumps({"id": 1, "jsonrpc": "2.0", "method": method, "params": params or []}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=5) as r:          # local RPC only
        out = json.loads(r.read())
    if "error" in out:
        raise RuntimeError(out["error"])
    return out.get("result")

def hex_to_int(h):
    try:
        return int(h, 16)
    except (TypeError, ValueError):
        return None

def collect(rpc_url):
    """Return the safe aggregate dict, or raise if the node is unreachable."""
    health = rpc(rpc_url, "system_health") or {}
    chain = rpc(rpc_url, "system_chain")
    best_hdr = rpc(rpc_url, "chain_getHeader") or {}
    best = hex_to_int(best_hdr.get("number"))
    finalized = None
    try:
        fin_hash = rpc(rpc_url, "chain_getFinalizedHead")
        fin_hdr = rpc(rpc_url, "chain_getHeader", [fin_hash]) or {}
        finalized = hex_to_int(fin_hdr.get("number"))
    except Exception:
        pass
    # Recent blocks (height + hash + finalised?), newest first. Two clusters so the viewer always shows
    # both the live tip AND the finalised frontier (the ✓): the 10 newest blocks + 6 around the finalised
    # head. When the finality lag is small they merge into one contiguous run; when it is large (e.g. a
    # catch-up) they are two groups and the viewer draws a "…" gap. ANONYMOUS: only the public chain head —
    # never authors, peers, IPs or identities.
    recent = []
    if best is not None:
        nums = set(range(max(0, best - 9), best + 1))                 # 10 newest (the live tip)
        if finalized is not None:
            nums |= set(range(max(0, finalized - 5), finalized + 1))  # 6 at the finalised frontier (the ✓)
        for num in sorted(nums, reverse=True):
            try:
                bh = rpc(rpc_url, "chain_getBlockHash", [num])
            except Exception:
                bh = None
            # Extrinsic COUNT only — an aggregate. Never the extrinsic bodies, senders, or args, so no
            # address or transaction pattern is ever exposed (ANONYMOUS invariant above).
            xcount = None
            if bh is not None:
                try:
                    blk = rpc(rpc_url, "chain_getBlock", [bh]) or {}
                    xcount = len(blk.get("block", {}).get("extrinsics", []))
                except Exception:
                    xcount = None
            recent.append({
                "n": num,
                "hash": bh,
                "fin": finalized is not None and num <= finalized,
                "x": xcount,
            })
    return {
        "chain": chain,
        "bestBlock": best,
        "finalizedBlock": finalized,
        "peers": health.get("peers"),
        "syncing": health.get("isSyncing"),
        "recentBlocks": recent,
        "updated": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }

def write_atomic(path, data):
    d = os.path.dirname(os.path.abspath(path)) or "."
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".network.", suffix=".json")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(data, f, separators=(",", ":"))
            f.write("\n")
        os.chmod(tmp, 0o644)
        os.replace(tmp, path)      # atomic on the same filesystem
    except Exception:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rpc", default="http://127.0.0.1:9944")
    ap.add_argument("--out", default="/var/www/harlequin/network.json")
    ap.add_argument("--interval", type=int, default=0)
    args = ap.parse_args()

    def tick():
        try:
            write_atomic(args.out, collect(args.rpc))
            return True
        except Exception as e:
            print(f"hlq-stats: node unreachable / error: {e}", file=sys.stderr)
            # leave the previous network.json in place; do not write a broken file
            return False

    if args.interval <= 0:
        sys.exit(0 if tick() else 1)
    while True:
        tick()
        time.sleep(args.interval)

if __name__ == "__main__":
    main()
