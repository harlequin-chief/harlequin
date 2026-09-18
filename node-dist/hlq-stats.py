#!/usr/bin/env python3
"""hlq-stats.py — publish a small, safe network summary for the public Network page.

Polls a LOCAL Harlequin node's JSON-RPC and writes `network.json` atomically. The web server (Caddy)
serves that file same-origin; the Network page (network.js) fetches it. Read-only: it never controls the
node, never writes anything but the output file.

What it publishes (deliberately minimal — aggregate only, nothing that deanonymises a peer):
  chain name, best block, finalized block, peer count, syncing flag, and how OLD the reading is.
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
        nums = set(range(max(0, best - 23), best + 1))                # 24 newest (the live tip)
        if finalized is not None:
            nums |= set(range(max(0, finalized - 5), finalized + 1))  # 6 at the finalised frontier (the ✓)
        for num in sorted(nums, reverse=True):
            try:
                bh = rpc(rpc_url, "chain_getBlockHash", [num])
            except Exception:
                bh = None
            # Extrinsic COUNT only — an aggregate. Never the extrinsic bodies, senders, or args, so no
            # address or transaction pattern is ever exposed (ANONYMOUS invariant above).
            # Parent hash ("p") is the block-explorer chain link — public header data, same anonymity class
            # as the block hash itself.
            xcount = None
            parent = None
            if bh is not None:
                try:
                    blk = rpc(rpc_url, "chain_getBlock", [bh]) or {}
                    xcount = len(blk.get("block", {}).get("extrinsics", []))
                    parent = blk.get("block", {}).get("header", {}).get("parentHash")
                except Exception:
                    xcount = None
            recent.append({
                "n": num,
                "hash": bh,
                "p": parent,
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
        # OPSEC (2026-08-01): this used to publish the absolute server clock ("updated":
        # "2026-07-30T18:01:36Z"). Anyone polling the file could read our machine time and,
        # over a few reads, the rhythm of how we operate it — when someone is at the keyboard
        # and when nobody is. A visitor only needs to know whether the reading is fresh, so we
        # serve an AGE in seconds and never a wall-clock stamp. `builtAt` stays internal.
        "ageSeconds": 0,
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
    ap.add_argument("--stale-after", type=int, default=480,
                    help="seconds without the finalized head advancing before the local view is "
                         "treated as suspect (natural cadence is ~180s; 480s ≈ >2 missed rounds)")
    args = ap.parse_args()

    # Anti-fork guard (2026-07-28, from the 27-jul incident): when the local node is isolated it
    # keeps authoring a solo fork and this feed would publish that fork as the truth. The finalized
    # head is the only part of the view that carries the committee's signatures, so it is the trust
    # anchor: if it stops advancing (or the node has 0 peers), we freeze the last CONFIRMED snapshot
    # and say so (viewStale) instead of serving an unbacked tip. Live peers/updated stay honest.
    guard = {"fin": None, "since": None, "snapshot": None}

    def tick():
        try:
            data = collect(args.rpc)
        except Exception as e:
            print(f"hlq-stats: node unreachable / error: {e}", file=sys.stderr)
            # leave the previous network.json in place; do not write a broken file
            return False
        now = time.time()
        fin = data.get("finalizedBlock")
        if fin is not None and fin != guard["fin"]:
            guard.update(fin=fin, since=now, snapshot=data)
        finality_stalled = guard["since"] is not None and (now - guard["since"]) > args.stale_after
        no_peers = not data.get("peers")
        if (finality_stalled or no_peers) and guard["snapshot"] is not None:
            out = dict(guard["snapshot"])            # last view backed by an advancing finality
            out["peers"] = data.get("peers")         # current reachability stays honest
            out["syncing"] = data.get("syncing")
            out["viewStale"] = True
            # Same rule for the freeze marker: how long we have been blind, not since when.
            out["staleForSeconds"] = int(now - guard["since"])
            out["ageSeconds"] = int(now - guard["since"])
        else:
            out = data
            out["viewStale"] = False
        try:
            write_atomic(args.out, out)
            return True
        except Exception as e:
            print(f"hlq-stats: write error: {e}", file=sys.stderr)
            return False

    if args.interval <= 0:
        sys.exit(0 if tick() else 1)
    while True:
        tick()
        time.sleep(args.interval)

if __name__ == "__main__":
    main()
