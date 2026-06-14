"""
Building node populations for the consensus simulator.

Key scenarios:
- `population_reputation_fraction`: the adversary controls a FRACTION of the total reputation (few
  nodes, much reputation). Used to sweep the security threshold as a function of adversarial reputation.
- `population_clustered_adversary`: the adversary earns real reputation but keeps it all in ONE
  correlated trust cluster. Used to measure independence-weighted sampling.
- `population_sybil`: the adversary has MANY nodes but reputation ~0 (the fake crowd). Used to show
  that the number of nodes grants no power if reputation is null.
"""

from __future__ import annotations


def population_reputation_fraction(
    f: float,
    n_honest: int = 80,
    n_adversaries: int = 5,
) -> tuple[dict[str, float], set[str]]:
    """
    The adversary controls a fraction `f` (0..1) of the TOTAL reputation, spread over `n_adversaries`
    nodes. The honest nodes have reputation 1 each.
    """
    reputation: dict[str, float] = {}
    for i in range(n_honest):
        reputation[f"h{i}"] = 1.0
    honest_total = float(n_honest)

    adversaries: set[str] = set()
    if f > 0.0:
        # adversary share = f  =>  adv_total / (adv_total + honest_total) = f
        adv_total = f * honest_total / (1.0 - f)
        per_node = adv_total / n_adversaries
        for i in range(n_adversaries):
            aid = f"a{i}"
            reputation[aid] = per_node
            adversaries.add(aid)
    return reputation, adversaries


def population_clustered_adversary(
    f: float,
    n_honest: int = 80,
    n_adversaries: int = 12,
    honest_per_cluster: int = 2,
    n_adv_clusters: int = 1,
) -> tuple[dict[str, float], set[str], dict[str, str]]:
    """
    CORRELATED adversary: controls a fraction `f` of the total reputation, spread over `n_adv_clusters`
    trust clusters. With `n_adv_clusters=1` all of its reputation lives in ONE bloc (pure correlated
    case). Raising `n_adv_clusters` the adversary FRAGMENTS its bloc to evade the sampling's per-cluster
    cap (the honest frontier: fragmenting requires each sub-bloc to look like an independent cluster,
    exactly what the reputation engine — community detection + damping — is built to resist).

    The honest nodes are spread over many small independent clusters (`honest_per_cluster`).
    Returns (reputation, adversaries, clusters).
    """
    reputation: dict[str, float] = {}
    clusters: dict[str, str] = {}
    for i in range(n_honest):
        hid = f"h{i}"
        reputation[hid] = 1.0
        clusters[hid] = f"hc{i // max(1, honest_per_cluster)}"   # small honest clusters
    honest_total = float(n_honest)

    adversaries: set[str] = set()
    if f > 0.0:
        adv_total = f * honest_total / (1.0 - f)
        per_node = adv_total / n_adversaries
        ncl = max(1, n_adv_clusters)
        for i in range(n_adversaries):
            aid = f"a{i}"
            reputation[aid] = per_node
            clusters[aid] = f"adv{i % ncl}"   # spread over n_adv_clusters blocs
            adversaries.add(aid)
    return reputation, adversaries, clusters


def population_sybil(
    n_honest: int = 80,
    n_sybil: int = 1000,
    sybil_rep: float = 1e-6,
) -> tuple[dict[str, float], set[str]]:
    """
    Fake crowd: `n_sybil` adversarial nodes with reputation ~0 (born without reputation, §1.5),
    against `n_honest` honest nodes with reputation 1. The adversary is the GREAT MAJORITY of nodes.
    """
    reputation: dict[str, float] = {}
    for i in range(n_honest):
        reputation[f"h{i}"] = 1.0
    adversaries: set[str] = set()
    for i in range(n_sybil):
        sid = f"s{i}"
        reputation[sid] = sybil_rep
        adversaries.add(sid)
    return reputation, adversaries
