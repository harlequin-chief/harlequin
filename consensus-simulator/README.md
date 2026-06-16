# Woven Trust Consensus — design & simulator

The original consensus of Harlequin: security anchored in **earned reputation over time**
(*reputation-time*), not in computation (proof of work) or capital (proof of stake).

- **The paper:** [Woven Trust Consensus](../docs/woven-trust-consensus.md) (English) ·
  [Consenso de Confianza Tejida](../docs/consenso-confianza-tejida.md) (Spanish, secondary).
- **The simulator:** this directory (`wtc_sim/` + `run_consensus.py`).

## What the simulator shows

A subsampled vote (Snowball / Avalanche), with WTC's own twist: **sampling weighted by reputation**
instead of uniform. Python standard library only.

```bash
python3 run_consensus.py            # generates RESULTS.md
python3 tests/test_consensus.py     # self-audit (11 tests)
```

Findings (see `RESULTS.md`):

- The **security threshold is measured in fraction of reputation, not in number of nodes.** Below
  ~40% adversarial reputation, the attacker never forces a false decision; liveness degrades earlier
  (~20–30%) as a stall, never as a wrong decision.
- A **Sybil swarm of 1000 nodes (93% of the network) with ~0 reputation does not break the network**
  under reputation-weighted sampling — it is almost never sampled. With *uniform* sampling (plain
  Avalanche) the same swarm captures the network: reputation-weighting is what defends.
- **Independence-weighted sampling (paper §5.4).** A correlated bloc that earns real reputation but
  keeps it in one trust cluster is captured at ≥~40% under reputation-only sampling; capping per-cluster
  committee seats neutralises it (it needs ⌈α/cap⌉ distinct clusters to capture, which the reputation
  engine resists). **The two prototypes compose.**
- **Adaptive adversary.** A minority-colour splitter attacks liveness (stalls) but never forces a false
  decision; under independence sampling it cannot even stall.
- **Network partition — attack found *and* mitigated (`partition.py`).** A globally harmless adversary
  (15%) concentrated in a small group, under a long partition, captures that group and forks on heal.
  Conditioning finality on seeing a network quorum drops the fork from ~100% to ~0–2% (safety over
  liveness).
- **Message loss / latency.** Loss degrades liveness (more stalls) but never breaks safety; progress
  needs α ≤ k·(1−p).

## Honesty

This is a binary-decision model. An analytical **safety/liveness analysis** that derives the regimes
from the parameters (safety `f < α/k` with a metastability barrier; liveness `f ≤ 1 − α/k`, and
`α ≤ k(1−p)(1−f)` under loss; the `⌈α/cap⌉` independence bound; the partition/quorum stance) is in
[`ANALYSIS-safety-liveness.md`](ANALYSIS-safety-liveness.md), with property-based regression tests.
A **machine-checked** proof (TLA+/Coq) remains open (see the paper, §10). The chain is not written
until the model holds up.
