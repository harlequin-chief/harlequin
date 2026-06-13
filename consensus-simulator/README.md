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
python3 tests/test_consenso.py     # self-audit (5 tests)
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

## Honesty

This is a binary-decision model with a simple Byzantine adversary. It does **not** yet cover an
adaptive adversary, attacks on the independence of the sampling, or a formal safety/liveness proof —
all open problems (see the paper, §10). The chain is not written until the model holds up.
