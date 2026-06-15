#!/usr/bin/env python3
"""
Self-audit tests for the consensus test-rig (epoch committees + VRF sortition + async network).
No dependencies (plain asserts).
Run:  python3 testrig/tests/test_testrig.py   (from prototipos/consenso/)
"""

from __future__ import annotations

import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))

from testrig import scenarios
from testrig.engine import RigParams, run_epoch, run_epochs
from testrig.vrf import vrf, vrf_verify, elect_committee

P = RigParams()


def _sweep(f, loss=0.0, trials=12, seed0=200):
    safe = cap = stall = 0
    for t in range(trials):
        rng = random.Random(seed0 + t)
        rep, sk, adv = scenarios.population_reputation_fraction(f)
        r = run_epoch(rep, sk, adv, f"seed{t}", P, rng, loss=loss)
        safe += r["safe"]; cap += r["capture"]; stall += 1 if r["undecided"] > 0 else 0
    return safe, cap, stall, trials


def test_vrf_verifiable_and_grind_resistant():
    """A VRF output verifies against its (sk, seed); it cannot be reused for a different seed."""
    v, proof = vrf("sk-h0", "epoch0")
    assert vrf_verify("sk-h0", "epoch0", v, proof), "valid VRF must verify"
    assert not vrf_verify("sk-h0", "epoch1", v, proof), "VRF must not verify under a different seed"
    assert not vrf_verify("sk-h9", "epoch0", v, proof), "VRF must not verify under a different key"


def test_committee_is_reputation_weighted():
    """Committee size ~ tau and seats ~ tau (sortition is calibrated to the target size)."""
    rep, sk, _ = scenarios.population_reputation_fraction(0.0)
    c = elect_committee(rep, sk, "seed-x", P.tau)
    assert 0.5 * P.tau <= sum(c.values()) <= 1.5 * P.tau, f"seats {sum(c.values())} far from tau={P.tau}"
    assert len(c) >= 20, "committee should be large enough to sample k=20"


def test_sybil_excluded_from_committee():
    """A crowd of 2000 reputation~0 nodes wins essentially no committee seats (Art. VI)."""
    rep, sk, adv = scenarios.population_sybil()
    c = elect_committee(rep, sk, "seed-y", P.tau)
    sybils = [n for n in c if n.startswith("s")]
    assert len(sybils) == 0, f"sybils should not enter the committee, got {len(sybils)}"


def test_honest_epoch_is_safe_and_live():
    """No adversary, no loss: the committee finalises the legitimate value every time."""
    safe, cap, stall, trials = _sweep(0.0, trials=10)
    assert cap == 0, "honest network must never capture"
    assert safe == trials, f"honest network must finalise every trial (safe {safe}/{trials})"


def test_safety_below_threshold():
    """Adversary at 20% of the reputation: it may stall (liveness) but NEVER forces a false value."""
    _, cap, _, _ = _sweep(0.2, trials=12)
    assert cap == 0, f"adversary below threshold must not capture (cap={cap})"


def test_capture_above_threshold():
    """Adversary with a reputation majority (50%) CAN capture the committee — the threshold is real."""
    _, cap, _, _ = _sweep(0.5, trials=12)
    assert cap > 0, "a 50% reputation adversary should be able to capture"


def test_loss_degrades_liveness_not_safety():
    """Round-trip loss (both legs) stalls progress but never produces a false finalisation."""
    _, cap, stall, trials = _sweep(0.0, loss=0.35, trials=10)
    assert cap == 0, "loss must never cause capture (safety preserved)"
    assert stall > 0, "loss above the liveness bound should produce stalls"


def test_committee_rotates_across_epochs():
    """Consecutive epochs elect different committees (rotation / anti-entrenchment, Art. VI)."""
    rep, sk, _ = scenarios.population_reputation_fraction(0.0)
    c0 = set(elect_committee(rep, sk, "beacon|epoch0", P.tau))
    c1 = set(elect_committee(rep, sk, "beacon|epoch1", P.tau))
    overlap = len(c0 & c1) / max(1, len(c0 | c1))
    assert overlap < 0.8, f"committees should rotate, overlap was {overlap:.2f}"


def test_multi_epoch_honest_is_consistent():
    """Across many epochs the honest network stays safe (no epoch finalises a conflicting value)."""
    rng = random.Random(7)
    rep, sk, adv = scenarios.population_reputation_fraction(0.0)
    results = run_epochs(rep, sk, adv, n_epochs=6, params=P, rng=rng)
    assert all(r["capture"] == 0 for r in results), "no epoch may capture"
    assert sum(r["safe"] for r in results) >= 5, "almost every honest epoch should finalise"


def _partition_forks(quorum, until=150.0, trials=12, seed0=500):
    fork = 0
    for t in range(trials):
        rng = random.Random(seed0 + t)
        rep, sk, adv, grp = scenarios.population_partition(global_f=0.2)
        r = run_epoch(rep, sk, adv, f"p{t}", P, rng, group=grp,
                      partition_until=until, network_quorum=quorum)
        fork += r["fork"]
    return fork, trials


def test_partition_forks_without_mitigation():
    """A long partition concentrates a globally-harmless (20%) adversary's LOCAL share → fork."""
    fork, trials = _partition_forks(quorum=0.0)
    assert fork > trials // 2, f"the long partition should fork without mitigation (forks {fork}/{trials})"


def test_partition_quorum_prevents_fork():
    """Conditioning finality on a committee-reputation quorum halts the isolated group → no fork."""
    fork_no, trials = _partition_forks(quorum=0.0)
    fork_q, _ = _partition_forks(quorum=0.6)
    assert fork_q <= 1, f"the quorum should eliminate the fork (forks {fork_q}/{trials})"
    assert fork_q < fork_no, "the quorum must reduce forking vs no mitigation"


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
