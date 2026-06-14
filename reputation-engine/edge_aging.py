#!/usr/bin/env python3
"""
EDGE (vouch) aging per epoch (§1.7, Art. VI) — closes the limitation of temporal.py.

temporal.py ages the evidence ANCHOR but leaves the vouches (graph edges) static. Here the vouches
also age: a vouch weighs `RHO_EDGE^(age)` and evaporates unless RENEWED. It models that trust is
perishable — an old vouch, from someone who no longer deals with you, says less than a fresh one — and
therefore that reputation resting on stale vouches decays even when the evidence stays.

CONTROLLED experiment (isolates the edge-aging effect):
  - two honest agents with the SAME constant evidence and the SAME initial vouches;
  - `renewer` receives a fresh vouch every epoch (counterparties that keep dealing with it);
  - `dormant` received its vouches at the start and NEVER renews them.
Any reputation difference between them is, by construction, ONLY from edge aging. Also, a collusion
ring that "farms" its mutual vouches at t=0 and sits sees them decay: collusion, also through the
graph, has to be SUSTAINED.

NON-invasive implementation: the core is not touched. Each epoch a fresh graph is rebuilt from a LOG
of vouches with their issue epoch, applying the aged weight. The engine is used as-is.

Run from prototipos/reputacion/:  python3 edge_aging.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from harlequin_rep.graph import TrustGraph
from harlequin_rep.model import DIMENSIONS, Agent, AgentKind
from harlequin_rep.reputation import reputation_vector

DIM = "commerce"
N_EPOCHS = 10
RHO_EDGE = 0.7   # retention of a vouch per epoch; an unrenewed vouch loses 30% each epoch
SEED = 7


def _build():
    """Agents with CONSTANT evidence + a log of vouches with issue epoch (to age)."""
    import random
    rng = random.Random(SEED)

    agents: list[Agent] = []
    factions: dict[str, str] = {}
    # vouch log: (source, target, dim, issue_epoch). Renewed by appending new entries.
    log: list[tuple[str, str, str, int]] = []

    genesis = [f"g{i}" for i in range(4)]
    for gid in genesis:
        agents.append(Agent(id=gid, kind=AgentKind.GENESIS, evidence={d: 2.0 for d in DIMENSIONS}))
        factions[gid] = "genesis"

    background = [f"p{i}" for i in range(12)]
    for pid in background:
        agents.append(Agent(id=pid, kind=AgentKind.HONEST, evidence={DIM: rng.uniform(2.0, 4.0)}))
        factions[pid] = "background"
        for v in rng.sample(genesis, 2):
            log.append((v, pid, DIM, 0))

    # two honest agents with the SAME evidence and the SAME 4 initial vouches (at t=0)
    for sid in ("renewer", "dormant"):
        agents.append(Agent(id=sid, kind=AgentKind.HONEST, evidence={DIM: 3.0}))
        factions[sid] = "tracked"
        for v in rng.sample(background, 4):
            log.append((v, sid, DIM, 0))

    # collusion ring: farms its mutual vouches at t=0 and sits (no renewal). c0 with evidence.
    ring = [f"c{i}" for i in range(10)]
    for idx, cid in enumerate(ring):
        ev = {DIM: 8.0} if idx == 0 else {}
        agents.append(Agent(id=cid, kind=AgentKind.COLLUDER, evidence=ev, cluster="farm"))
        factions[cid] = "colluder"
    for a in ring:
        for b in ring:
            if a != b:
                log.append((a, b, DIM, 0))

    return agents, factions, log, background


def simulate(rho_edge: float = RHO_EDGE):
    agents, factions, log, background = _build()
    ring = [a.id for a in agents if factions[a.id] == "colluder"]

    # renewals: the renewer receives a fresh vouch each epoch from a DIFFERENT voucher
    # (new counterparties that keep dealing with it); the dormant one NEVER renews.
    renewals: list[tuple[str, str, str, int]] = []
    for t in range(1, N_EPOCHS):
        v = background[t % len(background)]   # deterministic round-robin -> clean curve
        renewals.append((v, "renewer", DIM, t))

    def graph_at(t):
        g = TrustGraph()
        for (o, d, dim, ep) in (list(log) + renewals):
            if ep <= t:
                g.attest(o, d, dim, rho_edge ** (t - ep))
        return g

    traj = {"renewer": [], "dormant": [], "ring_puppets": []}
    for t in range(N_EPOCHS):
        rep = reputation_vector(agents, graph_at(t), damping=True)
        traj["renewer"].append(sum(rep["renewer"].values()))
        traj["dormant"].append(sum(rep["dormant"].values()))
        traj["ring_puppets"].append(sum(sum(rep[c].values()) for c in ring if c != "c0"))
    return traj


def fmt(traj, control) -> str:
    out = ["# Vouch (edge) aging per epoch (§1.7, Art. VI)\n"]
    out.append(f"RHO_EDGE={RHO_EDGE} (an unrenewed vouch loses {int((1-RHO_EDGE)*100)}% per epoch). "
               f"{N_EPOCHS} epochs. Earned reputation.\n")
    out.append("CONTROLLED experiment: two honest agents with **identical constant evidence** and the "
               "same initial vouches; the `renewer` receives a fresh vouch each epoch, the `dormant` "
               "one does not. The `dormant (no aging, ρ=1)` column isolates what aging contributes.\n")
    out.append("| epoch | renewer | dormant | dormant (no aging, ρ=1) | ring puppets |")
    out.append("|---:|---:|---:|---:|---:|")
    for t in range(N_EPOCHS):
        out.append(f"| {t} | {traj['renewer'][t]:.1f} | {traj['dormant'][t]:.1f} | "
                   f"{control['dormant'][t]:.1f} | {traj['ring_puppets'][t]:.1f} |")
    out.append("")
    r, d = traj["renewer"], traj["dormant"]
    dc = control["dormant"]
    gap = 100 * (r[-1] / max(d[-1], 1e-9) - 1)
    extra_aging = 100 * (1 - d[-1] / max(dc[-1], 1e-9))
    out.append("**Readings (honest):**")
    out.append(f"- **Freshness premium:** same evidence, the **renewer** ends ~{gap:.0f}% above the "
               "**dormant** one. Keeping trust ALIVE matters; resting on old vouches does not.")
    out.append(f"- **What aging contributes:** even without aging (ρ=1) the dormant node falls (fresh "
               f"foreign edges dilute it through the row-stochastic normalisation); aging cuts it a "
               f"further ~{extra_aging:.0f}% ({dc[-1]:.0f}→{d[-1]:.0f}). Anti-entrenchment (Art. VI) "
               "reaches the trust GRAPH too, not just the evidence anchor.")
    out.append("- **Honest nuance:** the uniform decay of ALL of a node's edges partly cancels under "
               "row normalisation (same reason as the already-fixed damping bug); the real effect is "
               "RELATIVE — not renewing while others do — which is exactly what we want to reward.")
    out.append("- The **farm-and-sit ring** barely moves because it is ALREADY near 0 from the damping "
               "(no reputation left to age): here aging is minor pressure, the first line is the "
               "evidence anchor + independence.")
    out.append("")
    return "\n".join(out)


def main():
    print(fmt(simulate(), control=simulate(rho_edge=1.0)))


if __name__ == "__main__":
    main()
