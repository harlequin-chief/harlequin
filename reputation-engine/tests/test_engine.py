#!/usr/bin/env python3
"""
Self-audit tests for the reputation engine. No dependencies (no pytest): plain asserts.

Run:
    python3 tests/test_engine.py     # from prototipos/reputacion/

Verifies the properties that hold up the report's conclusions, so they are not "looks like it works"
but checkable.
"""

from __future__ import annotations

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from harlequin_rep.graph import TrustGraph
from harlequin_rep.model import DIMENSIONS
from harlequin_rep.reputation import (
    conservative_aggregate,
    reputation_dimension,
    reputation_vector,
)
from harlequin_rep.vouch import VouchRegistry, vouch_quota, cascade_slashing
import scenarios


def _sum(vec):
    return sum(vec.values())


def test_mass_conservation():
    """EigenTrust with leak to the pre-trust must conserve mass: each dimension sums to ~scale."""
    sc = scenarios.scenario_collusion()
    for dim in DIMENSIONS:
        rep = reputation_dimension(sc.agents, sc.graph, dim, scale=1000.0)
        total = sum(rep.values())
        assert abs(total - 1000.0) < 1.0, f"mass not conserved in {dim}: {total}"


def test_sybil_has_no_power():
    """200 sybils with no evidence nor vouches from the reputed -> reputation ~ 0."""
    sc = scenarios.scenario_sybil()
    rep = reputation_vector(sc.agents, sc.graph)
    sybils = [aid for aid, f in sc.factions.items() if f == "sybil"]
    sybil_share = sum(_sum(rep[s]) for s in sybils)
    total = sum(_sum(v) for v in rep.values())
    assert sybil_share / total < 0.001, f"sybils capture too much: {sybil_share/total:.4f}"


def test_damping_reduces_laundering():
    """The damping (§1.6) reduces the worker c0's reputation laundered to its cronies."""
    sc = scenarios.scenario_collusion()
    on = reputation_vector(sc.agents, sc.graph, damping=True)
    off = reputation_vector(sc.agents, sc.graph, damping=False)
    cronies = [c for c, f in sc.factions.items() if f == "colluder" and c != "c0"]
    laundered_on = sum(_sum(on[s]) for s in cronies)
    laundered_off = sum(_sum(off[s]) for s in cronies)
    assert laundered_on < laundered_off, "the damping does not reduce laundering"
    assert laundered_off / max(laundered_on, 1e-9) > 3.0, "the damping contributes less than 3×"


def test_community_reduces_scattered_ring():
    """Community detection reduces the SCATTERED ring's laundering (frontier §1.6) without harming honest ones."""
    sc = scenarios.scenario_scattered_collusion()
    cronies = [c for c, f in sc.factions.items() if f == "colluder" and c != "c0"]
    honest = [a.id for a in sc.agents if sc.factions[a.id] in ("honest", "genesis")]

    def measure(community):
        rep = reputation_vector(sc.agents, sc.graph, damping=True, community=community)
        laundered = sum(_sum(rep[s]) for s in cronies)
        hon = sum(_sum(rep[h]) for h in honest)
        return laundered, hon

    lau_local, hon_local = measure(False)
    lau_comm, hon_comm = measure(True)
    assert lau_comm < lau_local * 0.8, "the community defence should reduce scattered laundering"
    assert hon_comm > hon_local * 0.9, "the community defence should NOT punish honest agents"


def test_adaptive_community_does_not_worsen():
    """
    ADAPTIVE collusion (fragment to evade the community label): at any fragmentation, the community
    defence NEVER launders more than the local one alone and ALWAYS reduces vs no damping. Evading the
    label buys the attacker no extra laundered reputation.
    """
    puppets_of = lambda sc: [c for c, f in sc.factions.items() if f == "colluder" and c != "c0"]
    for k in (1, 3, 6, 10):
        sc = scenarios.scenario_adaptive_collusion(n_fragments=k)
        tit = puppets_of(sc)
        off = sum(_sum(reputation_vector(sc.agents, sc.graph, damping=False)[t]) for t in tit)
        loc = sum(_sum(reputation_vector(sc.agents, sc.graph, damping=True)[t]) for t in tit)
        com = sum(_sum(reputation_vector(sc.agents, sc.graph, damping=True, community=True)[t]) for t in tit)
        assert com <= loc + 1e-6, f"k={k}: community ({com:.1f}) launders more than local ({loc:.1f})"
        assert com < off, f"k={k}: the defence does not reduce vs no damping"


def test_adaptive_has_no_consensus_power():
    """
    Adaptive laundering is SINGLE-DIMENSION (only 'commerce'): under the conservative aggregate (min,
    §1.2b) that governs consensus/vouch power, the puppets collapse to ~0 at any fragmentation.
    Leaking diffuse reputation in one dimension does NOT buy power that demands global reliability.
    """
    for k in (1, 3, 10):
        sc = scenarios.scenario_adaptive_collusion(n_fragments=k)
        tit = [c for c, f in sc.factions.items() if f == "colluder" and c != "c0"]
        rep = reputation_vector(sc.agents, sc.graph, damping=True, community=True)
        power = sum(conservative_aggregate(rep[t], "min") for t in tit)
        assert power < 0.01, f"k={k}: puppets capture consensus power {power:.4f} (> 0)"


def test_asymmetric_evades_damping_but_no_power():
    """
    ASYMMETRIC collusion (PageRank funnel, frontier §1.6): honest finding in two parts.
    (1) The funnel EVADES the local damping: since the target c0 vouches for nobody, neighbour overlap
        is 0 and reciprocity is 0 → independence(feeder→c0)=1 → the damping does NOT cut the pump (c0
        keeps most of its inflated reputation). A real gap of the graph damping.
    (2) But the pump is SINGLE-DIMENSION (only 'commerce') → under the conservative aggregate (min,
        §1.2b) c0's consensus power collapses to ~0. The vectorial aggregate is the backstop that holds.
    """
    sc = scenarios.scenario_asymmetric_collusion(diversify=0)
    on = reputation_vector(sc.agents, sc.graph, damping=True)
    off = reputation_vector(sc.agents, sc.graph, damping=False)
    # (1) local damping does NOT crush the funnel (documents the gap): c0 with damping is not << without
    assert _sum(on["c0"]) > 0.7 * _sum(off["c0"]), "the funnel should NOT be crushed by the local damping"
    # (2) but the consensus power (vectorial min) is ~0: conservative-aggregate backstop
    assert conservative_aggregate(on["c0"], "min") < 0.01, "the single-dimension pump should give no consensus power"


def test_in_concentration_closes_funnel():
    """
    ASYMMETRIC funnel, gap CLOSED at graph level (frontier §2d). The in-degree-concentration signal
    cuts the directed PageRank funnel that local independence and community suspicion miss, WITHOUT
    punishing legitimately popular honest members.

    Mechanism: a funnel concentrates a 0-evidence target's inflow into one (star-bound) community; the
    signal weights concentration by the target's evidence DEFICIT, so honest members (who earned their
    own evidence) are spared. Robust to feeder diversification (the star binds the feeders together).
    """
    for div in (0, 3, 6):
        sc = scenarios.scenario_asymmetric_collusion(diversify=div)
        com = reputation_vector(sc.agents, sc.graph, damping=True, community=True)
        inc = reputation_vector(sc.agents, sc.graph, damping=True, community=True, in_concentration=True)
        # the funnel target loses at least half its laundered reputation at every diversification
        assert _sum(inc["c0"]) < 0.5 * _sum(com["c0"]), f"div={div}: funnel not cut by in-concentration"

    # false-positive guard: honest members keep ~all their reputation (worst case > 0.85)
    sc = scenarios.scenario_asymmetric_collusion(diversify=0)
    com = reputation_vector(sc.agents, sc.graph, damping=True, community=True)
    inc = reputation_vector(sc.agents, sc.graph, damping=True, community=True, in_concentration=True)
    honest = [a.id for a in sc.agents if sc.factions[a.id] == "honest"]
    worst = min((_sum(inc[h]) + 1e-9) / (_sum(com[h]) + 1e-9) for h in honest)
    assert worst > 0.85, f"in-concentration punishes an honest member too much (worst retention {worst:.2f})"


def test_temporal_decays_inactive():
    """
    Temporal dynamics (§1.7, Art. VI): whoever stops contributing DECAYS; whoever keeps contributing
    holds. A pioneer of a single work does not keep power (anti-long-range); the active one does.
    """
    import temporal
    traj = temporal.simulate()
    active = traj["active_honest"]
    retired = traj["retired_honest"]
    pioneer = traj["sleeping_pioneer"]
    # the active one ends above where it started (contributing keeps/grows the power)
    assert active[-1] > active[0], "the active honest should not decay"
    # the retired one clearly decays from its peak after stopping (anti-entrenchment)
    assert retired[-1] < max(retired) * 0.8, "the retired one should decay after stopping"
    # the sleeping pioneer loses most of its power (anti-long-range)
    assert pioneer[-1] < max(pioneer) * 0.3, "the sleeping pioneer should lose almost everything"
    # the farm-and-sit ring puppets deflate over time
    puppets = traj["ring_puppets"]
    assert puppets[-1] < puppets[1], "the ring puppets should deflate"


def test_graduation_frees_mentor_quota():
    """
    Protege graduation (§1.5c): the protege starts depending on the mentor vouch (independent rep ~0)
    and, as it earns its own work+vouches, graduates → its independent rep dominates and the mentor's
    quota is freed (one less live vouch). The scaffolding dilutes.
    """
    import graduation
    traj = graduation.simulate()
    # starts dependent (little or no independent reputation) and ends standing on its own
    assert traj["independent"][0] < 0.2 * max(traj["total"][0], 1e-9), "A should not start independent"
    assert traj["graduated"][-1], "A should have graduated within the horizon"
    assert traj["independent"][-1] > 0.8 * traj["total"][-1], "after graduating, A stands on its own"
    # at graduation, the mentor's free quota rises (the live vouch was released)
    ep = next(t for t in range(len(traj["graduated"])) if traj["graduated"][t])
    assert traj["M_free_quota"][ep] > traj["M_free_quota"][ep - 1], "graduating should free the mentor's quota"


def test_edge_aging_freshness_premium():
    """
    Edge aging (§1.7, Art. VI): with identical evidence, whoever RENEWS their vouches beats whoever
    rests on old vouches (freshness premium), and aging adds EXTRA decay to the dormant node vs the
    no-aging control (ρ=1).
    """
    import edge_aging
    traj = edge_aging.simulate()                  # with aging (ρ=0.7)
    control = edge_aging.simulate(rho_edge=1.0)   # without aging
    # the renewer ends clearly above the dormant one (same evidence)
    assert traj["renewer"][-1] > 1.5 * traj["dormant"][-1], "the renewer should beat the dormant one"
    # aging cuts the dormant one vs the no-aging control
    assert traj["dormant"][-1] < control["dormant"][-1], "aging should cut the dormant one vs ρ=1"


def test_whitewashing_loses_everything():
    """The fresh pseudonym (whitewashing) inherits no reputation: ~0 vs the consolidated one."""
    sc = scenarios.scenario_whitewashing()
    rep = reputation_vector(sc.agents, sc.graph)
    old, new = _sum(rep["ww_old"]), _sum(rep["ww_new"])
    assert new < 0.01 * max(old, 1e-9), "the fresh pseudonym keeps too much reputation"
    assert old > 1.0, "the consolidated one should have appreciable reputation"


def test_cascade_slashing():
    """Slashing climbs the vouch chain; the outsider is unaffected."""
    reputation = {"mentor": 200.0, "middle": 120.0, "protege": 100.0, "outsider": 150.0}
    reg = VouchRegistry()
    reg.sponsor_link("mentor", "middle")
    reg.sponsor_link("middle", "protege")
    d = cascade_slashing(reputation, reg, "protege", 100.0, sponsor_fraction=0.5)
    assert d["protege"] == 0.0
    assert d["middle"] == 70.0   # -50
    assert d["mentor"] == 175.0  # -25
    assert d["outsider"] == 150.0  # unaffected


def test_conservative_aggregate():
    """min <= median; min penalises the weak dimension (you don't buy integrity with expertise)."""
    vec = {"a": 0.0, "b": 100.0, "c": 100.0, "d": 100.0}
    assert conservative_aggregate(vec, "min") == 0.0
    assert conservative_aggregate(vec, "median") == 100.0


def test_independence_penalises_inbreeding():
    """A reciprocal vouch with shared neighbours is less independent than one towards a stranger."""
    g = TrustGraph()
    # ring: a<->b, both vouch for x and y (high overlap)
    for u in ("a", "b"):
        for v in ("x", "y"):
            g.attest(u, v, "commerce")
    g.attest("a", "b", "commerce")
    g.attest("b", "a", "commerce")
    # stranger: p vouches for q, no reciprocity nor shared neighbours
    g.attest("p", "q", "commerce")
    inbred = g.independence("a", "b", "commerce")
    stranger = g.independence("p", "q", "commerce")
    assert inbred < stranger, f"inbred {inbred} should be < stranger {stranger}"
    assert stranger == 1.0


def test_sublinear_quota():
    """The vouch quota grows sublinearly (decreasing returns)."""
    c10, c100, c1000 = vouch_quota(10), vouch_quota(100), vouch_quota(1000)
    assert c10 < c100 < c1000
    assert (c1000 - c100) < (c100 - c10) * 3  # growth decelerates (log)


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    failures = 0
    for t in tests:
        try:
            t()
            print(f"  PASS  {t.__name__}")
        except AssertionError as e:
            failures += 1
            print(f"  FAIL  {t.__name__}: {e}")
    print(f"\n{len(tests) - failures}/{len(tests)} tests OK")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
