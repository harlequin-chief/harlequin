"""
Woven-Trust Consensus test-rig engine: epoch committees (VRF sortition by reputation, `vrf.py`) running
sub-sampled Snowball voting over an ASYNCHRONOUS, lossy network (`network.py`), with committee ROTATION
per epoch. This is the faithful version of `wtc_sim/`:

  wtc_sim (validated)         ->   testrig (this)
  ------------------------         ----------------------------------------
  every node votes every round     only the elected COMMITTEE votes (SPEC §2.2)
  committee fixed/implicit         committee re-elected by VRF each epoch (rotation, Art. VI)
  synchronous lockstep rounds      async messages with latency + loss (real-ish network)
  reputation = sample weight       reputation = sortition weight AND sample weight

What it lets us measure that the simulator could not: does consensus still hold when the committee is
a reputation-elected subset, when it rotates every epoch, and when the network is asynchronous?
"""

from __future__ import annotations

import random
from dataclasses import dataclass

from .network import Network
from .vrf import elect_committee


@dataclass
class RigParams:
    k: int = 20            # query sample size
    alpha: int = 14        # quorum among RECEIVED responses (> k/2)
    beta: int = 12         # consecutive quorums to finalise
    tau: float = 60.0      # target committee size (expected seats)
    window: float = 8.0    # time to collect responses before evaluating a round
    gap: float = 1.0       # pause between a node's query rounds
    max_rounds: int = 60   # per-epoch round budget (liveness timeout)


class _Voter:
    """A committee member's voting state for one epoch."""

    __slots__ = ("nid", "seats", "is_adv", "pref", "streak", "decided")

    def __init__(self, nid: str, seats: int, is_adv: bool) -> None:
        self.nid = nid
        self.seats = seats
        self.is_adv = is_adv
        self.pref = 0          # honest start at the legitimate value 0
        self.streak = 0
        self.decided: int | None = None

    def report(self, adv_color: int) -> int:
        if self.is_adv:
            return adv_color
        return self.decided if self.decided is not None else self.pref


def run_epoch(
    reputation: dict[str, float],
    secret_keys: dict[str, str],
    adversaries: set[str],
    seed: str,
    params: RigParams,
    rng: random.Random,
    loss: float = 0.0,
    adv_color: int = 1,
    group: dict[str, int] | None = None,
    partition_until: float = 0.0,
    network_quorum: float = 0.0,
) -> dict:
    """
    Elect the epoch committee by VRF sortition, then run async sub-sampled voting until the honest
    committee decides or the round budget runs out. Returns committee + outcome tally.

    PARTITION (faithful version of `partition.py`): if `group` is given, while `net.now <
    partition_until` a node can only sample committee members IN ITS OWN GROUP (the network is split);
    afterwards it heals and samples the whole committee. A long partition concentrates the adversary's
    LOCAL share in a small group → it can finalise the false value there → FORK on heal.
    MITIGATION `network_quorum`: a node finalises only if the committee reputation it can reach is
    ≥ network_quorum of the total committee reputation. Under partition the isolated group never
    reaches quorum → it STALLS instead of forking, and recovers on heal (safety over liveness).
    """
    committee = elect_committee(reputation, secret_keys, seed, params.tau)
    voters = {n: _Voter(n, s, n in adversaries) for n, s in committee.items()}
    ids = list(voters)
    seats = [voters[n].seats for n in ids]
    honest = [n for n in ids if not voters[n].is_adv]

    if not honest:
        return _tally(committee, voters, adversaries, decided_ok=0)

    # per-group committee pools (for sampling) and reputation (for the quorum gate)
    total_rep = sum(max(reputation[n], 0.0) for n in ids)
    pools: dict[int, tuple[list[str], list[int]]] = {}
    group_rep: dict[int, float] = {}
    if group is not None:
        for g in set(group.get(n, -1) for n in ids):
            gids = [n for n in ids if group.get(n) == g]
            pools[g] = (gids, [voters[n].seats for n in gids])
            group_rep[g] = sum(max(reputation[n], 0.0) for n in gids)

    net = Network(rng, loss=loss)
    rounds_done = {n: 0 for n in honest}

    def partition_active() -> bool:
        return group is not None and net.now < partition_until

    def visible_frac_now(n: str) -> float:
        """Fraction of committee reputation a node can reach RIGHT NOW (depends on partition state)."""
        if total_rep <= 0.0:
            return 0.0
        visible = group_rep.get(group[n], total_rep) if partition_active() else total_rep
        return visible / total_rep

    def query_round(n: str) -> None:
        v = voters[n]
        if v.decided is not None or rounds_done[n] >= params.max_rounds:
            return
        rounds_done[n] += 1
        responses: list[int] = []
        if partition_active():
            pool_ids, pool_seats = pools[group[n]]
        else:
            pool_ids, pool_seats = ids, seats
        # quorum is tied to the PROVENANCE of this sample: a sample drawn while partitioned carries the
        # group's reachable fraction even if it is evaluated after the heal (no in-flight finalisation).
        visible = visible_frac_now(n)
        targets = rng.choices(pool_ids, weights=pool_seats, k=params.k)
        for t in targets:
            def on_query(t=t) -> None:
                color = voters[t].report(adv_color)
                net.send(lambda color=color: responses.append(color))
            net.send(on_query)
        net.schedule(params.window, lambda: evaluate(n, responses, visible))

    def evaluate(n: str, responses: list[int], visible: float) -> None:
        v = voters[n]
        if v.decided is not None:
            return
        ones = sum(1 for c in responses if c == 1)
        zeros = len(responses) - ones
        color, count = (1, ones) if ones >= zeros else (0, zeros)
        if count >= params.alpha:                 # quorum among RECEIVED responses
            if color == v.pref:
                v.streak += 1
            else:
                v.pref = color
                v.streak = 1
            # finalise only with the streak AND if this sample reached a committee-reputation quorum
            if v.streak >= params.beta and (network_quorum <= 0.0 or visible >= network_quorum):
                v.decided = color
                return
        else:
            v.streak = 0                           # lost/late responses -> no quorum -> reset (liveness)
        net.schedule(params.gap, lambda: query_round(n))

    # stagger the honest nodes' first queries slightly (async start)
    for i, n in enumerate(honest):
        net.schedule(rng.uniform(0.0, 1.0), lambda n=n: query_round(n))

    t_end = params.max_rounds * (params.window + params.gap) + 10.0
    net.run_until(t_end, stop=lambda: all(voters[n].decided is not None for n in honest))

    res = _tally(committee, voters, adversaries, decided_ok=None)
    res["messages"] = net.sent
    res["dropped"] = net.dropped
    return res


def _tally(committee, voters, adversaries, decided_ok) -> dict:
    honest = [n for n in committee if n not in adversaries]
    d0 = sum(1 for n in honest if voters[n].decided == 0)
    d1 = sum(1 for n in honest if voters[n].decided == 1)
    undecided = len(honest) - d0 - d1
    return {
        "committee_size": len(committee),
        "committee_seats": sum(committee.values()),
        "honest_in_committee": len(honest),
        "adv_in_committee": len(committee) - len(honest),
        "decided_0": d0,
        "decided_1": d1,
        "undecided": undecided,
        "safe": int(d1 == 0 and undecided == 0),
        "capture": int(d1 > 0),
        "fork": int(d0 > 0 and d1 > 0),
    }


def run_epochs(
    reputation: dict[str, float],
    secret_keys: dict[str, str],
    adversaries: set[str],
    n_epochs: int,
    params: RigParams,
    rng: random.Random,
    loss: float = 0.0,
    beacon: str = "genesis",
) -> list[dict]:
    """
    Run `n_epochs` consecutive epochs, each with a fresh committee (the beacon chains forward, so the
    committee ROTATES). Returns the per-epoch results. Lets us measure rotation/anti-entrenchment and
    that finalisation stays consistent (safe) across epochs.
    """
    out = []
    seed = beacon
    for e in range(n_epochs):
        epoch_seed = f"{seed}|epoch{e}"
        r = run_epoch(reputation, secret_keys, adversaries, epoch_seed, params, rng, loss=loss)
        r["epoch"] = e
        r["committee"] = set(_committee_ids(reputation, secret_keys, epoch_seed, params))
        out.append(r)
        seed = epoch_seed  # chain the beacon forward
    return out


def _committee_ids(reputation, secret_keys, seed, params) -> list[str]:
    return list(elect_committee(reputation, secret_keys, seed, params.tau).keys())
