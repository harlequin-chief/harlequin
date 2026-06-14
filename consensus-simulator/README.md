# Woven Trust Consensus — design & simulator

The original consensus of Harlequin: security anchored in **earned reputation over time**
(*reputation-time*), not in computation (proof of work) or capital (proof of stake).

- **The paper:** [`../docs/woven-trust-consensus.md`](../docs/woven-trust-consensus.md) (English) ·
  [`../docs/consenso-confianza-tejida.md`](../docs/consenso-confianza-tejida.md) (Spanish).
- **The simulator:** this directory (`wtc_sim/` + `run_consenso.py`).

## What the simulator shows

A subsampled vote (Snowball / Avalanche), with WTC's own twist: **sampling weighted by reputation**
instead of uniform. Python standard library only.

```bash
python3 run_consenso.py            # generates RESULTADOS-consenso.md
python3 tests/test_consenso.py     # self-audit (10 tests)
```

Findings (see `RESULTADOS-consenso.md`):

- The **security threshold is measured in fraction of reputation, not in number of nodes.** Below
  ~40% adversarial reputation, the attacker never forces a false decision; liveness degrades earlier
  (~20–30%) as a stall, never as a wrong decision.
- A **Sybil swarm of 1000 nodes (93% of the network) with ~0 reputation does not break the network**
  under reputation-weighted sampling — it is almost never sampled.
- **Contrast:** with *uniform* sampling (plain Avalanche, ignoring reputation), the same swarm
  captures the network. Reputation-weighting is what defends — the same lesson as the anti-collusion
  damping in the reputation engine.
- **Independence-weighted sampling (paper §5.4).** A subtler attacker earns real reputation but keeps
  it all in **one correlated trust cluster**. Reputation-only sampling treats it as independent and
  is captured at ≥~40%. Capping how many committee seats a single cluster may fill **neutralises** it
  by structure: to force the false value the bloc needs ⌈α/cap⌉ seats from *distinct* clusters. The
  protection holds until the attacker fragments into that many clusters — but each fragment must then
  look independent to community detection, which the reputation engine is built to resist. **The two
  prototypes compose.**
- **Adaptive adversary.** A splitter that each round reports the minority colour (worst-case, anti-
  finality) attacks **liveness** (it stalls convergence) but **never forces a false decision** — and
  under independence-weighted sampling it cannot even stall.
- **Network partition — attack found *and* mitigated (`particion.py`).** A globally harmless adversary
  (15%) concentrated in a small group, under a long partition, captures that group (its *local* share
  exceeds the threshold), which finalises the false value → **fork on heal** (a real safety failure,
  found in simulation). Mitigation: gate finality on seeing a **quorum of total reputation** — under
  partition the isolated group never reaches quorum, so it **stalls instead of forking** and recovers
  on heal. Fork drops from ~100% to ~0–2% (safety over liveness, as a robust BFT should).

## Honesty

This is a binary-decision model; message latency/loss is not yet modelled, and a **formal**
safety/liveness proof is still open (see the paper, §10). The chain is not written until the model
holds up.
