# Woven Trust Consensus

### A security that cannot be bought: earned reputation, over time, as the foundation of a chain

*Draft — version 0.1 · 2026-06-13 · native English text (not a translation of the Spanish; written in
English, as the manifesto was). The Spanish companion lives in `PAPER-confianza-tejida.md`.*

> This paper accompanies code that already runs (`prototipos/reputacion/`) and the design that precedes
> it (`DISENO-CONSENSO-TEJIDO.md`). It keeps Harlequin's voice: plain, sober, free of urgency and idle
> ornament. Yet it hides nothing it does not know. **Where a claim rests on construction or on the
> prototype, it is stated firm; where it is a design wager, it is confessed a wager; where it is an
> unsolved problem, it is named open.** Nothing is claimed that has not been shown.

---

## Proem — the cloth of the costume

The harlequin's costume is a cloth of diamonds, each one stitched to the next. No single patch, however
bright its colour, holds the garment together; what holds it is the way they are all sewn. A loose
scrap gives no warmth; a patch stitched only to itself comes away at the first pull.

So we wanted the network. Let its strength live in no single piece — not the richest, not the most
powerful — but in the weave: in the trust each member earns before the others, and that the others, of
their own free will, grant back. The chains that exist today entrust their security to a good that can
be bought. We propose another thing: to entrust it to a good that can only be earned — slowly, before
the judgement of equals — and that is lost if abandoned. We call it **Woven Trust Consensus**.

◆

## 1. The question every chain answers

Every blockchain answers, whether it says so or not, a single question: **what scarce good keeps an
attacker from seizing the network?** The answer decides who rules.

- In **Bitcoin** (proof of work) the good is **computation**. Security grows with the machine and the
  current. Hence its drift: a few large miners, and a cost paid in kilowatts.
- In **Ethereum** (proof of stake) the good is **capital**. Security grows with the money locked away.
  Hence its root: power is bought.

Both share a flaw that Harlequin cannot accept, for its foundation holds that "the weight of each one
in the common decisions is born of their reputation, and of it alone; neither wealth, nor the number of
machines, confers authority over others" (Manifesto, Art. VI). In both, **power is merchandise**: a
thousand coins buy a thousand machines, or a thousand tokens, today, all at once and in parallel.

We propose another foundation. Harlequin's scarce good is **earned reputation**, which holds at once
three virtues that neither computation nor capital possess together:

1. **It does not transfer** — it is neither bought nor sold (Art. V); there is no market in which to
   acquire power.
2. **It is earned over time** — it asks for validated work and for the vouching of peers who stake
   their own name; money does not hasten it.
3. **It fades when abandoned** — it decays without continued contribution; it is no hoard to be stored.

From here the thesis: a chain whose security is measured in **reputation-time** changes the cost of an
attack, which ceases to be "a cheque" and becomes "a patient, sustained infiltration" — far costlier,
far slower, and far more visible.

◆

## 2. What it must meet

Inherited from the project's principles (SPEC §0, Manifesto):

- **It runs on any device**, from a phone to a modest machine. Light clients.
- **No proof of work.** No computation spent in competition.
- **A very wide mesh** of many small nodes, not a few enormous centres.
- **Power is reputation**, never wealth nor the count of machines (Art. VI).
- **Resistance to the false multitude** (Sybil) without economic or computational cost: anchored in
  reputation.
- **Faithful to the manifesto:** pseudonym (Art. VII), the right of exit (Art. VIII), light upon what
  is common (Art. X), a neutral base (Art. XI), the permanence of what is written (Art. XII).

◆

## 3. What others already said (the plain account)

Honesty is owed: this consensus **recombines** known pieces; it does not invent them.

- **Sortition by verifiable randomness (VRF)** — Algorand. Here weighted by **reputation**, not capital.
- **Voting by repeated sampling** — Avalanche. Here the sample is weighted by **reputation and
  independence** within the weave.
- **Graph reputation** — EigenTrust, PageRank and their weaknesses before the false multitude. Here,
  anchored in evidence and damped against collusion.
- **Web of trust** — PGP, BrightID. Here made the **substrate of consensus**, not a mere matter of
  identity.
- **Proof of unique personhood** — Idena, pseudonym parties. Here it grants the **base citizenship**,
  not power.
- **Reputation as proof** — it exists in the lesser literature, yet it tends to derive reputation from
  capital or activity, and often transferable. Here it is required **non-transferable, contextual,
  anchored and perishable**.
- **Two anchored layers** — Bitcoin and its fast layer. Here the fast layer is **routed by trust**, not
  by chance.

What is this consensus's own (§9) is not these pieces, but **the foundation** — reputation-time —, **the
weave** — the trust graph as topology — and **decay** as a twofold defence.

◆

## 4. The substrate: reputation

The consensus rests on a reputation engine already built and tested (`prototipos/reputacion/`).

**It is contextual.** One vector per pseudonym across distinct domains — commerce, technical work,
judicial function, governance —, extensible. To be a good trader makes no one a judge. For functions
that demand whole reliability, aggregation is **conservative** — minimums, medians, never a sum —: one
does not buy integrity with skill.

**It is anchored in the work.** Reputation is computed as the rest of a trust process (EigenTrust) that
teleports back to a **fund of real evidence** — settled deals, verified labour —: `t = (1−α)·Cᵀ·t +
α·p`, where `p` is that evidence. **Without verifiable work, no game of vouching manufactures power.**

**It damps collusion.** The reciprocal, endogamous vouch — the ring that votes for itself — is worth
almost nothing (SPEC §1.6). The detail that makes it work: normalization is done by the *undamped* sum,
so an endogamous node emits less than it receives, and its unpropagated trust **leaks back to the fund
of evidence** instead of circulating within the ring. (To normalize otherwise would cancel the effect:
a uniform factor divides out. This was a real error, found and corrected in the prototype, and a test
now guards it. It is told because to hide it would betray this house's voice.)

**It decays.** Reputation fades with inactivity: its influence after a silence wanes like `e^{−λt}` —
exact shape and rate yet to be set.

**What the prototype already shows** (reproducible, no dependencies):

- Two hundred false identities, forty per cent of the network, capture **zero** power.
- A ring of thirty vouching in a circle stays at **near nothing**, even with three deceived honest
  members vouching for it.
- The laundering of one legitimate reputation toward twenty-nine puppets is cut **ninefold**.
- Whoever abandons their pseudonym to start clean inherits **zero**.
- The protégé's fraud strikes upward, at whoever vouched for them; the stranger, it does not touch.

◆

## 5. The mechanism, weave by weave

Return to the image. Each diamond is a member and their reputation; each seam, a vouch. The strength
lives in the stitching.

**The diamond exists (identity).** One person, one base citizenship (proof of unique personhood,
pluggable, SPEC §1.5). It grants no power — base of one, earned reputation of zero —; it grants entry,
trade, and the start of a record.

**The thickness of the diamond (reputation epoch).** Each epoch, the engine recomputes the vector by
the same rule, from history already closed (§7). All honest nodes obtain the same result: they agree on
who weighs how much, with no central scribe.

**The drawing of the weavers (validators).** Each epoch, a **rotating** committee is drawn by
verifiable randomness (VRF), with probability set by the conservative aggregate of reputation. Rotation
is compelled: no function takes root (Art. VI). The false multitude, with reputation zero, never
enters. And the randomness is verifiable: no one can "grind for luck" until their turn comes.

**Finality (voting by trust-weighted sampling).** To run on a phone and across the wide mesh, finality
is reached by repeated sampling, in the manner of Avalanche. The own contribution: the sample is **not
uniform**, but weighted by reputation and by **independence from those already sampled**, so that a
dense ring is not over-represented in the query. It is a design wager: it requires analysis of the fork
probability (§10).

**The woven fast path (daily use, HLQ).** A deal between two members within a neighbourhood of trust
receives a soft, swift confirmation from the reputed nodes that already know them, without awaiting
global finality; firm finality is sealed by the settlement layer periodically. It is commerce as it
truly happens: one trusts sooner, and better, those whom one's own network already vouches for.

**Settlement (store of value).** A conservative layer, of strong finality and lesser throughput; it
keeps the canonical state — reserve balances, the root of reputation, identities, verdicts — and
anchors the coin of use. The security of both layers is the same woven trust, not labour nor capital.

◆

## 6. The measure of cost: reputation-time

**The unit.** We measure the cost of an attack not in money nor in computation, but in
**reputation-time**: the reputation the attacker must govern, multiplied by the time and the social
effort it costs to earn it for real before the honest network.

**The bound (informal).** Let `R_h` be the total honest reputation, and suppose that to command a
fraction `f` of consensus requires commanding about `f·R_h` of reputation. By what was said in §4:

- An attacking group's reputation springs **not** from its internal vouches — damping reduces them to
  nothing — but from the **legitimate, external flow** it receives from independent, reputed members.
  Its reputation is therefore **bounded by that external flow**, not by its number nor its inner mesh.
  The prototype evidences this: rings at near nothing, laundering cut ninefold.
- To gather that flow in proportion to `f·R_h` compels independent honest members to vouch for them
  **at the risk of their own name** — that is, the attacker must have **contributed real work over a
  span of time** long enough to deserve it.
- Money hastens none of this: reputation is not bought; evidence is checked; endogamous vouches cancel.
- And decay compels one to **sustain** the work: the attack is no single outlay, but a continuous
  effort.

**What follows** — a design wager, its core already backed by the prototype —: the cost of a majority
attack is of the order of **reproducing, with independent identities and across time, a fraction of the
honest social effort**. It is not bought, not computed, not parallelized with capital, and it betrays
itself: a torrent of vouches toward a newly arrived group is visible in the public graph (Art. X).

**The reckoning of threats, one by one:**

| Threat | Defence | Status |
|---|---|---|
| False multitude (Sybil) | born with reputation zero; the true wall is earned reputation, not the proof of personhood | firm (prototype: zero) |
| Collusion (ring of vouches) | damping by independence (leak to the fund of evidence); judicial slashing; the voucher's persistent liability | partial (dense rings defeated; subtle collusion, open, §10) |
| Majority (51%) | requires 51% of reputation-time: independent, sustained social work, not for sale | wager (informal bound §6; no formal proof yet) |
| Long range (old keys) | the reputation of old keys is decayed to nothing; to rewrite history asks for reputation that no longer exists; checkpoints signed by the present reputed committee | firm by design (decay covers it) |
| Nothing-at-stake (voting on several forks) | voting against the evidence is punished (Schelling point, SPEC §4); the name at stake deters | design wager |
| Grinding the randomness | verifiable randomness; reputation frozen from the prior epoch, not manipulable in flight | firm |
| Vote buying | reputation does not transfer (the seat is not sold); to sell one's vote risks the seller's name | wager (costlier than under staking; not impossible) |
| Censorship | neutral, unstoppable base layer (Art. XI, the doctrine of layers, SPEC §4c); a mesh without a centre | by design |
| Node isolation (eclipse) | a wide mesh and broad sampling make isolation hard; finality resumes on reconnection | wager (analysis pending) |

**Liveness and safety.** Finality is **probabilistic**, as in Avalanche; committee rotation ensures
progress while enough honest reputation is online. How much adversary is tolerated, and with what fork
probability, remains **open** (§10).

◆

## 7. The circle, and how it is broken

Reputation decides consensus, yet it is computed from data that consensus must close. A circle. It is
broken by time: a given epoch's reputation is computed, by a fixed rule, **from the already-closed
history of the previous one**. Each epoch looks backward; no snake bites its tail. The **genesis** sows
the first trust with a founding cohort **public and made to dilute** — no perpetual founder's privilege,
Art. V and VI —. And, in passing, this shuts the door on grinding the randomness within the epoch in
flight.

◆

## 8. The two coins, and the power that is not wealth

- **Store of value.** A fixed cap, an emission that goes quiet, no printing at will. To save and to
  settle.
- **Coin of use (HLQ).** A medium of exchange of large throughput. Its stability is born of **liquidity
  and low friction**, not of a promised parity (SPEC §3.2): value is kept in the reserve and moved in
  the coin of use; as it is held only briefly, little is exposed to its swing. No parity, no spiral, no
  price oracles.
- **A fair launch.** No pre-allotment to founders; a trickle per unique person and per work. Early
  concentration of wealth matters less, because **power is not wealth** (Art. VI): the rich do not rule.
  Security and money are **decoupled at the root** — capital does not turn into consensus power.

◆

## 9. What is ours (without inflating)

We claim neither verifiable randomness, nor sampled voting, nor EigenTrust, nor the web of trust, nor
the proof of personhood, nor the two layers. All of that belongs to others.

We claim as contribution:

1. **The foundation in reputation-time:** a security good that is not bought, not computed and not
   hoarded, as the **sole** root against the false multitude and as the base of consensus, with an
   attack cost measured in time and social independence.
2. **The trust graph as the topology** of consensus — the weave —: sampling weighted by reputation and
   independence, and a fast path by neighbourhood of trust. Not a flat sampling.
3. **Decay as a twofold defence:** against the entrenchment of power (Art. VI) and against the
   long-range attack, with a single mechanism.
4. **Non-transferable, contextual, anchored and damped reputation** as the substrate of consensus — not
   as a social layer apart —, already validated at its core by code that runs.

The originality lies in the **foundation and the synthesis**, not in new pieces. For a chain whose
premise is that power cannot be bought, that foundation is, indeed, unlike that of any large chain in
use — and, what matters most, **it is born of the manifesto, not added afterward.**

◆

## 10. What we do not know (without varnish)

1. **Subtle collusion is the front.** The prototype beats the dense ring; real collusion is subtle —
   scattered rings, lopsided vouches, attacks in the manner of those PageRank suffers, slow schemes
   that mimic the honest. Unsolved. From here hangs the security of all the consensus.
2. **Formal proofs are lacking** of safety and liveness under weighting by reputation and independence.
   Algorand and Avalanche have them; this variant must be analyzed.
3. **The cost bound (§6) is informal.** It remains to formalize the external flow and the role of
   damping and decay in a provable model.
4. **Proof of personhood against AI** is a race: pluggable and graduated.
5. **The privacy of the weave:** a public graph re-identifies patterns. Explore zero-knowledge proofs
   — to show "I hold reputation enough" without revealing the weave.
6. **The allotment of block space without a pure auction** — which would favour the rich, against Art.
   VI —: open.
7. **The bootstrap** without the genesis taking root: to encode its decay.

◆

## 11. How it is checked (not promised)

True to "quality over haste," **the chain is not written until the model is validated**:

1. *Done.* The reputation engine and the attack simulations (`prototipos/reputacion/`, eight of eight
   tests green).
2. *Next.* Harden the engine against subtle collusion — scattered rings, temporal dynamics with decay
   per epoch —. The first front.
3. *After.* A **consensus simulator** upon the engine: a network of many nodes, drawing by reputation,
   trust-weighted sampling, and the measure that matters: **how much reputation-time it takes to fork
   the chain**, and how long finality takes.
4. *Only then*, choose the building tool and begin the chain. While the simulator shows no reasonable
   tolerance to the adversary, the chain is not written.

◆

## Epilogue

*Bitcoin made the attack dear with energy; Ethereum, with money; Harlequin makes it dear with time and
with earned trust — a good that is not bought, not computed and not hoarded, woven into a graph of
reputation that is, at once, the society and its consensus. The proposal is ambitious, and honest: its
heart already beats and is measured; its formal proof and the subtle collusion lie ahead. What is
offered here is not a certainty, but a foundation — and the will to test it before building upon it.*

---

### References
- Gilad, Hemo, Micali, Vlachos, Zeldovich — *Algorand* (SOSP 2017): sortition by verifiable randomness.
- Rocket, Yin, Sekniqi, van Renesse, Sirer — *Avalanche* (2019): sampled voting and metastability.
- Kamvar, Schlosser, Garcia-Molina — *EigenTrust* (WWW 2003): graph reputation.
- Page, Brin, Motwani, Winograd — *PageRank* (1999): and its weaknesses before the false multitude.
- Ford — *Pseudonym Parties* (2008): in-person proof of personhood.
- Nakamoto — *Bitcoin* (2008): cap and emission; proof of work as contrast.
- Buterin et al. — *Ethereum / Casper*: proof of stake as contrast.
- Friedman, *The Machinery of Freedom*; Benson, *The Enterprise of Law*: legal order without a centre.
- Harlequin project — `MANIFIESTO.md`, `SPEC.md`, `prototipos/reputacion/`.
