"""
Computation of the vectorial reputation (the core, SPEC §1).

Central idea (§1.3, §1.5, §2.4): reputation is NOT manufactured just by vouching between accounts. It
is **anchored in real verifiable evidence** (settled deals, proven work) and propagated through the
vouch graph, but that propagation is (a) anchored to the pre-trust of evidence and (b) damped
anti-collusion (§1.6). Expected result:
  - Sybils with no evidence nor vouches from the reputed  -> reputation ~ 0  (power ~ 0).
  - A collusion ring vouching in a circle                 -> reputation ~ 0  (damping + no anchor).
  - Honest agents with evidence + independent vouches     -> high reputation.

Algorithm: EigenTrust (Kamvar et al. 2003) with teleport to the pre-trust.
    t_{k+1} = (1 - alpha) * C^T t_k  +  alpha * p
where:
  - C = row-stochastic local trust matrix, with anti-collusion damping (graph.py).
  - p = pre-trust = normalised objective evidence (+ genesis seed). It is the real ANCHOR.
  - alpha = weight of the anchor (how much is reinjected towards the evidence at each step).
  - the "dangling" mass (nodes vouching for nobody) is redistributed by p, not uniformly -> it
    reinforces the anchoring in evidence.
"""

from __future__ import annotations

from .graph import TrustGraph
from .model import DIMENSIONS, Agent


def _pretrust(agents: list[Agent], dim: str, genesis_weight: float = 1.0) -> dict[str, float]:
    """
    Pre-trust p per dimension: normalised objective evidence (§1.3a) + genesis seed (§1.4).

    The genesis cohort receives a small seed (temporary scaffolding, designed to dilute as the
    network grows). Everything else in the anchor comes from real evidence per dimension. If there
    were NO anchor at all, it falls back to uniform among unique humans (degenerate, avoids /0).
    """
    raw: dict[str, float] = {}
    for a in agents:
        ev = a.evidence_in(dim)
        seed = genesis_weight if a.kind.value == "genesis" else 0.0
        raw[a.id] = ev + seed

    total = sum(raw.values())
    if total <= 0:
        humans = [a for a in agents if a.unique_human]
        if not humans:
            return {a.id: 0.0 for a in agents}
        return {a.id: (1.0 / len(humans) if a.unique_human else 0.0) for a in agents}
    return {k: v / total for k, v in raw.items()}


def reputation_dimension(
    agents: list[Agent],
    graph: TrustGraph,
    dim: str,
    alpha: float = 0.30,
    iterations: int = 200,
    tol: float = 1e-12,
    scale: float = 1000.0,
    damping: bool = True,
    community: bool = False,
) -> dict[str, float]:
    """
    EARNED reputation (gate 2, §1.4) of each agent in one dimension.

    Returns a dict id -> reputation (scaled to `scale` for readability; the distribution is what
    matters, not the units). Does NOT include the base citizenship (§1.4), which is added separately
    where appropriate (the base is 1 per person and is not earned).
    """
    nodes = [a.id for a in agents]
    p = _pretrust(agents, dim)
    # TOTAL evidence per node (all dimensions), for the community suspicion (§1.6)
    total_evidence = {a.id: sum(a.evidence.values()) for a in agents}
    C = graph.damped_local_matrix(
        dim, nodes, damping=damping, community=community, evidence=total_evidence
    )

    # Sum of each row of C (≤ 1). The deficit (1 - sum) is the mass that does NOT propagate: either
    # the node vouches for nobody (dangling, sum 0) or its vouches are inbred and got damped (§1.6).
    # That mass is reinjected towards the pre-trust (anchoring in evidence). Keeps total mass = 1.
    row_sum = {i: sum(C[i].values()) for i in nodes}

    t = dict(p)  # start at the pre-trust
    for _ in range(iterations):
        nt = {n: alpha * p[n] for n in nodes}
        leak_total = 0.0

        for i in nodes:
            ti = t[i]
            if ti == 0.0:
                continue
            emitted = (1.0 - alpha) * ti
            row = C[i]
            for j, w in row.items():
                nt[j] += emitted * w
            # row deficit -> leaks to the pre-trust
            leak_total += emitted * (1.0 - row_sum[i])

        if leak_total:
            for n in nodes:
                nt[n] += leak_total * p[n]

        # convergence
        delta = sum(abs(nt[n] - t[n]) for n in nodes)
        t = nt
        if delta < tol:
            break

    return {k: v * scale for k, v in t.items()}


def reputation_vector(
    agents: list[Agent],
    graph: TrustGraph,
    **kwargs,
) -> dict[str, dict[str, float]]:
    """Earned reputation per agent, as a VECTOR over all dimensions (§1.2b)."""
    per_dim = {dim: reputation_dimension(agents, graph, dim, **kwargs) for dim in DIMENSIONS}
    out: dict[str, dict[str, float]] = {}
    for a in agents:
        out[a.id] = {dim: per_dim[dim][a.id] for dim in DIMENSIONS}
    return out


def conservative_aggregate(vector: dict[str, float], mode: str = "min") -> float:
    """
    CONSERVATIVE aggregation of the vector (§1.2b): minimums or medians, NEVER a sum.

    For powers that demand global reliability (consensus, vouching), a high dimension does NOT make
    up for a low one: you do not "buy" integrity with expertise. Default `min` (the most
    conservative).
    """
    vals = list(vector.values())
    if not vals:
        return 0.0
    if mode == "min":
        return min(vals)
    if mode == "median":
        from statistics import median

        return median(vals)
    if mode == "mean":
        return sum(vals) / len(vals)
    raise ValueError(f"unknown mode: {mode}")


def decay(reputation: dict[str, float], factor: float = 0.9) -> dict[str, float]:
    """
    Decay by inactivity (§1.7): uncontributed reputation evaporates.

    Simple per-epoch model: r <- r * factor for whoever did not contribute new evidence. Farming and
    then sitting still does not pay off long-term (extra anti-collusion defence, §1.6).
    """
    return {k: v * factor for k, v in reputation.items()}
