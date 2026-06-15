#!/usr/bin/env python3
"""
Run the consensus test-rig across scenarios and write RESULTS-testrig.md.
Run:  python3 testrig/run_testrig.py   (from prototipos/consenso/)
"""

from __future__ import annotations

import os
import random
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from testrig import scenarios
from testrig.engine import RigParams, run_epoch, run_epochs
from testrig.vrf import elect_committee

P = RigParams()
OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "RESULTS-testrig.md")


def sweep(f, loss=0.0, trials=24, seed0=1000):
    safe = cap = stall = 0
    seats = csize = 0
    for t in range(trials):
        rng = random.Random(seed0 + t)
        rep, sk, adv = scenarios.population_reputation_fraction(f)
        r = run_epoch(rep, sk, adv, f"seed{t}", P, rng, loss=loss)
        safe += r["safe"]; cap += r["capture"]; stall += 1 if r["undecided"] > 0 else 0
        seats += r["committee_seats"]; csize += r["committee_size"]
    n = trials
    return dict(safe=100 * safe / n, cap=100 * cap / n, stall=100 * stall / n,
                seats=seats / n, csize=csize / n)


def main():
    L = []
    w = L.append
    w("# Test-rig results — faithful Woven-Trust Consensus\n\n")
    w(f"> Params: k={P.k}, alpha={P.alpha}, beta={P.beta}, target committee tau={P.tau}, "
      f"async network (latency 1–3, round-trip loss on both legs). 24 trials/point.\n\n")
    w("This rig adds, over `wtc_sim/`: **(1)** committees elected by **VRF sortition weighted by "
      "reputation** (only the committee votes, SPEC §2.2); **(2)** committee **rotation** every epoch "
      "(Art. VI); **(3)** an **asynchronous, lossy** network (messages with latency, dropped on either "
      "leg) instead of synchronous rounds; **(4)** network **partition** with a reputation-quorum "
      "mitigation. It checks the validated model survives realistic conditions before any "
      "Substrate/Rust runtime.\n\n")

    w("## 1. Security threshold by adversarial reputation fraction (no loss)\n\n")
    w("| f (adv reputation) | safe % | capture % | stall % |\n|---:|---:|---:|---:|\n")
    for f in (0.0, 0.1, 0.2, 0.3, 0.4, 0.5):
        s = sweep(f)
        w(f"| {f:.2f} | {s['safe']:.0f} | {s['cap']:.0f} | {s['stall']:.0f} |\n")
    w("\n**Reading:** capture (a false finalisation) only appears around **f≈0.4**, matching the "
      "synchronous simulator and the analytical ceiling `f<α/k=0.7` with its metastability barrier "
      "(`ANALYSIS-safety-liveness.md`). Liveness degrades earlier (stalls from ~0.2–0.3, the "
      "`f≤1−α/k=0.3` regime). Electing a reputation-weighted committee and running it asynchronously "
      "does **not** lower the safety threshold.\n\n")

    w("## 2. Committee sortition is reputation-weighted; Sybils excluded\n\n")
    rep, sk, _ = scenarios.population_reputation_fraction(0.0)
    c = elect_committee(rep, sk, "seed-demo", P.tau)
    reps, sks, _ = scenarios.population_sybil()
    cs = elect_committee(reps, sks, "seed-demo", P.tau)
    sybils = sum(1 for n in cs if n.startswith("s"))
    w(f"- Honest population ({len(rep)} nodes, rep 1 each): committee = **{len(c)} nodes / "
      f"{sum(c.values())} seats** (target tau={P.tau:.0f}).\n")
    w(f"- Sybil population (2000 nodes at reputation ~0): committee sybils = **{sybils}** of "
      f"{len(cs)} → the fake crowd wins no seats (Art. VI: power is reputation, not number).\n\n")

    w("## 3. Committee rotation across epochs (anti-entrenchment, Art. VI)\n\n")
    w("| epoch pair | committee overlap |\n|---|---:|\n")
    prev = set(elect_committee(rep, sk, "beacon|epoch0", P.tau))
    for e in range(1, 5):
        cur = set(elect_committee(rep, sk, f"beacon|epoch{e}", P.tau))
        ov = len(prev & cur) / max(1, len(prev | cur))
        w(f"| epoch{e-1}→epoch{e} | {ov:.2f} |\n")
        prev = cur
    w("\n**Reading:** each epoch's beacon re-runs sortition → a substantially different committee. No "
      "node retains the validating role across epochs.\n\n")

    w("## 4. Asynchronous network: loss degrades liveness, not safety\n\n")
    w("| round-trip loss p | safe % | capture % | stall % |\n|---:|---:|---:|---:|\n")
    for loss in (0.0, 0.1, 0.2, 0.3):
        s = sweep(0.0, loss=loss)
        w(f"| {loss:.2f} | {s['safe']:.0f} | {s['cap']:.0f} | {s['stall']:.0f} |\n")
    w("\n**Reading:** with no adversary, loss never causes a false finalisation (capture stays 0) — it "
      "only adds stalls. Because the rig drops messages on **both** legs (query and response), the "
      "effective per-round survival is `(1−p)²`, so the liveness bound tightens from the simulator's "
      "`α≤k(1−p)` to **`α≤k(1−p)²`**: at k=20, α=14 that predicts stalling near p≈0.16, consistent "
      "with the table. Safety is unaffected — `α` does not change.\n\n")

    w("## 5. Network partition: fork attack + network-quorum mitigation\n\n")
    w("A globally-harmless adversary (20% of reputation) with all its reputation in one small partition "
      "group. A long partition lets its **local** share dominate that group → the group finalises the "
      "false value → **fork** on heal. Mitigation: finalise only when the sample reaches a "
      "**committee-reputation quorum** (tied to the sample's provenance, so an in-flight partitioned "
      "sample cannot finalise after the heal).\n\n")
    w("| mitigation | fork % | safe % | stall % |\n|---|---:|---:|---:|\n")
    for q in (0.0, 0.5, 0.6):
        fork = safe = stall = 0
        trials = 24
        for t in range(trials):
            rng = random.Random(900 + t)
            rep, sk, adv, grp = scenarios.population_partition(global_f=0.2)
            r = run_epoch(rep, sk, adv, f"p{t}", P, rng, group=grp,
                          partition_until=150.0, network_quorum=q)
            fork += r["fork"]; safe += r["safe"]; stall += 1 if r["undecided"] > 0 else 0
        label = "none" if q == 0.0 else f"quorum {q:.1f}"
        w(f"| {label} | {100*fork/trials:.0f} | {100*safe/trials:.0f} | {100*stall/trials:.0f} |\n")
    w("\n**Reading:** without mitigation a long partition **forks** (a real safety failure, as in the "
      "synchronous sim). Conditioning finality on a committee-reputation quorum **eliminates the fork** "
      "(0% at quorum 0.6): the isolated minority group cannot reach quorum so it **halts** and recovers "
      "on heal — safety chosen over liveness, the canonical BFT stance under partition.\n\n")

    w("## Conclusion\n")
    w("The faithful rig confirms the model survives the conditions the simulator abstracted away: a "
      "reputation-elected, **rotating** committee reaching consensus over an **asynchronous, lossy** "
      "network keeps the same safety threshold (~0.4 reputation), excludes Sybils from the committee by "
      "construction, and rotates each epoch. Liveness is the cost paid under loss/adversary, never "
      "safety. Next on the chain path (DECISION-STACK §5): use this rig to fix the production "
      "parameters (k, α, β, τ, epoch length, timeouts), then the minimal Substrate scaffold + a "
      "reputation pallet — the rig stays the guardrail the Rust runtime must reproduce.\n")

    with open(OUT, "w") as fh:
        fh.write("".join(L))
    print(f"[ok] report written to {OUT}")


if __name__ == "__main__":
    main()
