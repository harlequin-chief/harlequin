# Architecture — how Harlequin fits together

Harlequin is a voluntary order of cooperation among sovereign individuals: a place to cooperate and
trade **beyond the State**, where no central authority decides what is true or who holds power. This
document is the map — how the pieces stack, which principle each serves, and where the code is.

## The one bet everything rests on
> **Power is earned reputation — it cannot be bought, faked, or seized.**

Not proof-of-work (burned electricity), not proof-of-stake (the rich rule), not one-person-one-vote
(Sybil-farmable). In Harlequin your weight comes from reputation earned by real, verified deeds
(Manifesto Art. V–VI). If that can be gamed, everything above it collapses — so the foundation is
built first, and built to be unbreakable.

## The stack (bottom carries the top)

```
  ┌─────────────────────────────────────────────────────────────┐
  │  CLIENT — a PWA, light client to the network (the window)    │  future
  ├─────────────────────────────────────────────────────────────┤
  │  LAYERS — identity · two currencies · justice · market ·     │  future
  │           mutual aid · private chat / forum                  │
  ├─────────────────────────────────────────────────────────────┤
  │  CHAIN — sovereign Substrate solochain, custom WTC consensus │  in progress
  ├─────────────────────────────────────────────────────────────┤
  │  CONSENSUS — reputation-weighted VRF sortition committees    │  validated
  ├─────────────────────────────────────────────────────────────┤
  │  REPUTATION ENGINE — the root of trust (anti-collusion)      │  validated
  ├─────────────────────────────────────────────────────────────┤
  │  MANIFESTO — the principles, few/clear/unbreakable           │  published
  └─────────────────────────────────────────────────────────────┘
```

### Manifesto — the foundation (Art. I–XII)
The constitution of the society: twelve articles, public, sealed at chain launch. It fixes principles
and limits, never mechanisms. Everything below is the manifesto **made executable**.

### Reputation engine — the root of trust  (`reputation-engine/`, validated · `chain/reputation-core/`, Rust)
Computes each pseudonym's **vectorial** reputation (the four suits: ♦ commerce, ♣ technical, ♠ judicial,
♥ governance) from objective evidence + a vouch graph, and **defends it against gaming**: Sybils get
~0, collusion rings get ~0, directed funnels are cut — across dense, scattered, adaptive, asymmetric
and cross-dimension attacks (Art. V). Anchored in evidence, so vouches amplify real work, they don't
replace it (Art. VI). Ported to Rust at parity with the validated prototype.

### Consensus — agreement without a State  (`consensus-simulator/`, validated · `chain/consensus-core/`, Rust)
"Woven-Trust Consensus": each epoch a committee is elected by **VRF sortition weighted by reputation**
and rotated (Art. VI, no entrenchment), reaching finality by sub-sampled voting. The security threshold
is a fraction of **reputation, not of nodes** — a Sybil crowd of 93% of nodes does not break it.
Derived safety/liveness regimes + a faithful test-rig (async network, partition) back it.

### Chain — the load-bearing structure  (`chain/`, in progress)
A sovereign **Substrate** solochain (Rust) running WTC. Inputs on-chain (evidence + vouch graph),
reputation **derived** and recomputed each epoch (verify-by-recompute, no trusted oracle). Light
clients so it runs on a phone (Art. on accessibility). Being hardened for on-chain use (deterministic
fixed-point arithmetic). See `chain/PALLET-DESIGN.md`.

### Layers — the society itself (future)
On the trust substrate: **identity** (pseudonymous, Art. VII) · **two currencies** (a scarce store of
value + HLQ for daily use) · **justice** (juries by reputation, separated powers, Art. IX) ·
**market** (trade sustained by reputation) · **mutual aid** · **private chat / forum**.

### Client — the gathering place (future)
A **PWA** that is a **light client to the network**, not a front-end to a central server — so there is
no single point the State can seize or censor. The market and the forum live on the chain / P2P; the
app is only the window. Packageable as a native shell, never dependent on an app store.

## How they compose
Reputation feeds everything: the consensus committee, the justice jury, market trust, and the right to
vouch all read the same reputation, aggregated conservatively (the minimum over the four suits — you
cannot buy authority in one suit with another). The end-to-end claim is runnable today:
`cargo run -p composition` shows a collusion ring + a Sybil crowd (the numeric majority) winning **zero**
committee power.

## Principles that shape every choice
- **No central chokepoint** — nothing the State can seize, deplatform, or censor (no third parties in
  front, no app-store gatekeepers).
- **Runs on a phone** — power must be reachable by the weakest hands, not a privilege of big machines.
- **Built in the open** — AGPL-3.0, every step public and verifiable (Art. X). Nothing trusted that
  cannot be re-checked.
- **Quality over speed** — the foundations carry money and justice; they are made exact and unbreakable
  before anything is built on them.
