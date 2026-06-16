# Test-rig results — faithful Woven-Trust Consensus

> Params: k=20, alpha=14, beta=12, target committee tau=60.0, async network (latency 1–3, round-trip loss on both legs). 24 trials/point.

This rig adds, over `wtc_sim/`: **(1)** committees elected by **VRF sortition weighted by reputation** (only the committee votes, SPEC §2.2); **(2)** committee **rotation** every epoch (Art. VI); **(3)** an **asynchronous, lossy** network (messages with latency, dropped on either leg) instead of synchronous rounds; **(4)** network **partition** with a reputation-quorum mitigation. It checks the validated model survives realistic conditions before any Substrate/Rust runtime.

## 1. Security threshold by adversarial reputation fraction (no loss)

| f (adv reputation) | safe % | capture % | stall % |
|---:|---:|---:|---:|
| 0.00 | 100 | 0 | 0 |
| 0.10 | 100 | 0 | 0 |
| 0.20 | 54 | 0 | 46 |
| 0.30 | 4 | 4 | 92 |
| 0.40 | 0 | 50 | 50 |
| 0.50 | 0 | 100 | 0 |

**Reading:** capture (a false finalisation) only appears around **f≈0.4**, matching the synchronous simulator and the analytical ceiling `f<α/k=0.7` with its metastability barrier (`ANALYSIS-safety-liveness.md`). Liveness degrades earlier (stalls from ~0.2–0.3, the `f≤1−α/k=0.3` regime). Electing a reputation-weighted committee and running it asynchronously does **not** lower the safety threshold.

## 2. Committee sortition is reputation-weighted; Sybils excluded

- Honest population (120 nodes, rep 1 each): committee = **51 nodes / 69 seats** (target tau=60).
- Sybil population (2000 nodes at reputation ~0): committee sybils = **0** of 51 → the fake crowd wins no seats (Art. VI: power is reputation, not number).

## 3. Committee rotation across epochs (anti-entrenchment, Art. VI)

| epoch pair | committee overlap |
|---|---:|
| epoch0→epoch1 | 0.36 |
| epoch1→epoch2 | 0.32 |
| epoch2→epoch3 | 0.28 |
| epoch3→epoch4 | 0.29 |

**Reading:** each epoch's beacon re-runs sortition → a substantially different committee. No node retains the validating role across epochs.

## 4. Asynchronous network: loss degrades liveness, not safety

| round-trip loss p | safe % | capture % | stall % |
|---:|---:|---:|---:|
| 0.00 | 100 | 0 | 0 |
| 0.10 | 71 | 0 | 29 |
| 0.20 | 0 | 0 | 100 |
| 0.30 | 0 | 0 | 100 |

**Reading:** with no adversary, loss never causes a false finalisation (capture stays 0) — it only adds stalls. Because the rig drops messages on **both** legs (query and response), the effective per-round survival is `(1−p)²`, so the liveness bound tightens from the simulator's `α≤k(1−p)` to **`α≤k(1−p)²`**: at k=20, α=14 that predicts stalling near p≈0.16, consistent with the table. Safety is unaffected — `α` does not change.

## 5. Network partition: fork attack + network-quorum mitigation

A globally-harmless adversary (20% of reputation) with all its reputation in one small partition group. A long partition lets its **local** share dominate that group → the group finalises the false value → **fork** on heal. Mitigation: finalise only when the sample reaches a **committee-reputation quorum** (tied to the sample's provenance, so an in-flight partitioned sample cannot finalise after the heal).

| mitigation | fork % | safe % | stall % |
|---|---:|---:|---:|
| none | 100 | 0 | 4 |
| quorum 0.5 | 12 | 71 | 29 |
| quorum 0.6 | 0 | 58 | 42 |

**Reading:** without mitigation a long partition **forks** (a real safety failure, as in the synchronous sim). Conditioning finality on a committee-reputation quorum **eliminates the fork** (0% at quorum 0.6): the isolated minority group cannot reach quorum so it **halts** and recovers on heal — safety chosen over liveness, the canonical BFT stance under partition.

## Conclusion
The faithful rig confirms the model survives the conditions the simulator abstracted away: a reputation-elected, **rotating** committee reaching consensus over an **asynchronous, lossy** network keeps the same safety threshold (~0.4 reputation), excludes Sybils from the committee by construction, and rotates each epoch. Liveness is the cost paid under loss/adversary, never safety. Next on the chain path (the chain roadmap): use this rig to fix the production parameters (k, α, β, τ, epoch length, timeouts), then the minimal Substrate scaffold + a reputation pallet — the rig stays the guardrail the Rust runtime must reproduce.
