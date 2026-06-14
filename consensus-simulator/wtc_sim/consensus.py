"""
Consensus core: sub-sampled Snowball (Avalanche) voting with reputation-weighted sampling (SPEC
§2.2; PAPER §5.4).

Model (binary decision, enough to measure safety and forking):
- Honest nodes start preferring the legitimate value `0`.
- Adversarial nodes are Byzantine. Two strategies:
    * "fixed": always answer `1` (push a conflicting value).
    * "adaptive": each round they report the colour with the LEAST honest support, to keep honest
      nodes split and stop any colour from reaching the decision streak (LIVENESS / anti-finality
      attack). Worst case: they see the honest state of the moment.
- Each round, every undecided honest node queries a sample of `k` peers **weighted by reputation**; if
  a majority of at least `alpha` agrees on a colour, it reinforces its preference (Snowball); after
  `beta` consecutive rounds on the same colour it decides.

TWO contributions of WTC over plain Avalanche (both connect to the reputation engine):
1. Sampling is NOT uniform: a node enters with probability ∝ its reputation. A thousand identities
   with reputation ~0 almost never enter the sample (power is reputation, not number, Art. VI).
2. INDEPENDENCE-weighted sampling (PAPER §5.4): the committee is forced to be DIVERSE by limiting how
   many nodes a single trust cluster can contribute (`cap_cluster`). So an adversary that concentrated
   a lot of reputation in one correlated bloc CANNOT fill the sample: its influence is bounded by
   structure, not just by its reputation. Defends against correlated failures (a whole trust
   neighbourhood lying at once).
"""

from __future__ import annotations

import itertools
import random
from dataclasses import dataclass


@dataclass
class ConsensusParams:
    k: int = 20          # sample size per query
    alpha: int = 14      # quorum: minimum that must agree to count (alpha > k/2)
    beta: int = 12       # consecutive rounds on the same colour to decide
    max_rounds: int = 80


def _cum_weights(weights: list[float]) -> list[float]:
    return list(itertools.accumulate(weights))


def run_once(
    reputation: dict[str, float],
    adversaries: set[str],
    params: ConsensusParams,
    rng: random.Random,
    weighted: bool = True,
    clusters: dict[str, str] | None = None,
    cap_cluster: int | None = None,
    adversary: str = "fixed",
    group: dict[str, int] | None = None,
    partition_rounds: int = 0,
    network_quorum: float = 0.0,
    loss: float = 0.0,
) -> dict[str, int]:
    """
    One run of the consensus. Returns a tally of outcomes among the HONEST nodes:
      - decided_0: decided the legitimate value (correct)
      - decided_1: decided the adversary's value (captured)
      - undecided: did not converge within max_rounds
    Plus aggregate flags: safe (all 0), capture (some 1), fork (there is both 0 and 1).

    `cap_cluster`: if given (with `clusters`), no sample may contain more than `cap_cluster` nodes of
    the same cluster -> independence-weighted sampling (PAPER §5.4).
    `adversary`: "fixed" (always 1) or "adaptive" (reports the minority colour among the honest).
    `group` + `partition_rounds`: NETWORK partition. During the first `partition_rounds` rounds, each
    node can only sample within its own group (the network is split); afterwards it heals and samples
    the whole network. Measures safety (do the two sides decide differently?) and liveness (recovery?).
    `network_quorum`: anti-partition MITIGATION. A node does not FINALISE (decide) if the reputation
    it reaches is < `network_quorum` of the total. Under partition the isolated group never reaches
    quorum -> it does not finalise (stalls, no fork) -> recovers on heal. 0.0 = no mitigation (base).
    `loss`: probability that EACH queried response is lost (latency / network loss). Reduces the
    effective votes per round -> slower convergence (liveness cost), but the threshold alpha does not
    change -> safety preserved. 0.0 = reliable network (base behaviour).
    """
    ids = list(reputation)
    if weighted:
        w = [max(reputation[i], 0.0) for i in ids]
    else:
        w = [1.0 for _ in ids]  # uniform sampling (contrast: ignores reputation)
    cum = _cum_weights(w)

    use_cap = cap_cluster is not None and clusters is not None

    # per-group pools for the partition phase (ids + cum_weights restricted to each group)
    pools: dict[int, tuple[list[str], list[float]]] = {}
    group_rep: dict[int, float] = {}
    if group is not None and partition_rounds > 0:
        for g in set(group.values()):
            gids = [i for i in ids if group.get(i) == g]
            gw = [max(reputation[i], 0.0) if weighted else 1.0 for i in gids]
            pools[g] = (gids, _cum_weights(gw))
            group_rep[g] = sum(gw)
    total_rep = sum(max(reputation[i], 0.0) if weighted else 1.0 for i in ids)

    def reaches_quorum(node: str, rnd: int) -> bool:
        """Does the node see enough of the network's reputation to FINALISE? (partition mitigation)"""
        if network_quorum <= 0.0:
            return True
        visible = group_rep.get(group[node], total_rep) if (pools and rnd < partition_rounds) else total_rep
        return total_rep > 0 and visible / total_rep >= network_quorum

    def sample(node: str, rnd: int) -> list[str]:
        """k nodes weighted by reputation; with per-cluster cap and/or network partition if applicable."""
        # partition active: the node only sees its group
        if pools and rnd < partition_rounds:
            gids, gcum = pools[group[node]]
            base_ids, base_cum = gids, gcum
        else:
            base_ids, base_cum = ids, cum
        if not use_cap:
            return rng.choices(base_ids, cum_weights=base_cum, k=params.k)
        local_ids, local_cum = base_ids, base_cum
        picked: list[str] = []
        per_cluster: dict[str, int] = {}
        attempts = 0
        limit = params.k * 40  # anti-loop bound if there is not enough diversity
        while len(picked) < params.k and attempts < limit:
            attempts += 1
            cand = rng.choices(local_ids, cum_weights=local_cum, k=1)[0]
            cl = clusters.get(cand, cand)
            if per_cluster.get(cl, 0) >= cap_cluster:
                continue
            picked.append(cand)
            per_cluster[cl] = per_cluster.get(cl, 0) + 1
        # if diversity is not enough for k, fill the rest without the cap (do not penalise liveness)
        while len(picked) < params.k:
            picked.append(rng.choices(local_ids, cum_weights=local_cum, k=1)[0])
        return picked

    honest = [i for i in ids if i not in adversaries]
    pref = {i: 0 for i in honest}     # honest nodes start at the legitimate value
    streak = {i: 0 for i in honest}
    decision: dict[str, int] = {}

    adv_color = 1  # recomputed per round if the adversary is adaptive

    def report(i: str) -> int:
        if i in adversaries:
            return adv_color
        if i in decision:
            return decision[i]
        return pref[i]

    for rnd in range(params.max_rounds):
        if len(decision) == len(honest):
            break
        if adversary == "adaptive":
            # minority colour among the current honest preference -> keep the network split
            state = [decision.get(n, pref[n]) for n in honest]
            ones = sum(1 for c in state if c == 1)
            zeros = len(state) - ones
            adv_color = 1 if ones <= zeros else 0
        for n in honest:
            if n in decision:
                continue
            m = sample(n, rnd)
            if loss > 0.0:
                m = [s for s in m if rng.random() >= loss]   # responses that are lost
            ones = sum(1 for s in m if report(s) == 1)
            zeros = len(m) - ones
            color, count = (1, ones) if ones >= zeros else (0, zeros)
            if count >= params.alpha:
                if color == pref[n]:
                    streak[n] += 1
                else:
                    pref[n] = color
                    streak[n] = 1
                # finalise only with enough streak AND seeing a network quorum (anti-partition)
                if streak[n] >= params.beta and reaches_quorum(n, rnd):
                    decision[n] = color
            else:
                streak[n] = 0

    d0 = sum(1 for n in honest if decision.get(n) == 0)
    d1 = sum(1 for n in honest if decision.get(n) == 1)
    undecided = len(honest) - d0 - d1
    return {
        "decided_0": d0,
        "decided_1": d1,
        "undecided": undecided,
        "safe": int(d1 == 0 and undecided == 0),
        "capture": int(d1 > 0),
        "fork": int(d0 > 0 and d1 > 0),
    }
