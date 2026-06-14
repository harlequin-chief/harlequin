# Safety & liveness analysis of Woven-Trust Consensus (WTC)

> Analytical companion to the simulator (`wtc_sim/`, `RESULTS.md`). This is a **proof sketch /
> parameter analysis**, not a machine-checked proof: it derives the safety and liveness regimes from
> the model parameters and reconciles them with the simulation. A machine-checked proof (TLA+/Coq)
> remains open work (PAPER §10). Where a step is argued rather than proven, it is marked *(sketch)*.

## 1. Model

`N` nodes. Node `i` has reputation `r_i ≥ 0`; total `R = Σ r_i`. The decision is binary (`0` =
legitimate value all honest nodes start at; `1` = the adversary's conflicting value). Sub-sampled
Snowball (Avalanche family), one round at a time:

- Each undecided honest node samples `k` peers i.i.d. **with replacement, weighted by reputation**
  (`rng.choices(..., cum_weights)`), so a given draw is node `j` with probability `r_j / R`.
- It counts colours in the sample. If some colour has `≥ α` votes (the quorum, `α > k/2`) it adopts
  that colour and increments a **streak**; otherwise the streak resets.
- After `β` consecutive rounds on the same colour it **finalises** that colour (and, with the
  partition mitigation, only if it also sees a reputation quorum — §6).

Reference parameters: `k = 20`, `α = 14`, `β = 12`.

### Threat model
The adversary controls a set of nodes whose reputation sums to a fraction `f = R_adv / R` of the
total. Two behaviours: **fixed** (always report `1`) and **adaptive** (each round report the colour
in the *minority* among honest nodes, to prevent any colour from settling — the worst case for
liveness; it sees the current honest state). The adversary may also be **correlated** (all its
reputation in one trust cluster) — see §5.

Because sampling is reputation-weighted, the probability that any single draw lands on an adversarial
node is exactly `f`, **independent of how many nodes the adversary runs**. This is the formal content
of "power is reputation, not node count" (Art. VI): a Sybil crowd with `r ≈ 0` contributes draw
probability `≈ 0` (verified: `population_sybil`, RESULTS §2).

## 2. One-round vote distribution

Let `q` be the fraction of honest *reputation* currently preferring colour `1`. A single sampled
peer reports `1` with probability

    s = f · 1 + (1 − f) · q          (adversary fixed at 1; honest report their preference)

so the number of `1`-votes in a sample of `k` is `X ~ Binomial(k, s)`. A node registers a `1`-quorum
this round iff `X ≥ α`. Define the per-round quorum probability `Q1(s) = P(Binomial(k, s) ≥ α)`.
Symmetrically `Q0(s) = P(Binomial(k, 1−s) ≥ α)` for colour `0`.

Two facts used throughout (`α > k/2`):
- `Q1(s)` is increasing in `s`, with `Q1(α/k) = 1/2` to first order and `Q1(s) → 0` exponentially as
  `s` falls below `α/k` (Chernoff: `Q1(s) ≤ exp(−k·D(α/k ‖ s))` for `s < α/k`, `D` = KL divergence).
- Therefore a colour can only *gain* ground when its sampled share exceeds `≈ α/k`.

## 3. Safety *(sketch)*

**Claim.** If `f < α/k`, then starting from honest agreement on `0` the network finalises `1` only
with probability exponentially small in `β` (and in `N`). I.e. no conflicting finalisation w.h.p.

**Argument.** Track `q` (honest reputation share at `1`). Its expected one-round change is governed by
how many honest nodes flip to `1` (those that see `X ≥ α`) versus back to `0`. The drift is **toward
`0`** while the `1`-share `s = f + (1−f)q` stays below `α/k`, because then `Q1(s)` is exponentially
small so few honest nodes adopt `1`, while `Q0` is large so adopters of `1` are pulled back. Setting
`s = α/k` gives the **metastability barrier**

    q* = (α/k − f) / (1 − f).

- If `f < α/k`, then `q* > 0`: there is a strictly positive barrier between the all-`0` basin and any
  `1`-majority. From `q = 0`, the process is a supermartingale below `q*`; crossing it requires a
  large deviation of probability `≤ exp(−Θ(N))`. *(This is the standard Snowball metastability
  argument; making the constants rigorous for reputation-weighted sampling is the open part.)*
- Even conditional on a momentary excursion, finalising `1` needs the `β`-streak: `β` consecutive
  rounds each of probability `≤ Q1(s) < 1`, i.e. `≤ Q1(s)^β`. The streak gives an **exponential-in-β**
  safety margin on top of the barrier. This is why `β` is the safety knob.
- If `f ≥ α/k` the barrier vanishes (`q* ≤ 0`): the adversary alone supplies a quorum and safety is
  lost. Hence `f < α/k = 0.7` is **necessary**; with `k=20, α=14` that is the hard ceiling.

**Reconciliation with simulation.** The *measured* capture threshold is `≈ 0.4`, below the `α/k = 0.7`
ceiling. The barrier argument explains why: `q*` shrinks as `f → α/k`, and once `q*` is small enough
that ordinary sampling fluctuations (scale `~1/√k` per node, plus Snowball reinforcement once a few
nodes flip) can cross it, capture becomes practically reachable well before `q*` hits `0`. So the
closed-form gives the *regime* (`f < α/k` necessary; positive barrier ⇒ safe) and the simulation
pins the *practical* threshold inside it. A tight closed form for the crossing probability as a
function of `(k, α, β, N, f)` is left open.

**Adaptive adversary.** Reporting the honest minority colour maximises *churn*, not the `1`-share
beyond `s`: it cannot raise any colour's sampled share above what its reputation `f` allows. So it
cannot lower the barrier below the fixed-adversary case → it **cannot break safety**, only liveness
(§4). Confirmed: capture `= 0%` for the adaptive adversary at all sub-threshold `f` (RESULTS §4).

## 4. Liveness *(sketch)*

**Claim.** With adversary fraction `f`, honest nodes finalise the legitimate value `0` (in expected
`O(β)` rounds after the preference stabilises) iff the honest sampled share clears the quorum:

    (1 − f) ≥ α/k      ⇔      f ≤ 1 − α/k.

**Argument.** From all-honest-at-`0`, the `0`-share is `s0 = 1 − f`. Each honest node finalises after
seeing `β` consecutive `0`-quorums, each of probability `Q0` with `P(Binomial(k, 1−f) ≥ α)`. If
`1 − f ≥ α/k`, `Q0` is bounded away from `0`, so the all-`0` streak completes in expected `O(β/Q0^?)`
rounds (geometric per node) → progress. If `1 − f < α/k`, `Q0 → 0` exponentially: nodes rarely reach
quorum, streaks keep resetting → **stall** (no decision), but note this is a liveness loss, not a
safety loss (no false finalisation — consistent with §3).

With `k=20, α=14`: liveness needs `f ≤ 1 − 0.7 = 0.3`. The simulator shows degradation beginning
`~0.2–0.3` (RESULTS §1) — matching: below `0.3` everyone decides; approaching it, stalls appear first
while capture stays `0`.

**Message loss.** If each queried response is dropped with probability `p`, the effective sample is
`Binomial(k, 1−p)` responses, each `0` with prob `1−f`. Progress needs the expected live `0`-votes to
clear `α`:

    k·(1 − p)·(1 − f) ≥ α      ⇔      α ≤ k·(1 − p)·(1 − f).

For `f ≈ 0` this is the rule of thumb **`α ≤ k(1 − p)`** (≈ 30% loss tolerance at `k=20, α=14`).
Crucially `α` is unchanged, so loss never lowers the safety barrier `q*` → **loss costs liveness, not
safety** (RESULTS §6). This couples the two parameters: raising `α` buys safety margin but lowers the
tolerable loss/adversary for liveness. The `(α, k)` choice is a safety↔liveness trade to set with
real network modelling.

## 5. Independence cap (correlated adversary) — lemma

Reputation-only sampling treats a correlated bloc as if independent. WTC caps at `c` (`cap_cluster`)
the number of sample seats any single trust cluster may occupy.

**Lemma.** A single cluster contributes at most `c` of the `k` votes, regardless of its reputation
share. Therefore an adversary keeping all reputation in one cluster supplies at most `c` `1`-votes,
and `c < α` ⇒ it can **never** reach the `1`-quorum alone, *whatever* its `f`. To force colour `1`
it must occupy `α` seats from distinct clusters, i.e. spread across at least

    ⌈α / c⌉  distinct clusters.

With `c=3, α=14`: `⌈14/3⌉ = 5` clusters (verified: capture stays `0` up to 4 adversarial clusters,
returns at 5+, RESULTS §3). The cap converts "enough reputation" into "enough *independent* reputation",
raising the effective safety threshold against correlated failure.

**Composition with the reputation engine.** Fragmenting into `⌈α/c⌉` clusters that each *look*
independent to community detection is exactly what the reputation engine resists: the anti-collusion
damping and community signal drive a closed/correlated bloc's earned reputation toward `0`
(`reputation-engine/RESULTS.md` §1–2d). So the seats a fragmented adversary could claim are seats it
no longer has the reputation to win. **The two prototypes compose**: WTC's independence sampling rests
on the cluster identification and reputation values the engine produces.

## 6. Partition + network-quorum mitigation

**Failure (found in simulation, not on paper).** During a partition each node samples only its own
group, so the *local* adversary share `f_local` can exceed the global `f`. A long partition can push
`f_local` above the barrier in the small group → that side finalises `1` → on heal the two sides
disagree → **fork** (a genuine safety failure: a 15% global, harmless adversary reaches ~100% fork
under a 90-round partition, RESULTS §5).

**Mitigation.** Condition *finality* on seeing a reputation quorum: a node finalises only if the
reputation it can reach is `≥ Q·R` (e.g. `Q = 0.6`). An isolated minority group reaches
`visible/R < Q` → it **does not finalise, it stalls**, and recovers on heal. Fork drops from ~100% to
~0–2% (RESULTS §5). This is the canonical BFT stance: **when a partition is suspected, prefer to halt
(liveness) over deciding wrongly (safety)** — a CAP-style choice of consistency over availability for
the finalisation step. Open: detecting the seen fraction dynamically (here modelled via groups) under
latency.

## 7. Summary of regimes (`k=20, α=14, β=12`)

| Property | Condition | Knob | Simulation |
|---|---|---|---|
| Safety (no false finality / fork) | `f < α/k = 0.7` necessary; barrier `q*=(α/k−f)/(1−f)>0`; streak gives `≤ Q1^β` | `β`, `α` | capture starts `~0.4` |
| Liveness (all decide) | `f ≤ 1 − α/k = 0.3` | `α`, `k` | stalls start `~0.2–0.3` |
| Liveness under loss `p` | `α ≤ k(1−p)(1−f)` | `α/k` | safe ≤ ~30% loss |
| Correlated adversary | needs `⌈α/c⌉` independent clusters (`c=cap`) | `cap` | `⌈14/3⌉=5` |
| Partition | local `f_local` can break safety; quorum `Q` halts instead of forking | `Q` | fork ~100%→~0% |

## 8. What remains for a machine-checked proof
- Rigorous constants for the metastability barrier under **reputation-weighted, with-replacement**
  sampling (here argued via the standard Snowball drift/supermartingale picture).
- A closed form for the capture probability inside `f < α/k` as a function of `(k, α, β, N)`.
- Beyond the binary decision: multi-value finalisation and concurrent decisions.
- Encoding §3–§6 as TLA+ safety/liveness properties and model-checking small instances (TLC), then a
  Coq/Isabelle proof of the invariants for the general case.

These are the next steps for `ROADMAP`/PAPER §10; the simulator already exercises every regime above
as executable regression (`tests/test_consensus.py`).
