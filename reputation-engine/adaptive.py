#!/usr/bin/env python3
"""
Sweep of the ADAPTIVE collusion attack (open frontier §1.6).

Measures the central tension of the sophisticated attacker: to evade community detection it fragments
the ring into small, scattered sub-rings; but to LAUNDER the real reputation of its only node with
evidence (c0) onto the puppets, the reputation has to FLOW between fragments through a few bridges.
Hypothesis: it cannot have both — evade AND launder a lot.

We sweep `n_fragments` (1 = classic scattered ring ... up to heavily fragmented) and, for each point,
measure the TOTAL reputation captured by the 29 puppets (c1..c29, evidence 0), under three regimes:
  - no damping             (control: how much it would launder with no defence)
  - local damping          (edge independence, §1.6 first line)
  - damping + community     (global community defence, §1.6 third line)

It also reports how many communities the label detection "sees": if it grows with fragmentation, the
attacker DOES evade the label — but we must check whether the laundering rises or not.

Run from prototipos/reputacion/:  python3 adaptive.py
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from harlequin_rep.reputation import conservative_aggregate, reputation_vector
import scenarios


def _sum(vec: dict[str, float]) -> float:
    return sum(vec.values())


def _puppets(sc) -> list[str]:
    return [c for c, f in sc.factions.items() if f == "colluder" and c != "c0"]


def _honest(sc) -> list[str]:
    return [a.id for a in sc.agents if sc.factions[a.id] in ("honest", "genesis")]


def _n_ring_communities(sc) -> int:
    """How many distinct communities the detection labels WITHIN the collusion ring."""
    nodes = [a.id for a in sc.agents]
    label = sc.graph.communities("commerce", nodes)
    ring = [c for c, f in sc.factions.items() if f == "colluder"]
    return len(set(label[c] for c in ring))


def measure(sc):
    puppets, honest = _puppets(sc), _honest(sc)

    def laundered(damping, community):
        rep = reputation_vector(sc.agents, sc.graph, damping=damping, community=community)
        # gross laundering (sum over all dimensions) of the puppets
        gross = sum(_sum(rep[t]) for t in puppets)
        # real CONSENSUS POWER: conservative (min) aggregate of each puppet's vector (§1.2b). Since
        # they only launder in 'commerce' and have 0 in the other 3 dimensions, the min must collapse
        # to ~0.
        power = sum(conservative_aggregate(rep[t], "min") for t in puppets)
        hon = sum(_sum(rep[h]) for h in honest)
        return gross, power, hon

    none, _, _ = laundered(False, False)
    loc, _, hon_loc = laundered(True, False)
    com, power_com, hon_com = laundered(True, True)
    return {
        "none": none, "local": loc, "community": com,
        "power": power_com, "hon_community": hon_com,
        "n_com": _n_ring_communities(sc),
    }


def sweep(ring_size=30, fragments=(1, 2, 3, 5, 6, 10, 15), bridges=1, seed=7):
    rows = []
    for k in fragments:
        sc = scenarios.scenario_adaptive_collusion(
            seed=seed, ring_size=ring_size, n_fragments=k, bridges=bridges
        )
        rows.append((k, measure(sc)))
    return rows


def fmt(rows, bridges) -> str:
    out = []
    out.append(f"### Fragmentation sweep (ring=30, {bridges} bridge(s)/fragment pair)\n")
    out.append("TOTAL reputation captured by the 29 puppets (evidence 0). Lower = better defence.\n")
    out.append("| fragments | communities seen | laundered no damping | laundered local | laundered +community | **consensus power (min)** |")
    out.append("|---:|---:|---:|---:|---:|---:|")
    for k, m in rows:
        out.append(
            f"| {k} | {m['n_com']} | {m['none']:.1f} | {m['local']:.1f} | "
            f"{m['community']:.1f} | **{m['power']:.3f}** |"
        )
    out.append("")
    return "\n".join(out)


def main():
    print("# Adaptive collusion — fragmentation sweep\n")
    for bridges in (1, 2):
        print(fmt(sweep(bridges=bridges), bridges))


if __name__ == "__main__":
    main()
