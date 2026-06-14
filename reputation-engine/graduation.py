#!/usr/bin/env python3
"""
Protege graduation (§1.5c): sponsorship is SCAFFOLDING, not a permanent leash.

A newcomer (protege A) enters sponsored by a mentor M: at first its reputation leans on M's vouch
(the only one it has). As A does real work and receives INDEPENDENT vouches (from counterparties it
dealt with), its reputation comes to stand on its own. When A stands without M's vouch, it
**graduates**: the link stops being `live` and **frees M's vouch quota** (M can sponsor someone else).
The LIABILITY persists (cascade slashing still reaches M).

Measured each epoch by recomputing the engine:
  - total_rep(A)        : A's reputation WITH M's vouch present.
  - independent_rep(A)  : A's reputation REMOVING M's vouch (what A holds on its own).
  - graduation when independent_rep(A) ≥ threshold · total_rep(A): A no longer depends on the scaffold.
  - M's free quota = vouch_quota(rep(M)) − live_vouches(M): rises when A graduates.

Run from prototipos/reputacion/:  python3 graduation.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import random

from harlequin_rep.graph import TrustGraph
from harlequin_rep.model import DIMENSIONS, Agent, AgentKind
from harlequin_rep.reputation import reputation_dimension
from harlequin_rep.vouch import VouchRegistry, vouch_quota

DIM = "commerce"
N_EPOCHS = 8
GRADUATION_THRESHOLD = 0.6   # A graduates when ≥60% of its reputation stands WITHOUT the mentor vouch
RHO = 0.8


def _build():
    """Honest base + mentor M (with real work) + protege A (enters at zero) + counterparties."""
    rng = random.Random(7)
    agents: list[Agent] = []
    graph = TrustGraph()

    genesis = [f"g{i}" for i in range(4)]
    for gid in genesis:
        agents.append(Agent(id=gid, kind=AgentKind.GENESIS, evidence={d: 2.0 for d in DIMENSIONS}))

    # independent counterparties (will deal with A and vouch for it as time passes)
    counterparties = [f"p{i}" for i in range(8)]
    for pid in counterparties:
        agents.append(Agent(id=pid, kind=AgentKind.HONEST, evidence={DIM: rng.uniform(3.0, 6.0)}))
        for v in rng.sample(genesis, 2):
            graph.attest(v, pid, DIM, 1.0)

    # mentor: consolidated real work
    agents.append(Agent(id="M", kind=AgentKind.HONEST, evidence={DIM: 12.0}))
    for v in rng.sample(genesis + counterparties, 4):
        graph.attest(v, "M", DIM, 1.0)

    # protege: enters at zero, only with the mentor vouch (gate 1 = base citizenship)
    agents.append(Agent(id="A", kind=AgentKind.HONEST, evidence={}))

    reg = VouchRegistry()
    reg.sponsor_link("M", "A")
    graph.attest("M", "A", DIM, 1.0)   # the mentor vouch (scaffolding)

    return agents, graph, reg, counterparties


def _stream_protege(t: int) -> float:
    """Evidence (real work) A accumulates per epoch: starts small and grows (settles in)."""
    return 2.0 * t   # 0, 2, 4, 6, ... growing work


def simulate():
    agents, graph, reg, counterparties = _build()
    base_ev = {a.id: dict(a.evidence) for a in agents}
    A = next(a for a in agents if a.id == "A")

    traj = {"total": [], "independent": [], "M_free_quota": [], "graduated": []}
    graduated = False

    for t in range(N_EPOCHS):
        # A accumulates aged evidence + receives new independent vouches as it grows
        acc = 0.0
        for s in range(t + 1):
            acc += _stream_protege(s) * (RHO ** (t - s))
        A.evidence = {DIM: acc} if acc > 0 else {}
        # each epoch a new counterparty that dealt with A vouches for it (independent vouch, not the mentor)
        if 0 < t <= len(counterparties):
            graph.attest(counterparties[t - 1], "A", DIM, 1.0)

        # rep WITH the mentor vouch vs WITHOUT it (what A holds on its own)
        total_rep = reputation_dimension(agents, graph, DIM)
        # temporarily remove the mentor vouch to measure A's independent reputation
        m_weight = graph.outgoing("M", DIM).get("A", 0.0)
        graph._edges[DIM]["M"].pop("A", None)
        independent_rep = reputation_dimension(agents, graph, DIM)
        if m_weight:                      # restore the vouch
            graph._edges[DIM]["M"]["A"] = m_weight

        rt, ri = total_rep["A"], independent_rep["A"]
        # graduation: A stands without the scaffold
        if not graduated and rt > 0 and ri >= GRADUATION_THRESHOLD * rt:
            reg.graduate("M", "A")
            graduated = True

        free_quota = vouch_quota(total_rep["M"]) - reg.live_vouches("M")

        traj["total"].append(rt)
        traj["independent"].append(ri)
        traj["M_free_quota"].append(free_quota)
        traj["graduated"].append(graduated)

    return traj


def fmt(traj) -> str:
    out = ["# Protege graduation (§1.5c): sponsorship is scaffolding, not a leash\n"]
    out.append(f"ρ={RHO}, graduation threshold = {int(GRADUATION_THRESHOLD*100)}% independent reputation. "
               f"{N_EPOCHS} epochs.\n")
    out.append("| epoch | A total rep | A independent rep | % independent | M free quota | graduated? |")
    out.append("|---:|---:|---:|---:|---:|:--:|")
    for t in range(N_EPOCHS):
        rt = traj["total"][t]; ri = traj["independent"][t]
        pct = 100 * ri / rt if rt else 0.0
        grad = "yes" if traj["graduated"][t] else "—"
        out.append(f"| {t} | {rt:.1f} | {ri:.1f} | {pct:.0f}% | {traj['M_free_quota'][t]} | {grad} |")
    out.append("")
    ep_grad = next((t for t in range(N_EPOCHS) if traj["graduated"][t]), None)
    out.append("**Readings:**")
    out.append("- At first A depends on the mentor vouch: its **independent** reputation is a small "
               "fraction of the total (the scaffolding holds the rest).")
    if ep_grad is not None:
        out.append(f"- At **epoch {ep_grad}**, A stands on its own (≥{int(GRADUATION_THRESHOLD*100)}% "
                   "independent) → it **graduates**: the mentor vouch is released and stops taking up its "
                   f"quota (M's free quota rises from {traj['M_free_quota'][ep_grad-1]} to {traj['M_free_quota'][ep_grad]}).")
    out.append("- After graduating, A keeps its reputation through its own work; M recovers capacity to "
               "sponsor someone else. The **liability persists** (the link stays registered: cascade "
               "slashing would still reach M if A defrauded).")
    out.append("- Incentive moral: sponsoring well is investing in the protege **growing and becoming "
               "independent**, not in tethering it. The scaffolding is designed to dilute (consistent "
               "with the genesis seed and anti-entrenchment, Art. VI).")
    out.append("")
    return "\n".join(out)


def main():
    print(fmt(simulate()))


if __name__ == "__main__":
    main()
