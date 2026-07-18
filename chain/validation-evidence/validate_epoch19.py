#!/usr/bin/env python3
"""
validate_epoch19.py — Validation #19, "counting WITHOUT a king": recompute/decay happens
AUTOMATICALLY at every epoch boundary (every EpochLength blocks), with NOBODY signing
advance_epoch (the extrinsic was removed). Proves the chain's own clock does the counting,
not a root button. Binary sha df85528b. Node producing blocks (manual-seal-1000).
"""
import time, sys
from substrateinterface import SubstrateInterface, Keypair

s = SubstrateInterface(url="ws://127.0.0.1:9944", ss58_format=42)
bob = Keypair.create_from_uri("//Bob", ss58_format=42)

# 1) advance_epoch must be GONE from the metadata
calls = [c.name for c in s.get_metadata_module("Reputation").calls or []]
ae_gone = "advance_epoch" not in calls
print(f"Reputation calls: {calls}")
print(f"[{'PASS' if ae_gone else 'FAIL'}] advance_epoch REMOVED from the runtime: {ae_gone}")

def block():
    h = s.get_block_header()
    return int(h["header"]["number"], 16) if isinstance(h["header"]["number"], str) else int(h["header"]["number"])
def epoch(): return int(s.query("Reputation", "Epoch", []).value)
def bob_ev():
    v = s.query("Reputation", "Evidence", [bob.ss58_address, "Commerce"])
    return int(v.value) if v.value is not None else 0

print(f"\nStart: block={block()} epoch={epoch()} Bob.ev={bob_ev()} (genesis seed, NOT re-injected)")
print("Observing (signing NOTHING) until several epoch boundaries are captured...\n")
print(f"{'t':>3} {'block':>6} {'epoch':>6} {'Bob.ev':>10} {'epoch_ratio':>12}")

seen = {}            # epoch -> bob_ev when first observed
last_e = epoch()
seen[last_e] = bob_ev()
ratios = []
t0 = time.time()
while time.time() - t0 < 90 and len(seen) < 7:
    e = epoch(); b = bob_ev(); bl = block()
    if e not in seen:
        prev = seen[e-1] if (e-1) in seen else None
        r = (b/prev) if prev else 0.0
        if prev: ratios.append(r)
        seen[e] = b
        print(f"{int(time.time()-t0):>3} {bl:>6} {e:>6} {b:>10} {r:>12.4f}")
    time.sleep(1.5)

import statistics
avg = statistics.mean(ratios) if ratios else 0.0
print(f"\nepoch boundaries observed: {sorted(seen)}  mean Bob.ev ratio = {avg:.4f} (expected ≈0.90)")
decayed = seen[max(seen)] < seen[min(seen)]
ticks_auto = len(seen) >= 3   # epoch advanced without us signing advance_epoch
ok = ae_gone and decayed and ticks_auto and abs(avg-0.90) < 0.03
print(f"[{'PASS' if ticks_auto else 'FAIL'}] Epoch advances ON ITS OWN (no advance_epoch signed): {len(seen)-1} boundaries")
print(f"[{'PASS' if decayed else 'FAIL'}] Bob.ev decays automatically: {seen[min(seen)]} -> {seen[max(seen)]}")
print("\nRESULT #19:", "✅ COUNTING WITHOUT A KING OK (decay driven by the chain clock, no root button)" if ok else "❌ review")
sys.exit(0 if ok else 1)
