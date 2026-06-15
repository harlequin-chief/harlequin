# The story of the numbers — reputation engine

> Companion to `../consenso/PARAMETERS.md`. Same rule: **no magic constants**. Every knob of the
> reputation engine has a job, a value chosen for a reason, and — where natural — a resonance with what
> Harlequin is (`LORE.md`: the four suits, the struggle against the State). These are PARAMETERs:
> tuned for the prototype's target behaviour, to be re-derived with real data before mainnet. The
> point is that none is arbitrary.

## The anchor weight — α = 0.30  ·  trust always falls back to reality
EigenTrust teleport: each propagation step returns a fraction **α = 0.30** of trust to the **pre-trust**
(the objective evidence anchor: settled deals, proven work — §1.3a).
- **Why:** α is the leash between propagated trust and provable reality. Too low (→0) and reputation
  becomes a pure popularity graph a ring can inflate; too high (→1) and vouches stop mattering at all.
  0.30 keeps ~a third of all trust re-anchored to evidence every step — vouches amplify real work, they
  do not replace it.
- **The story:** the State manufactures legitimacy **by decree**, detached from anything real. Harlequin
  refuses that: **you cannot vouch your way to power detached from real work.** α=0.30 is that refusal,
  made numeric. (`reputation.py`)

## The inbreeding penalty — β = γ = 4  ·  a closed ring is worth almost nothing
`independence(i→j) = 1/(1 + β·reciprocal + γ·overlap)`.
- **Why these values:** a fully inbred vouch (mutual, β·1, with high neighbour overlap, γ·1) collapses to
  `1/(1+4+4) ≈ 0.11` — worth a ninth of a vouch between independent strangers (≈1). Strong enough that
  a closed cluster cannot farm power, gentle enough that ordinary overlap (friends-of-friends) is barely
  touched. β=γ keeps reciprocity and cluster-overlap equally suspect.
- **The story:** trust that only circles among the same hands is nearly worthless — like a regime
  endorsing itself. (`graph.py:independence`)

## The community brake — κ = 0.5  ·  a quiet clique that only vouches itself
Community-suspicion damping: edges inside a community with many internal vouches and little evidence are
scaled by `1/(1+κ·suspicion)`.
- **Why 0.5:** enough to choke a scattered ring (it cuts laundering measurably, `RESULTS.md §2b–c`)
  without punishing the honest mega-community, whose evidence keeps its suspicion low.
- Status: PARAMETER. (`graph.py:damped_local_matrix`)

## The funnel signal — μ = 8, ρ = 2, k₀ = 26  ·  no pumping an unearned idol
In-degree-concentration damping against the PageRank funnel (§2d):
`1/(1 + μ·conc²·dom·gate·deficit)`.
- **Why these values:** chosen so the funnel onto a no-evidence target is cut **~70–80 %** while honest
  members keep **≥85 %** (`RESULTS.md §2d`). μ=8 sets the bite; ρ=2 sets the evidence-deficit guard;
  k₀=26 makes the volume gate ignore low-degree members (a funnel needs many feeders). Tuned against the
  false-positive frontier, not picked round.
- **The deficit is PER-DIMENSION (per-suit):** the guard uses the target's evidence **in that suit**,
  not its total. So a funnel in a suit where the target earned nothing is cut **even if the target is
  established in another suit** — closing the cross-dimension funnel (the old §2d residual). k₀ was
  raised 8→26 precisely because the per-suit deficit is stricter: the larger volume gate keeps honest
  members (vouched by their sponsors in a suit before they have evidence there) above 85 % retention.
- **The story:** you cannot manufacture an idol by funnelling a crowd's praise onto someone who **earned
  nothing** — and **you cannot buy authority in one suit with another**. Standing is earned per suit,
  not pumped. (`graph.py:in_concentration_signals`)

## The inactivity decay — 0.9 per epoch  ·  earned continuously, never inherited
Uncontributed reputation decays by **0.9** each epoch.
- **Why:** ~10 % erosion per idle epoch makes farm-and-sit unprofitable and keeps power from ossifying,
  without punishing brief absences harshly.
- **The story:** Art. VI — **no entrenchment**. Reputation is earned **continuously**, never inherited
  and never rested upon. Where the State breeds a permanent ruling class, Harlequin lets idle power
  fade. (`reputation.py:decay`)

## The sponsorship economy — quota k = 3 (log₂), dividend 0.05, slash ½ per hop, depth 3
- **Live-vouch quota `⌊3·log₂(1+rep)⌋`:** sublinear on purpose — more reputation buys more sponsorship
  but with **diminishing returns**, so no one monopolises who gets in (`vouch.py:vouch_quota`).
- **Mentor dividend echo = 0.05:** a sponsor earns a *small* (5 %) echo of the protégé's **independent**
  reputation — enough to reward genuine mentorship, too small to make sponsoring puppets pay.
- **Cascade slash ½ per hop, depth 3:** if a protégé defrauds, each sponsor up the chain loses **half**
  of what the one below lost, three hops deep. **The story:** you answer for whom you bring in. Trust is
  a chain of responsibility, not a free favour — the opposite of a State that shields its own.

## What this buys
Together these make the engine's verdict (`RESULTS.md`): Sybils ~0 power, collusion rings ~0, funnels
cut, whitewashing loses everything, careless sponsorship is costly — **structural power cannot be bought
or faked, only earned.** Each number above is a measurable piece of that promise. All are PARAMETERs to
be re-derived with real network data; none is arbitrary.
