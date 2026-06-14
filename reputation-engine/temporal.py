#!/usr/bin/env python3
"""
TEMPORAL dynamics of the reputation engine (§1.7 decay, Art. VI anti-entrenchment).

The base engine is static: it computes reputation from a fixed snapshot. But SPEC §1.7 says
uncontributed reputation **evaporates**, and the manifesto (Art. VI) forbids entrenchment: yesterday's
power cannot shield tomorrow's. Here we model TIME in epochs.

Decay implementation (the most principled one): we age the ANCHOR, not a separate marker. Reputation
is anchored in evidence (pre-trust); OLD evidence loses weight exponentially. At epoch t, an agent's
effective evidence is

    anchor_t[dim] = Σ_{s≤t}  raw_evidence_s[dim] · ρ^(t−s)         (ρ = retention per epoch)

So recomputing EigenTrust epoch by epoch yields a reputation that:
  - rises and holds if you KEEP contributing (active honest),
  - rises and then DECAYS if you stop contributing (retired honest) -> anti-entrenchment (Art. VI),
  - a pioneer with a single great work who then sleeps does NOT keep power forever
    -> FREE anti-long-range defence (an old history cannot be reactivated into power),
  - a collusion farm that farms and sits deflates -> collusion has to be SUSTAINED.

Honest limitation: vouches (the graph) are modelled static; only evidence ages. A future iteration
can also age the edges and graduate proteges per epoch (see edge_aging.py / graduation.py).

Run from prototipos/reputacion/:  python3 temporal.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import random

from harlequin_rep.graph import TrustGraph
from harlequin_rep.model import DIMENSIONS, Agent, AgentKind
from harlequin_rep.reputation import reputation_vector

RHO = 0.7          # evidence retention per epoch (decay §1.7); 1−ρ evaporates
N_EPOCHS = 10
SEED = 7

# tracked profiles (id -> (readable label, raw_evidence(epoch) -> {dim: amount}))
DIM_A, DIM_B = "commerce", "technical_contribution"


def _stream_active(t: int) -> dict[str, float]:
    """Contributes real work every epoch in 2 dimensions."""
    return {DIM_A: 4.0, DIM_B: 3.0}


def _stream_retired(t: int) -> dict[str, float]:
    """Contributes strongly in epochs 0-2, then retires (stops contributing)."""
    return {DIM_A: 5.0, DIM_B: 4.0} if t <= 2 else {}


def _stream_pioneer(t: int) -> dict[str, float]:
    """A single great work at epoch 0 (pioneer) and then silence (long-range case)."""
    return {DIM_A: 30.0, DIM_B: 25.0} if t == 0 else {}


def _stream_farm(t: int) -> dict[str, float]:
    """Collusion farm: the anchor (c0) farms hard in epochs 0-1, then sits to recirculate."""
    return {DIM_A: 20.0} if t <= 1 else {}


PROFILES = {
    "active_honest":   ("Active honest (always contributes)", _stream_active),
    "retired_honest":  ("Retired honest (contributes, stops at t=3)", _stream_retired),
    "sleeping_pioneer": ("Pioneer (single great work at t=0, then sleeps)", _stream_pioneer),
}


def _build(rng: random.Random):
    """Honest base network + the tracked profile agents + a collusion ring that farms-and-sits."""
    agents: list[Agent] = []
    factions: dict[str, str] = {}
    graph = TrustGraph()

    # genesis (diluting seed) + honest background fill
    genesis = [f"g{i}" for i in range(5)]
    for gid in genesis:
        agents.append(Agent(id=gid, kind=AgentKind.GENESIS, evidence={d: 2.0 for d in DIMENSIONS}))
        factions[gid] = "genesis"
    background = [f"h{i}" for i in range(20)]
    for hid in background:
        agents.append(Agent(id=hid, kind=AgentKind.HONEST, evidence={}))
        factions[hid] = "background"

    # tracked profile agents (their evidence is injected per epoch; here they start with none)
    tracked = list(PROFILES)
    for sid in tracked:
        agents.append(Agent(id=sid, kind=AgentKind.HONEST, evidence={}))
        factions[sid] = "tracked"

    # each tracked agent receives independent vouches from the background (real trust, low inbreeding)
    established = genesis + background
    for sid in tracked:
        for v in rng.sample(established, 4):
            for d in (DIM_A, DIM_B):
                graph.attest(v, sid, d, 1.0)
    # the background also gets some vouches so it is not at zero (network texture)
    for hid in background:
        for v in rng.sample([e for e in established if e != hid], 2):
            graph.attest(v, hid, DIM_A, 1.0)

    # farm-and-sit collusion ring: c0 with per-epoch evidence (stream_farm), c1..c9 puppets
    ring = [f"c{i}" for i in range(10)]
    for cid in ring:
        agents.append(Agent(id=cid, kind=AgentKind.COLLUDER, evidence={}, cluster="farm"))
        factions[cid] = "colluder"
    for a in ring:               # reciprocal clique (farm)
        for b in ring:
            if a != b:
                graph.attest(a, b, DIM_A, 1.0)

    return agents, graph, factions, tracked, ring


def simulate():
    rng = random.Random(SEED)
    agents, graph, factions, tracked, ring = _build(rng)

    # raw evidence history per epoch for the agents with a stream
    streams = {sid: PROFILES[sid][1] for sid in tracked}
    streams["c0"] = _stream_farm   # the ring's anchor

    base = {a.id: dict(a.evidence) for a in agents}  # static evidence (genesis)

    trajectory: dict[str, list[float]] = {sid: [] for sid in tracked}
    trajectory["ring_puppets"] = []   # sum of c1..c9

    for t in range(N_EPOCHS):
        # aged anchor: Σ_{s≤t} raw_s · ρ^(t−s)
        for a in agents:
            aged: dict[str, float] = dict(base[a.id])  # static evidence does not age (genesis)
            if a.id in streams:
                acc: dict[str, float] = {}
                for s in range(t + 1):
                    raw = streams[a.id](s)
                    w = RHO ** (t - s)
                    for d, v in raw.items():
                        acc[d] = acc.get(d, 0.0) + v * w
                for d, v in acc.items():
                    aged[d] = aged.get(d, 0.0) + v
            a.evidence = aged

        rep = reputation_vector(agents, graph, damping=True)
        for sid in tracked:
            trajectory[sid].append(sum(rep[sid].values()))
        trajectory["ring_puppets"].append(sum(sum(rep[c].values()) for c in ring if c != "c0"))

    return trajectory


def fmt(traj) -> str:
    out = ["# Temporal dynamics of the engine (§1.7 decay, Art. VI anti-entrenchment)\n"]
    out.append(f"ρ = {RHO} (evidence retention per epoch), {N_EPOCHS} epochs. Earned reputation "
               "(sum of the vector) per epoch.\n")
    labels = {
        "active_honest": "Active honest",
        "retired_honest": "Retired honest (stops at t=3)",
        "sleeping_pioneer": "Sleeping pioneer (single work at t=0)",
        "ring_puppets": "Farm-and-sit ring puppets",
    }
    header = "| epoch | " + " | ".join(labels[k] for k in labels) + " |"
    out.append(header)
    out.append("|---:|" + "---:|" * len(labels))
    for t in range(N_EPOCHS):
        cells = " | ".join(f"{traj[k][t]:.1f}" for k in labels)
        out.append(f"| {t} | {cells} |")
    out.append("")
    # automatic readings
    active = traj["active_honest"]
    ret = traj["retired_honest"]
    pio = traj["sleeping_pioneer"]
    peak_ret = max(ret)
    drop_ret = 100.0 * (1.0 - ret[-1] / peak_ret) if peak_ret else 0.0
    drop_pio = 100.0 * (1.0 - pio[-1] / max(pio)) if max(pio) else 0.0
    out.append("**Readings:**")
    out.append(f"- **Active** honest: holds (t0={active[0]:.0f} → t{N_EPOCHS-1}={active[-1]:.0f}). "
               "Contributing keeps the power.")
    out.append(f"- **Retired** honest (stops contributing at t=3): peak {peak_ret:.0f} → "
               f"{ret[-1]:.0f} ({drop_ret:.0f}% less). **Anti-entrenchment (Art. VI)**: yesterday's "
               "power is not kept without new work.")
    out.append(f"- **Sleeping pioneer** (one great work at t=0, then nothing): {max(pio):.0f} → "
               f"{pio[-1]:.0f} ({drop_pio:.0f}% less). **Free anti-long-range**: an old history cannot "
               "be reactivated into present power.")
    out.append(f"- **Collusion farm** that farms (t≤1) and sits: the puppets deflate "
               f"({traj['ring_puppets'][1]:.0f} at t=1 → {traj['ring_puppets'][-1]:.0f} at the end). "
               "Collusion has to be SUSTAINED over time, not a sprint.")
    out.append("")
    return "\n".join(out)


def main():
    print(fmt(simulate()))


if __name__ == "__main__":
    main()
