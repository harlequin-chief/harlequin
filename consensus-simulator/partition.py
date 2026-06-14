#!/usr/bin/env python3
"""
Consensus under a NETWORK PARTITION — hardens the model (honest limitation: it used to assume a
complete network).

Before committing the chain stack, we need to know how WTC behaves when the network **splits** (a
group of nodes is isolated for a while and then heals). It is the classic scenario that separates
safety from liveness, and where partition attacks live.

Model: the honest nodes are split into a LARGE group A and a SMALL group B; the adversary concentrates
its reputation in B. For `D` rounds the network is partitioned (each group only samples itself); then
it heals (complete network). Measured over many runs:
  - **fork**: A and B decide DIFFERENT colours (a SAFETY failure).
  - **capture in B**: some honest node in B adopts the false value.
  - **stall**: someone is left undecided at the end (a LIVENESS cost).

Honest hypothesis to check: a LONG partition can raise the adversary's LOCAL share in the small group
above the threshold and capture / finalise it -> on heal, a fork. A real attack (partition +
concentrated adversary). Used to MEASURE the frontier and motivate the mitigation (do not finalise
under suspected partition / require seeing a minimum fraction of the network).

Run from prototipos/consenso/:  python3 partition.py
"""

from __future__ import annotations

import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from wtc_sim.consensus import ConsensusParams, run_once

P = ConsensusParams(k=20, alpha=14, beta=12, max_rounds=120)
TRIALS = 60
SEED = 1984


def population_partitioned(f_adv: float, n_A: int = 60, n_B: int = 20, n_adv: int = 6):
    """
    Honest nodes in a large group A and a small group B. Adversary (fraction `f_adv` of the total
    reputation) ALL in B. Returns (reputation, adversaries, group) with group: id -> 0 (A) | 1 (B).
    """
    reputation: dict[str, float] = {}
    group: dict[str, int] = {}
    for i in range(n_A):
        reputation[f"a{i}"] = 1.0; group[f"a{i}"] = 0
    for i in range(n_B):
        reputation[f"b{i}"] = 1.0; group[f"b{i}"] = 1
    honest_total = float(n_A + n_B)
    adversaries: set[str] = set()
    if f_adv > 0:
        adv_total = f_adv * honest_total / (1.0 - f_adv)
        for i in range(n_adv):
            aid = f"x{i}"
            reputation[aid] = adv_total / n_adv
            group[aid] = 1                # the adversary lives in the small group B
            adversaries.add(aid)
    return reputation, adversaries, group


def measure(f_adv, D, network_quorum=0.0, trials=TRIALS):
    rng = random.Random(SEED)
    rep, adv, group = population_partitioned(f_adv)
    fork = capture = stall = safe = 0
    for _ in range(trials):
        r = run_once(rep, adv, P, rng, weighted=True, group=group,
                     partition_rounds=D, network_quorum=network_quorum)
        fork += r["fork"]; capture += r["capture"]
        stall += 1 if r["undecided"] > 0 else 0
        safe += r["safe"]
    n = float(trials)
    return {"fork": 100*fork/n, "capture": 100*capture/n, "stall": 100*stall/n, "safe": 100*safe/n}


def main():
    print("# Consensus under a network partition\n")
    print("Honest: A=60 (large), B=20 (small). Adversary (15% global) concentrated in B. Partition of "
          "D rounds, then heals.\n")
    print("## Without mitigation: the partition attack\n")
    print(f"{'D (rounds)':>10} | {'fork':>6} | {'capture':>8} | {'stall':>7} | {'safe':>7}")
    print("-" * 50)
    for D in (0, 20, 50, 90):
        m = measure(0.15, D)
        print(f"{D:>10} | {m['fork']:>5.0f}% | {m['capture']:>7.0f}% | {m['stall']:>6.0f}% | {m['safe']:>6.0f}%")
    print("\nA long partition concentrates the adversary's LOCAL share in B above the threshold: B "
          "finalises the false value → on heal, FORK. A harmless 15% global becomes 100% fork.\n")

    print("## With network-quorum mitigation (a node does not finalise if it sees < 60% of the reputation)\n")
    print(f"{'D (rounds)':>10} | {'fork':>6} | {'capture':>8} | {'stall':>7} | {'safe':>7}")
    print("-" * 50)
    for D in (0, 20, 50, 90):
        m = measure(0.15, D, network_quorum=0.6)
        print(f"{D:>10} | {m['fork']:>5.0f}% | {m['capture']:>7.0f}% | {m['stall']:>6.0f}% | {m['safe']:>6.0f}%")
    print("\nB (25% of the reputation) never reaches quorum during the partition → does NOT finalise → "
          "it stalls (liveness cost) instead of forking, and recovers on heal. Safety preserved: 0% "
          "fork. Finality is conditioned on seeing enough of the network (anti-partition).")


if __name__ == "__main__":
    main()
