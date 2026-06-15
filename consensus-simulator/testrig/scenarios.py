"""
Populations for the test-rig: like `wtc_sim/population.py` but each node also gets a VRF secret key
(needed for committee sortition). Reputation is the sortition + sampling weight.
"""

from __future__ import annotations


def _keys(reputation: dict[str, float]) -> dict[str, str]:
    """A deterministic per-node secret key (a simulated VRF sk). In production this is a real keypair."""
    return {n: f"sk-{n}" for n in reputation}


def population_reputation_fraction(
    f: float, n_honest: int = 120, n_adversaries: int = 6
) -> tuple[dict[str, float], dict[str, str], set[str]]:
    """Adversary controls a fraction `f` of total reputation over few nodes; honest have rep 1 each."""
    reputation: dict[str, float] = {f"h{i}": 1.0 for i in range(n_honest)}
    adversaries: set[str] = set()
    if f > 0.0:
        adv_total = f * n_honest / (1.0 - f)
        per = adv_total / n_adversaries
        for i in range(n_adversaries):
            aid = f"a{i}"
            reputation[aid] = per
            adversaries.add(aid)
    return reputation, _keys(reputation), adversaries


def population_partition(
    global_f: float = 0.2, n_honest: int = 120, n_adversaries: int = 8, group1_honest: int = 12
) -> tuple[dict[str, float], dict[str, str], set[str], dict[str, int]]:
    """
    Partition scenario: a GLOBALLY harmless adversary (fraction `global_f`) whose reputation is all in
    the small group 1, together with a few honest nodes. Globally f is low, but the LOCAL share inside
    group 1 is high → a long partition lets group 1 finalise the false value → fork on heal (unless the
    network-quorum mitigation halts it). Returns (reputation, keys, adversaries, group).
    """
    reputation: dict[str, float] = {f"h{i}": 1.0 for i in range(n_honest)}
    group: dict[str, int] = {f"h{i}": (1 if i < group1_honest else 0) for i in range(n_honest)}
    adversaries: set[str] = set()
    if global_f > 0.0:
        adv_total = global_f * n_honest / (1.0 - global_f)
        per = adv_total / n_adversaries
        for i in range(n_adversaries):
            aid = f"a{i}"
            reputation[aid] = per
            group[aid] = 1                 # the whole adversary lives in the small group
            adversaries.add(aid)
    return reputation, _keys(reputation), adversaries, group


def population_sybil(
    n_honest: int = 120, n_sybil: int = 2000, sybil_rep: float = 1e-6
) -> tuple[dict[str, float], dict[str, str], set[str]]:
    """Fake crowd: many adversarial nodes with reputation ~0 against honest nodes with rep 1."""
    reputation: dict[str, float] = {f"h{i}": 1.0 for i in range(n_honest)}
    adversaries: set[str] = set()
    for i in range(n_sybil):
        sid = f"s{i}"
        reputation[sid] = sybil_rep
        adversaries.add(sid)
    return reputation, _keys(reputation), adversaries
