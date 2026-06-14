#!/usr/bin/env python3
"""
Self-audit tests for the consensus simulator. No dependencies (plain asserts).
Run:  python3 tests/test_consensus.py   (from prototipos/consenso/)
"""

from __future__ import annotations

import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from wtc_sim.consensus import ConsensusParams, run_once
from wtc_sim.population import (
    population_clustered_adversary,
    population_reputation_fraction,
    population_sybil,
)
import partition

PARAMS = ConsensusParams(k=20, alpha=14, beta=12, max_rounds=80)


def _aggregate(rep, adv, weighted, trials=40, seed=1984):
    rng = random.Random(seed)
    safe = capture = 0
    for _ in range(trials):
        r = run_once(rep, adv, PARAMS, rng, weighted=weighted)
        safe += r["safe"]; capture += r["capture"]
    return 100 * safe / trials, 100 * capture / trials


def _aggregate_cap(rep, adv, clusters, cap_cluster, adversary="fixed", trials=60, seed=1984):
    rng = random.Random(seed)
    safe = capture = 0
    for _ in range(trials):
        r = run_once(rep, adv, PARAMS, rng, weighted=True,
                     clusters=clusters, cap_cluster=cap_cluster, adversary=adversary)
        safe += r["safe"]; capture += r["capture"]
    return 100 * safe / trials, 100 * capture / trials


def test_no_adversary_is_safe():
    rep, adv = population_reputation_fraction(0.0)
    safe, capture = _aggregate(rep, adv, weighted=True)
    assert safe == 100.0 and capture == 0.0


def test_sybil_weighted_is_safe():
    """93% of the nodes in the adversary's hands, but with reputation ~0 -> safe network."""
    rep, adv = population_sybil()
    safe, capture = _aggregate(rep, adv, weighted=True)
    assert safe >= 95.0, f"weighted sybil should be safe, was {safe}%"
    assert capture == 0.0


def test_sybil_uniform_fails():
    """Without weighting by reputation, the same fake crowd breaks the network (contrast)."""
    rep, adv = population_sybil()
    safe, capture = _aggregate(rep, adv, weighted=False)
    assert safe < 50.0, f"uniform sybil should NOT be safe, was {safe}%"


def test_reputation_majority_captures():
    """With 50% of the reputation, the adversary captures honest nodes."""
    rep, adv = population_reputation_fraction(0.5)
    safe, capture = _aggregate(rep, adv, weighted=True)
    assert capture > 0.0 and safe == 0.0


def test_threshold_is_reputation_not_nodes():
    """Little adversarial reputation (10%) in few nodes -> safe; shows reputation is what weighs."""
    rep, adv = population_reputation_fraction(0.1)
    safe, _ = _aggregate(rep, adv, weighted=True)
    assert safe == 100.0


def test_independence_protects_from_correlated_bloc():
    """
    An adversary with 45% of the reputation but ALL in one correlated cluster captures the network
    with rep-only sampling; INDEPENDENCE-weighted sampling (per-cluster cap) neutralises it.
    """
    rep, adv, cl = population_clustered_adversary(0.45, n_adv_clusters=1)
    safe_no, cap_no = _aggregate_cap(rep, adv, cl, cap_cluster=None)      # rep-only
    safe_cap, cap_cap = _aggregate_cap(rep, adv, cl, cap_cluster=3)       # +independence
    assert cap_no > 0.0, "rep-only should be captured by the correlated bloc"
    assert safe_cap >= 95.0 and cap_cap == 0.0, "the independence cap should protect"


def test_independence_yields_if_adversary_fragments_enough():
    """
    The cap over k=20, alpha=14 forces the adversary to use >= ceil(alpha/cap) distinct clusters to
    capture. With cap=3 it must fragment into >=5 blocs; with fewer, the network stays safe. Honest
    frontier: each bloc must look INDEPENDENT (which the reputation engine resists).
    """
    safe2, cap2 = _aggregate_cap(*population_clustered_adversary(0.45, n_adv_clusters=2), cap_cluster=3)
    safe6, cap6 = _aggregate_cap(*population_clustered_adversary(0.45, n_adv_clusters=6), cap_cluster=3)
    assert cap2 == 0.0, "with 2 blocs (<5) the cap should hold"
    assert cap6 > 0.0, "with 6 blocs (>=5) the adversary should be able to capture again"


def test_adaptive_adversary_does_not_break_safety():
    """
    The ADAPTIVE adversary (anti-finality splitter) attacks LIVENESS (stalls), but below the
    reputation threshold it NEVER forces a false decision (safety intact).
    """
    rep, adv, cl = population_clustered_adversary(0.3, n_adv_clusters=1)
    _, capture = _aggregate_cap(rep, adv, cl, cap_cluster=None, adversary="adaptive")
    assert capture == 0.0, "the adaptive one should not capture (only stall)"


def test_message_loss_degrades_liveness_not_safety():
    """
    Message loss (latency / unreliable network): an adversary below the threshold (25%) NEVER captures
    even under high loss (safety preserved); loss only adds stalls (liveness).
    """
    rep, adv = population_reputation_fraction(0.25)
    for p in (0.0, 0.4, 0.6):
        rng = random.Random(1984)
        capture = 0
        for _ in range(40):
            r = run_once(rep, adv, PARAMS, rng, weighted=True, loss=p)
            capture += r["capture"]
        assert capture == 0, f"loss {p}: should not capture (safety), was {capture}"


def test_partition_long_forks_without_mitigation():
    """
    Partition attack: a globally harmless adversary (15%) concentrated in the small group, with a long
    partition, captures that group and produces a FORK on heal (safety failure). Documents the real
    risk the simulator found.
    """
    m = partition.measure(0.15, D=90, network_quorum=0.0, trials=40)
    assert m["fork"] > 50.0, f"the long partition should fork, was {m['fork']}%"


def test_partition_quorum_preserves_safety():
    """
    Network-quorum mitigation: conditioning finality on seeing ≥60% of the reputation almost entirely
    eliminates the fork (safety preserved), at the cost of stalls (recoverable liveness).
    """
    no = partition.measure(0.15, D=90, network_quorum=0.0, trials=40)
    yes = partition.measure(0.15, D=90, network_quorum=0.6, trials=40)
    assert yes["fork"] < no["fork"] * 0.2, f"the quorum should cut the fork sharply ({no['fork']}→{yes['fork']})"
    assert yes["fork"] < 10.0, f"with quorum the fork should be low, was {yes['fork']}%"


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    failures = 0
    for t in tests:
        try:
            t(); print(f"  PASS  {t.__name__}")
        except AssertionError as e:
            failures += 1; print(f"  FAIL  {t.__name__}: {e}")
    print(f"\n{len(tests)-failures}/{len(tests)} tests OK")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
