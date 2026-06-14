# Harlequin reputation engine prototype

The project's first **executable code**. It implements the core described in `SPEC.md §1`
(reputation), `§1.6` (anti-collusion) and `§2.2` (consensus by reputation-weighted sortition), and
**tests it with attack simulations**. Python 3 standard library only — **no external dependencies**
(the project's OPSEC criterion).

> This is NOT the blockchain. It is the test bench for the mechanism that matters most: reputation. It
> serves to **measure** whether the anti-Sybil and anti-collusion defences do what the SPEC promises,
> before building the chain around them. Layer 2 of `ROADMAP.md`, step 0.

## What it demonstrates

The SPEC's central bet (§1.5, §2.4): **the real anti-Sybil wall is EARNED reputation, not the
personhood test.** Creating fake identities or farming vouches in a circle must not grant power.

| Simulation | Result (see `RESULTS.md`) |
|---|---|
| 200 Sybils (40% of the network) | capture **0%** of reputation and of consensus |
| Collusion ring of 30 | ~0% of consensus power |
| Reputation laundering (1 real splits to 29 puppets) | the anti-collusion damping cuts it **~9×** |
| Pseudonym whitewashing | the fresh pseudonym loses **all** reputation |
| Cascade slashing | the protege's fraud hits its sponsors |

## How to run

```bash
cd prototipos/reputacion
python3 run_all.py            # generates RESULTS.md
python3 run_all.py --stdout   # also prints it
python3 tests/test_engine.py  # self-audit (15 tests, no pytest)
```

## Layout

```
prototipos/reputacion/
├── harlequin_rep/          # the engine (package)
│   ├── model.py            # dimensions (§1.2b) + agent with two gates (§1.4)
│   ├── graph.py            # vouch graph + anti-collusion independence (§1.6)
│   ├── reputation.py       # evidence-anchored EigenTrust + damping (§1.3, §1.6)
│   ├── consensus.py        # reputation-weighted sortition (§2.2)
│   └── vouch.py            # quota, mentor dividend, cascade slashing (§1.5c, §1.7)
├── scenarios.py            # network populations + attacks
├── adaptive.py             # adaptive-collusion sweep (fragment to evade, §1.6)
├── temporal.py             # multi-epoch dynamics: decay §1.7 / anti-entrenchment Art. VI
├── graduation.py           # protege graduation: sponsorship dilutes (§1.5c)
├── edge_aging.py           # vouch aging per epoch: trust is perishable (§1.7)
├── run_all.py              # runner -> RESULTS.md
├── tests/test_engine.py    # self-audit tests
└── RESULTS.md              # generated report
```

## How it works (technical summary)

Each dimension's reputation is the stationary vector of **EigenTrust** (Kamvar 2003) with teleport to
a **pre-trust anchored in real evidence** (settled deals, §1.3a) + genesis seed (§1.4):

```
t = (1 - α)·Cᵀ·t + α·p
```

- `C` = vouch matrix, row-stochastic, **damped by independence** (§1.6): a vouch between members of a
  closed ring (reciprocal + shared neighbours) is worth almost nothing.
- **Key detail:** the damping is normalised by the UNDAMPED sum, so an inbred node emits
  **sub-stochastic** rows; the trust that "does not propagate" **leaks to the pre-trust**. This is what
  stops a ring from recirculating and amplifying reputation. (If we normalised by the damped sum, a
  uniform factor would cancel and the damping would do nothing — a real bug, fixed; see the test
  `test_damping_reduces_laundering`.)
- `p` = pre-trust = normalised evidence. Without real evidence, no game of vouches manufactures power.

The **consensus** (§2.2) draws committees with probability ∝ the **conservative** aggregate of the
vector (minimums/medians, never a sum: you do not buy integrity with expertise, §1.2b).

## Limitations (honest) and next iteration

- **Sophisticated collusion — addressed (§2b/§2c of the report).** Besides the dense clique, we model
  the **scattered** ring (community-detection defence) and the **adaptive** one that fragments the ring
  to evade that label (`adaptive.py`). Finding: fragmenting evades the label but chokes the reputation
  flow, and what leaks is **single-dimension** → under the conservative aggregate (§1.2b) the puppets'
  consensus power collapses to ~0 at any fragmentation.
- **Asymmetric collusion — gap closed (§2d of the report).** A **PageRank funnel** (many feeders vouch
  for a single target, no reciprocity) **evades the local damping**: the target vouches for nobody, so
  it leaves no reciprocity nor overlap signature, and `independence(feeder→c0)=1`. Now stopped by an
  **in-degree-concentration** signal (`graph.py:in_concentration_signals`): it measures how
  concentrated a target's inflow is per community (Herfindahl), gated by the number of distinct
  vouchers and by the target's **evidence deficit** — so a funnel onto a 0-evidence target is cut
  ~77–85% (robust to feeder diversification, since funnelling itself binds the feeders into one
  star-community) while legitimately popular honest members keep ≥85%. Two independent defences now
  stop it: this graph signal **and** the conservative aggregate (§1.2b), since the pump is
  single-dimension.
- **Temporal dynamics — addressed (§6 of the report, `temporal.py`).** Multi-epoch simulation aging
  the evidence anchor: whoever stops contributing decays (anti-entrenchment, Art. VI), a single-work
  pioneer keeps no power (free anti-long-range), a farm-and-sit ring deflates.
- **Protege graduation — addressed (§7 of the report, `graduation.py`).** A protege enters leaning on
  the mentor vouch and, as it earns independent reputation, graduates: the vouch is released and stops
  taking up the mentor's quota (the liability persists). The scaffolding is designed to dilute.
- **Vouch aging — addressed (§8 of the report, `edge_aging.py`).** Besides the evidence, vouches decay
  over time unless renewed. Controlled experiment (same evidence, different vouch freshness): freshness
  premium ~200%; aging adds relative decay over the no-aging control. Honest nuance: uniform decay
  partly cancels under row normalisation, so the real effect is RELATIVE (not renewing while others
  do) — exactly what we want to reward.
- The VRF is simulated with a seeded PRNG; in production it is a real verifiable random function.
- The figures are **relative** (power distribution), not production parameters (those are `[PARAMETER]`
  in the SPEC, to be derived with modelling).

Live frontiers: anti-collusion with an explicit economic cost, and a formal safety/liveness proof of
the consensus.
