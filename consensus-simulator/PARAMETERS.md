# The story of the numbers — Woven-Trust Consensus parameters

> No magic constants. Every number here is **derived** from a stated security goal and a real
> constraint, and where it is natural, it **echoes what Harlequin is**. A protocol meant to free
> people from the State cannot rest on numbers copied from a paper; the numbers must mean something.
> Companion to `ANALYSIS-safety-liveness.md` (the math) and `LORE.md` (the myth).

## The promise the numbers must keep
> **No faction can impose a false truth on Harlequin unless it commands an overwhelming share of
> *earned reputation*; and even then, every honest node must confirm a verdict many times before it
> becomes irreversible. The network keeps judging as long as the honest majority of reputation can be
> reached.**

That sentence is the design. Each parameter is a measurable piece of it. (Symbols: ♦ ambition,
♥ love/family, ♠ war/struggle, ♣ freedom — the four suits, `LORE.md`.)

## The quorum ratio — α/k = 14/20 = 0.70  ·  the wall ♠
A value is only accepted in a round when **≥ 70 % of the sampled reputation** agrees on it.
- **Why 0.70 and not a bare majority (0.51):** safety holds while the adversary's reputation
  fraction `f < α/k` (`ANALYSIS §3`), and a bare majority leaves the metastability barrier
  `q*=(α/k−f)/(1−f)` razor-thin — ordinary fluctuations cross it. A 0.70 bar keeps `q*` wide:
  capture requires storming a high wall, not nudging a coin. Practical capture threshold lands near
  **f ≈ 0.40** (confirmed by simulator and test-rig).
- **The story:** truth in Harlequin is **not a 51 % vote**. It is near-consensus. A simple majority
  is how the State manufactures "legitimacy"; here a minority of power — however large — cannot
  declare what is true. This is the wall the adversary must take, and loses.
- Status: **PARAMETER** (ratio fixed at 0.70 as the principled floor; exact k, α below).

## The sample size — k = 20  ·  it runs on a phone ♣
Each node asks **20** peers per round.
- **Why small:** Harlequin must run on a **phone** (light clients, SPEC §2.3 — freedom means no need
  for a datacenter). Cost per round scales with k, so k stays small.
- **Why not smaller:** with the 0.70 bar, the Chernoff separation between an honest supermajority
  (sampled share ≈ 1) and a capture attempt (`f < 0.4`) needs enough samples to make a false quorum
  exponentially rare; `k = 20` is the knee where that holds while staying phone-cheap. `k = 5` is too
  noisy; `k = 100` buys little and costs bandwidth the poorest member cannot spend.
- **The story:** power must be **reachable by the weakest hands**. The sample is small on purpose so
  that freedom is not a privilege of those with big machines.
- Status: **PARAMETER** (k=20 stand-in; finalise with real network modelling, task #18).

## The finality depth — β = 12  ·  judgment is not hasty ♥ + the twelve
A node finalises only after **12** consecutive rounds of the same supermajority.
- **Why:** a wrong finalisation needs the unlikely wrong-quorum to repeat β times: probability
  `≤ Q1(s)^β` (`ANALYSIS §3`). Below the wall each wrong round is already < ½, so β=12 drives the
  chance of an irreversible mistake toward `2⁻¹²` and far lower with the barrier. β is the **safety
  dial**: it trades a little latency for near-certainty.
- **The numbers:** with `Q1(s)=P(Binomial(k,s)≥α)` the one-round chance of a wrong-colour quorum at
  sampled wrong-share `s`, a finalisation needs it `β` times → `≤ Q1(s)^β`:

  | wrong-share s | Q1 (one round) | Q1¹² (β=12) |
  |---:|---:|---:|
  | 0.50 | 5.8 % | 1.4·10⁻¹⁵ |
  | 0.60 | 25 % | 6.0·10⁻⁸ |
  | 0.65 | 42 % | 2.7·10⁻⁵ |
  | 0.70 (the wall α/k) | 61 % | 2.5·10⁻³ |

  So β=12 buys **astronomical** safety while the adversary cannot push the sampled wrong-share above
  ~0.5 (i.e. f comfortably below threshold) — and the formula tells you the dial: to hold even at the
  metastable edge (worst case Q1≈0.5) to 10⁻⁶/10⁻⁹ you raise β to ~20/~30. β=12 is the operating-regime
  choice; β is the lever for edge-hardening.
- **The resonance:** the Constitution has **twelve articles** (the manifesto). The network confirms
  a verdict **twelve times** before it is permanent — one confirmation per article it must not betray.
  Judgment is deliberate, never rushed: irreversibility is earned, like reputation itself.
- Status: **PARAMETER** (β=12 for the operating regime; raise toward 20–30 for edge-margin; confirm
  against target finality time).

## The committee size — τ = 60  ·  the rotating jury ♠ + ♥
Each epoch, VRF sortition elects an expected **60** seats, weighted by reputation, and the committee
**rotates** every epoch.
- **Why ~60:** the committee must keep an honest supermajority with high probability even when the
  global adversary fraction approaches the threshold. By Chernoff on the Poisson sortition, τ≈60
  (= 3·k) makes the chance that a sub-threshold adversary captures a committee negligible, while
  giving the k=20 sample real diversity. Smaller τ → committees too easy to swing by variance;
  larger τ → more messaging for little gain.
- **Why rotation:** Art. VI — **no one keeps power**. A fresh beacon re-draws the jury each epoch, so
  the validating role cannot be hoarded (anti-entrenchment, the anti-State principle in code).
- **The story:** justice is a **rotating jury drawn by lot, weighted by earned trust** — never a
  standing court, never the same hands. The ♠ that judges and the ♥ that serves the community, handed
  on each epoch.
- Status: **PARAMETER** (τ=60; finalise with sortition-failure modelling, task #18).

## Empirical confirmation (test-rig sweeps, 2026-06-15)
Sweeps with `testrig/` (committee + async network) locate the operating point — not guessed:

**Quorum α (k=20, τ=60):** raising α hardens safety but costs liveness — the trade is real and measured.

| α (ratio) | capture @ f=0.4 | capture @ f=0.5 | stall @ f=0.15 |
|---:|---:|---:|---:|
| 12 (0.60) | 75% | 100% | 0% |
| 14 (0.70) | 75% | 100% | 25% |
| 16 (0.80) | 25% | 75% | 88% |

α/k = 0.70 sits where safety is strong without strangling liveness; pushing to 0.80 makes the network
stall even against a tiny adversary. The 0.70 wall is a balance point, not a round number.

**Committee size τ (k=20, α=14):** τ does **not** move the safety threshold (capture is 0% at f=0.3
and 100% at f=0.45 for every τ — the threshold sits at **~0.40 reputation**, matching the simulator and
the analytical practical threshold). τ is the **liveness / variance knob**: a bigger committee damps
sortition variance, removing spurious stalls.

| τ | capture @ f=0.4 | stall @ f=0.15 |
|---:|---:|---:|
| 60 | 75% | 25% |
| 150 | 62% | 12% |
| 200 | 75% | 0% |

So τ is chosen large enough that committee-composition variance does not cause false stalls, traded
against per-epoch message cost — to be fixed against the real network size. **The safety threshold
(~0.40) is a property of α/k and β, confirmed independent of τ.**

## Where the math lives
The inequalities behind all of the above — safety `f < α/k`, liveness `f ≤ 1 − α/k` and
`α ≤ k(1−p)(1−f)` (tightening to `α ≤ k(1−p)²` under round-trip loss), the independence bound
`⌈α/cap⌉` — are derived in `ANALYSIS-safety-liveness.md` and exercised by `wtc_sim/` (13/13) and
`testrig/` (9/9). This file is the **why**; that file is the **proof sketch**; the code is the
**measurement**.

## The four dimensions of reputation are the four suits — CANON
Reputation is **vectorial** (SPEC §1.2b): the engine uses four dimensions. They are not an arbitrary
list — they are the **four suits of Harlequin** (`LORE.md`). Canonical mapping (blessed by Chief,
2026-06-15):

| Suit | Meaning | Reputation dimension | Why |
|---|---|---|---|
| ♦ Diamond | Ambition | `commerce` | enterprise, trade, the drive to build and prosper |
| ♣ Club | Freedom / Independence | `technical_contribution` | building the tools and infrastructure that free people |
| ♠ Spade | War / Struggle | `judicial_function` | defending the society, judging wrongs, the fight |
| ♥ Heart | Love / Family | `governance` | stewardship of the community, the bonds that hold it |

Every place that lists the dimensions carries the suit, so the architecture itself tells the story:
**your standing in Harlequin is measured in the four suits you earn.** The
conservative aggregate (min over suits, §1.2b) then reads: *to wield power over the whole, you must be
trusted in every suit — you cannot buy war-standing with ambition alone.*

## What is still open (and must get its story too)
The reputation engine's own constants — anti-collusion strength `μ`, evidence-deficit `ρ`, the
volume gate `k₀`, the EigenTrust teleport `α=0.30`, inactivity decay, the independence `β/γ` — are
currently set from empirical tuning (`reputation-engine/RESULTS.md`). Each needs the same treatment:
a stated goal, a derivation, and its resonance. Tracked as follow-on work; they are **PARAMETERs**,
not truths, until grounded.
