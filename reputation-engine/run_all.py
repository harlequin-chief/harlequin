#!/usr/bin/env python3
"""
Prototype runner: runs every scenario, measures how much power each faction captures and writes the
report RESULTS.md. No external dependencies.

Usage:
    python3 run_all.py            # runs and writes RESULTS.md
    python3 run_all.py --stdout   # also dumps the report to the screen

Metrics:
  - Earned-reputation share per faction (sum of the whole vector) -> intuition of "power mass".
  - Consensus committee share (reputation-weighted sortition, §2.2) -> real structural power.
  - Collusion: comparison WITH vs WITHOUT damping (§1.6) to measure what anti-collusion contributes.
  - Whitewashing: reputation of the old vs the new pseudonym (§5).
  - Cascade slashing (§1.5c, §1.7): what the sponsor loses when the protege defrauds.
"""

from __future__ import annotations

import sys
from collections import defaultdict

from harlequin_rep.consensus import weighted_sortition
from harlequin_rep.reputation import conservative_aggregate, reputation_vector
from harlequin_rep.vouch import (
    VouchRegistry,
    cascade_slashing,
    mentor_dividend,
    vouch_quota,
)
import adaptive
import scenarios


# ----------------------------- measurement utilities --------------------------------------------

def sum_vector(vec: dict[str, float]) -> float:
    return sum(vec.values())


def shares_by_faction(value_per_agent: dict[str, float], factions: dict[str, str]) -> dict[str, float]:
    """Groups a per-agent value into (%) shares per faction."""
    per_fac: dict[str, float] = defaultdict(float)
    for aid, v in value_per_agent.items():
        per_fac[factions[aid]] += v
    total = sum(per_fac.values())
    if total <= 0:
        return {f: 0.0 for f in per_fac}
    return {f: 100.0 * v / total for f, v in per_fac.items()}


def consensus_weights(rep_vec: dict[str, dict[str, float]], mode: str = "median") -> dict[str, float]:
    """Consensus weight = conservative aggregate of the reputation vector (§1.2b, §2.2)."""
    return {aid: conservative_aggregate(vec, mode) for aid, vec in rep_vec.items()}


def fmt_pct(d: dict[str, float]) -> str:
    order = ["genesis", "honest", "sybil", "colluder", "whitewasher"]
    parts = [f"{f}={d[f]:.2f}%" for f in order if f in d and d[f] > 1e-9]
    return ", ".join(parts) if parts else "(none)"


# ----------------------------- scenario execution -----------------------------------------------

def measure_scenario(sc) -> dict:
    rep_vec = reputation_vector(sc.agents, sc.graph)
    rep_total = {aid: sum_vector(vec) for aid, vec in rep_vec.items()}
    rep_share = shares_by_faction(rep_total, sc.factions)

    weights = consensus_weights(rep_vec, "median")
    count = weighted_sortition(weights, committee_size=21, epochs=2000)
    committee_share = shares_by_faction({k: float(v) for k, v in count.items()}, sc.factions)

    return {
        "scenario": sc,
        "rep_vec": rep_vec,
        "rep_total": rep_total,
        "rep_share": rep_share,
        "committee_share": committee_share,
        "weights": weights,
    }


def measure_collusion_damping() -> dict:
    """
    Same collusion ring (with c0 = real worker the ring tries to amplify), with and without the
    anti-collusion damping (§1.6). Measures: the ring's reputation share, and how much is laundered
    to the 29 cronies.
    """
    sc = scenarios.scenario_collusion()
    on = reputation_vector(sc.agents, sc.graph, damping=True)
    off = reputation_vector(sc.agents, sc.graph, damping=False)
    on_total = {aid: sum_vector(v) for aid, v in on.items()}
    off_total = {aid: sum_vector(v) for aid, v in off.items()}

    colluders = [aid for aid, f in sc.factions.items() if f == "colluder"]
    cronies = [c for c in colluders if c != "c0"]  # the 29 with no real work

    def crony_split(rep_total: dict[str, float]) -> tuple[float, float, float]:
        """(c0 reputation, total laundered to the 29 cronies, max of one crony)."""
        rep_c0 = rep_total["c0"]
        total_cr = sum(rep_total[s] for s in cronies)
        max_cr = max(rep_total[s] for s in cronies)
        return rep_c0, total_cr, max_cr

    return {
        "on": crony_split(on_total),
        "off": crony_split(off_total),
        "n_cronies": len(cronies),
    }


def _laundered(sc, damping: bool = True, community: bool = False) -> tuple[float, float]:
    """(c0 reputation, total laundered to the other colluders) under the given scenario."""
    rep = reputation_vector(sc.agents, sc.graph, damping=damping, community=community)
    total = {aid: sum_vector(v) for aid, v in rep.items()}
    cronies = [c for c, f in sc.factions.items() if f == "colluder" and c != "c0"]
    return total["c0"], sum(total[s] for s in cronies)


def measure_scattered_collusion() -> dict:
    """
    SCATTERED ring (frontier §1.6) with three defences: no damping, LOCAL damping (pairwise
    independence only), and LOCAL + COMMUNITY damping (label propagation + suspicion). Measures
    whether community detection closes the gap left by the local damping.
    """
    dense = scenarios.scenario_collusion()
    scattered = scenarios.scenario_scattered_collusion()
    _, cr_dense_local = _laundered(dense, damping=True)
    _, cr_none = _laundered(scattered, damping=False)
    _, cr_local = _laundered(scattered, damping=True, community=False)
    _, cr_comm = _laundered(scattered, damping=True, community=True)
    # false-positive control: does community detection harm honest reputation?
    rep_local = reputation_vector(dense.agents, dense.graph, damping=True, community=False)
    rep_comm = reputation_vector(dense.agents, dense.graph, damping=True, community=True)
    honest = [a.id for a in dense.agents if dense.factions[a.id] in ("honest", "genesis")]
    h_local = sum(sum_vector(rep_local[h]) for h in honest)
    h_comm = sum(sum_vector(rep_comm[h]) for h in honest)
    return {
        "cr_dense_local": cr_dense_local,
        "cr_none": cr_none,
        "cr_local": cr_local,
        "cr_comm": cr_comm,
        "honest_local": h_local,
        "honest_comm": h_comm,
    }


def measure_whitewashing() -> dict:
    sc = scenarios.scenario_whitewashing()
    rep_vec = reputation_vector(sc.agents, sc.graph)
    old = sum_vector(rep_vec["ww_old"])
    new = sum_vector(rep_vec["ww_new"])
    return {"old": old, "new": new}


def demo_slashing() -> dict:
    """
    Demonstrates persistent liability (§1.5c) + cascade slashing (§1.7).

    Vouch chain: mentor -> middle -> protege. The protege defrauds and loses 100. The hit propagates
    upwards (fraction 0.5 per hop). Sponsoring carelessly is costly.
    """
    reputation = {"mentor": 200.0, "middle": 120.0, "protege": 100.0, "outsider": 150.0}
    reg = VouchRegistry()
    reg.sponsor_link("mentor", "middle")
    reg.sponsor_link("middle", "protege")
    after = cascade_slashing(reputation, reg, culprit="protege", loss=100.0)
    return {"before": reputation, "after": after}


def demo_vouch_economy() -> dict:
    """Sublinear quota (§1.5c) + mentor dividend that does NOT pay off with puppets."""
    quotas = {rep: vouch_quota(rep) for rep in (1, 10, 100, 1000, 10000)}
    div_real = mentor_dividend(protege_independent_rep=80.0)   # protege with independent rep
    div_puppet = mentor_dividend(protege_independent_rep=0.5)  # puppet: independent rep ~0
    return {"quotas": quotas, "div_real": div_real, "div_puppet": div_puppet}


# ----------------------------- report -----------------------------------------------------------

def build_report() -> str:
    L: list[str] = []
    w = L.append

    w("# RESULTS — Harlequin reputation engine prototype\n")
    w("> Generated by `run_all.py` (stdlib only). Reproducible: seeded PRNG. The figures are\n"
      "> **relative** (power distribution), not absolute units. Maps SPEC §1 (reputation), §1.6\n"
      "> (anti-collusion), §2.2 (consensus) and §5 (exit/pseudonym).\n")

    w("\n## Thesis under test\n")
    w("**The real anti-Sybil wall is EARNED reputation, not the personhood test** (SPEC §1.5, §2.4).\n"
      "Creating fake identities or farming vouches in a circle must NOT grant structural power,\n"
      "because reputation is anchored in real evidence and collusion is damped (§1.6).\n")

    # main scenarios
    results = {}
    for fab in scenarios.ALL:
        sc = fab()
        if sc.name == "whitewashing":
            continue  # handled separately
        results[sc.name] = measure_scenario(sc)

    w("\n## 1. Reputation and consensus power per scenario\n")
    w("| Scenario | # agents | Earned-reputation share | Consensus committee share |\n")
    w("|---|---|---|---|\n")
    for name in ("honest", "sybil", "collusion"):
        r = results[name]
        sc = r["scenario"]
        w(f"| **{name}** | {len(sc.agents)} | {fmt_pct(r['rep_share'])} | "
          f"{fmt_pct(r['committee_share'])} |\n")

    w("\n**Reading:**\n")
    syb = results["sybil"]
    col = results["collusion"]
    w(f"- **Sybil:** 200 fake accounts (40% of the network) capture "
      f"**{syb['rep_share'].get('sybil', 0.0):.2f}%** of the reputation and "
      f"**{syb['committee_share'].get('sybil', 0.0):.2f}%** of the consensus committee. "
      "They are born with reputation 0; without evidence nor vouches from the reputed, they gain no power.\n")
    w(f"- **Collusion:** the ring of 30 vouching in a circle captures "
      f"**{col['rep_share'].get('colluder', 0.0):.2f}%** of the reputation and "
      f"**{col['committee_share'].get('colluder', 0.0):.2f}%** of the committee, despite the 3 fooled "
      "honest members vouching for it.\n")

    # damping
    damp = measure_collusion_damping()
    c0_on, cr_on, max_on = damp["on"]
    c0_off, cr_off, max_off = damp["off"]
    w("\n## 2. How much does anti-collusion (damping, §1.6) contribute?\n")
    w("**Reputation-laundering** attack: a ring member (`c0`) DOES have high legitimate reputation\n"
      "(real work) and tries to **split it** among 29 cronies by vouching in a circle. The damping\n"
      "must stop the propagation: `c0` keeps its own, the cronies stay at ~0.\n\n")
    w(f"| | `c0` reputation (legitimate) | Split to the {damp['n_cronies']} cronies | "
      "Max of one crony |\n|---|---|---|---|\n")
    w(f"| **WITHOUT damping** | {c0_off:.1f} | {cr_off:.1f} | {max_off:.1f} |\n")
    w(f"| **WITH damping** | {c0_on:.1f} | {cr_on:.1f} | {max_on:.1f} |\n")
    if cr_on > 0:
        w(f"\nThe damping cuts the reputation laundered to the cronies **{cr_off / cr_on:.1f}×**.\n")
    else:
        w("\nWith damping, what is laundered to the cronies falls to ~0.\n")
    w("Without anti-collusion, a single member with real reputation can 'lend' it to its whole\n"
      "ring of puppets; with it, reputation stays where it was earned.\n")

    # scattered collusion (open frontier §1.6) + community defence
    sca = measure_scattered_collusion()
    w("\n## 2b. Open frontier: SOPHISTICATED collusion (scattered ring, §1.6)\n")
    w("The dense clique is easy to detect. A **scattered** ring (each colluder vouches for few, low "
      "reciprocity/overlap) mimics honest patterns and **slips through** the local damping. We test a "
      "new defence: **community detection** (the global signal the scattered ring does leave).\n\n")
    w("| Defence on the scattered ring | Laundered to the cronies |\n|---|---|\n")
    w(f"| no damping (reference) | {sca['cr_none']:.1f} |\n")
    w(f"| LOCAL damping (pairwise independence only) | {sca['cr_local']:.1f} |\n")
    w(f"| LOCAL + COMMUNITY damping (new) | {sca['cr_comm']:.1f} |\n")
    w(f"\n(Reference: the dense clique with local damping launders only {sca['cr_dense_local']:.1f}.)\n")
    gain = sca["cr_local"] / sca["cr_comm"] if sca["cr_comm"] > 0 else float("inf")
    w(f"\n**Result:** community detection reduces the scattered ring's laundering from "
      f"**{sca['cr_local']:.0f}** (local damping) to **{sca['cr_comm']:.0f}** "
      f"(~{gain:.1f}× less) — it closes much of the gap the local damping left.\n")
    harm = 100.0 * (1.0 - sca["honest_comm"] / sca["honest_local"]) if sca["honest_local"] else 0.0
    w(f"**False-positive control:** total honest reputation barely changes with the community defence "
      f"({harm:+.1f}% over the base) → it does not punish honest communities (they have real evidence, "
      "lowering their suspicion). Honest: still opt-in and to validate further; adaptive collusion "
      "(fragmenting the ring into several communities) is the next frontier (§1.6, PAPER §10).\n")

    # adaptive collusion: fragment to evade the community label (§1.6, open frontier)
    rows = adaptive.sweep(bridges=1)
    w("\n## 2c. Open frontier: ADAPTIVE collusion (fragment to evade, §1.6)\n")
    w("The attacker knows we punish dense-without-evidence communities, so it **fragments** the ring "
      "into small scattered sub-rings to fall below the radar. But to launder c0's real reputation to "
      "the puppets, it must **flow** between fragments through a few bridges. That is the tension: "
      "evading the label chokes the flow.\n\n")
    w("| fragments | communities seen | laundered no damping | laundered +community | **consensus power (min)** |\n")
    w("|---:|---:|---:|---:|---:|\n")
    for k, m in rows:
        w(f"| {k} | {m['n_com']} | {m['none']:.1f} | {m['community']:.1f} | **{m['power']:.3f}** |\n")
    w("\n**Result (two-fold):** (1) fragmenting **raises** the number of communities seen (evades the "
      "label) but does **not raise** the laundering under defence — as it fragments, the bridges become "
      "bottlenecks the LOCAL damping bites harder; the worst case for the defence is the scattered ring "
      "WITHOUT fragmenting. (2) What does leak is **single-dimension** (only `commerce`): under the "
      "conservative aggregate (min, §1.2b) that governs consensus/vouch power, the puppets collapse to "
      "**~0** at any fragmentation. For real power the attacker would need verifiable evidence in "
      "**every** dimension for each puppet = doing the honest work. *Diffuse laundering buys no "
      "structural power.*\n")

    # asymmetric collusion: PageRank funnel (frontier §1.6, honest gap finding)
    from harlequin_rep.reputation import conservative_aggregate as _agg_min
    w("\n## 2d. Open frontier: ASYMMETRIC collusion (PageRank funnel, §1.6)\n")
    w("Instead of a reciprocal ring, a **directed funnel**: many feeders (with some real work) all "
      "vouch for the same target c0 (evidence 0), without reciprocity, to concentrate/funnel their "
      "reputation onto c0. **Honest finding:** the funnel **evades the local damping** — since c0 "
      "vouches for nobody, neighbour overlap is 0 and reciprocity is 0, so `independence(feeder→c0)=1` "
      "and the damping does not cut the pump.\n\n")
    w("| feeder diversification | c0 no damping | c0 +local damping | c0 +community | **c0 consensus power (min)** |\n")
    w("|---:|---:|---:|---:|---:|\n")
    for div in (0, 3, 6):
        sc_a = scenarios.scenario_asymmetric_collusion(diversify=div)
        r_off = reputation_vector(sc_a.agents, sc_a.graph, damping=False)
        r_loc = reputation_vector(sc_a.agents, sc_a.graph, damping=True)
        r_com = reputation_vector(sc_a.agents, sc_a.graph, damping=True, community=True)
        w(f"| {div} | {sum_vector(r_off['c0']):.1f} | {sum_vector(r_loc['c0']):.1f} | "
          f"{sum_vector(r_com['c0']):.1f} | **{_agg_min(r_com['c0'], 'min'):.3f}** |\n")
    w("\n**Result (two-fold, as in §2c):** (1) the graph damping does **not** stop the funnel — a real "
      "gap: local independence looks at reciprocity and overlap, signatures the funnel does not leave. "
      "(2) But the pump is **single-dimension** (only `commerce`): under the conservative aggregate "
      "(min, §1.2b) c0's consensus power collapses to **~0**. *The vectorial aggregate is the backstop "
      "where the graph damping does not reach.* For real power, c0 would need evidence funnelled in "
      "**every** dimension = the feeders doing real work in all of them and ceding it = honest work. "
      "**Live frontier:** harden independence with an **in-degree-concentration-from-one-community** "
      "signal (with false-positive analysis on legitimately popular honest nodes) — next engine "
      "iteration.\n")

    # whitewashing
    ww = measure_whitewashing()
    w("\n## 3. Pseudonym whitewashing (§5)\n")
    w("Same human, two masks: a consolidated one and a fresh one.\n\n")
    w("| Pseudonym | Earned reputation |\n|---|---|\n")
    w(f"| consolidated (`ww_old`) | {ww['old']:.2f} |\n")
    w(f"| fresh (`ww_new`) | {ww['new']:.2f} |\n")
    w("\nAbandoning the pseudonym to 'start clean' costs **all** the earned reputation: it falls back to\n"
      "base citizenship. Reputation is bound to the mask and is not transferred (Art. VII/VIII).\n")

    # slashing
    sl = demo_slashing()
    w("\n## 4. Persistent liability + cascade slashing (§1.5c, §1.7)\n")
    w("The protege defrauds and loses 100. The hit climbs the vouch chain (½ per hop).\n\n")
    w("| Agent | Before | After | Δ |\n|---|---|---|---|\n")
    for aid in ("protege", "middle", "mentor", "outsider"):
        a, d = sl["before"][aid], sl["after"][aid]
        w(f"| {aid} | {a:.1f} | {d:.1f} | {d - a:+.1f} |\n")
    w("\nWhoever vouches answers for whom they let in, even later. The `outsider` (did not vouch) is\n"
      "unaffected. Sponsoring carelessly is costly -> it forces selectivity.\n")

    # vouch economy
    ec = demo_vouch_economy()
    w("\n## 5. Vouch economy (§1.5c)\n")
    w("**Live-vouch quota = sublinear function of reputation** (decreasing returns):\n\n")
    w("| Reputation | Vouch quota |\n|---|---|\n")
    for rep, quota in ec["quotas"].items():
        w(f"| {rep} | {quota} |\n")
    w(f"\n**Mentor dividend:** sponsoring someone with real independent reputation yields "
      f"{ec['div_real']:.2f}; sponsoring a puppet (independent reputation ~0) yields "
      f"{ec['div_puppet']:.2f}. Sponsor->puppet farms do not pay off (§1.6).\n")

    # temporal dynamics (§1.7 decay, Art. VI anti-entrenchment)
    import temporal
    traj = temporal.simulate()
    w("\n## 6. Temporal dynamics: decay and anti-entrenchment (§1.7, Art. VI)\n")
    w(f"The base engine is a fixed snapshot; here TIME is modelled in epochs by aging the evidence "
      f"anchor (ρ={temporal.RHO} retention per epoch). Uncontributed reputation evaporates (§1.7) and "
      "yesterday's power does not shield tomorrow's (Art. VI).\n\n")
    w("| epoch | Active honest | Retired honest (stops at t=3) | Sleeping pioneer (single work t=0) | Farm-and-sit ring puppets |\n")
    w("|---:|---:|---:|---:|---:|\n")
    for t in range(temporal.N_EPOCHS):
        w(f"| {t} | {traj['active_honest'][t]:.0f} | {traj['retired_honest'][t]:.0f} | "
          f"{traj['sleeping_pioneer'][t]:.0f} | {traj['ring_puppets'][t]:.0f} |\n")
    ret, pio = traj["retired_honest"], traj["sleeping_pioneer"]
    w(f"\n**Result:** whoever **keeps contributing** holds and grows; whoever **retires** decays "
      f"(~{100*(1-ret[-1]/max(ret)):.0f}% from its peak) → anti-entrenchment (Art. VI). A **pioneer** "
      f"of a single work deflates (~{100*(1-pio[-1]/max(pio)):.0f}%) → **free anti-long-range** defence: "
      "an old history is not reactivated into power. And a **farm** that farms and sits sees its puppets "
      "evaporate: collusion has to be SUSTAINED, not a sprint.\n")

    # protege graduation (§1.5c): the scaffolding dilutes
    import graduation
    gt = graduation.simulate()
    ep_g = next((t for t in range(len(gt["graduated"])) if gt["graduated"][t]), None)
    w("\n## 7. Protege graduation: sponsorship is scaffolding (§1.5c)\n")
    w("A protege enters sponsored (its reputation leans on the mentor vouch) and, as it does real work "
      "and receives independent vouches, it graduates: the mentor vouch is released and stops taking up "
      "its quota. The liability persists (cascade slashing). We measure A's reputation WITH vs WITHOUT "
      "the mentor vouch (the independent one).\n\n")
    w("| epoch | A total rep | A independent rep | % independent | M free quota | graduated |\n")
    w("|---:|---:|---:|---:|---:|:--:|\n")
    for t in range(graduation.N_EPOCHS):
        rt, ri = gt["total"][t], gt["independent"][t]
        pct = 100 * ri / rt if rt else 0.0
        w(f"| {t} | {rt:.0f} | {ri:.0f} | {pct:.0f}% | {gt['M_free_quota'][t]} | "
          f"{'yes' if gt['graduated'][t] else '—'} |\n")
    if ep_g is not None:
        w(f"\n**Result:** A starts dependent on the scaffolding (independent rep ~0) and at **epoch "
          f"{ep_g}** stands on its own (≥{int(graduation.GRADUATION_THRESHOLD*100)}% independent) → it "
          f"**graduates**, freeing the mentor's quota ({gt['M_free_quota'][ep_g-1]}→{gt['M_free_quota'][ep_g]}). "
          "Sponsoring done well invests in the protege becoming **independent**, not in tethering it; the "
          "scaffolding is designed to dilute (consistent with the genesis seed and Art. VI).\n")

    # edge aging (§1.7, Art. VI): trust is perishable
    import edge_aging
    at = edge_aging.simulate()
    ac = edge_aging.simulate(rho_edge=1.0)
    w("\n## 8. Vouch aging: trust is perishable (§1.7, Art. VI)\n")
    w(f"Closes the limitation of §6: besides aging the evidence, the **vouches** age "
      f"(RHO_EDGE={edge_aging.RHO_EDGE}). Controlled experiment: two honest agents with **identical "
      "constant evidence**; one renews vouches each epoch, the other does not.\n\n")
    w("| epoch | renewer | dormant | dormant no aging (ρ=1) |\n|---:|---:|---:|---:|\n")
    for t in range(edge_aging.N_EPOCHS):
        w(f"| {t} | {at['renewer'][t]:.0f} | {at['dormant'][t]:.0f} | {ac['dormant'][t]:.0f} |\n")
    gap = 100 * (at["renewer"][-1] / max(at["dormant"][-1], 1e-9) - 1)
    extra = 100 * (1 - at["dormant"][-1] / max(ac["dormant"][-1], 1e-9))
    w(f"\n**Result:** freshness premium ~{gap:.0f}% (renewer over dormant, same evidence); aging cuts "
      f"the dormant node a further ~{extra:.0f}% over the no-aging control. Honest nuance: the uniform "
      "decay of all of a node's edges partly cancels under row normalisation (same as the already-fixed "
      "damping bug); the real effect is RELATIVE — not renewing while others do — exactly what we want "
      "to reward. Anti-entrenchment in the trust graph too.\n")

    w("\n## Conclusion\n")
    w("The prototype confirms, in figures, the SPEC's central bet: **structural power is not bought\n"
      "with identities nor with inbred vouches**. Sybils and collusion rings stay near 0% of consensus\n"
      "power; the anti-collusion damping is measurable and necessary; whitewashing does not pay off;\n"
      "and persistent liability makes collusion expensive. Sophisticated collusion — scattered ring\n"
      "(§2b) and fragmented/adaptive (§2c) — buys no power either: what leaks is single-dimension and\n"
      "the conservative aggregate (§1.2b) nullifies it. Live frontiers: hardening independence against\n"
      "the asymmetric funnel (§2d) and a formal safety/liveness proof of the consensus.\n")

    return "".join(L)


def main() -> None:
    report = build_report()
    path = "RESULTS.md"
    with open(path, "w", encoding="utf-8") as f:
        f.write(report)
    print(f"[ok] report written to {path}")
    if "--stdout" in sys.argv:
        print("\n" + report)


if __name__ == "__main__":
    main()
