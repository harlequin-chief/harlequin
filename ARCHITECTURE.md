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
clients so it runs on a phone (Art. on accessibility). Deterministic fixed-point arithmetic throughout.
See `chain/PALLET-DESIGN.md`.

The node now authors under Woven-Trust: each slot it runs the reputation-weighted sortition
(`consensus-core`) over the chain's own reputation, read through a runtime API (`consensus-api`), and
only seals when elected — no placeholder seal. The **manifesto is sealed into the genesis block**
(text + SHA-256, no mutator extrinsic, Art. XII) by `pallet-manifesto`, on the shared SHA-256 of
`manifesto-core`; the founding cohort's reputation is derived at block 0. The voting core itself is
ported and **cross-validated 13/13** against the reference simulator (Sybil, correlated blocs, adaptive
adversary, loss, partition). Finality is a **networked Snowball gadget**: committee members sign their
votes (sr25519, bound to their reputation account) and only votes from the reputation-elected committee
count, so an unbacked node cannot move finality; the justification carries that signed-vote proof and is
**verified on import**, so a peer cannot make a node accept finality the committee did not earn; those
proofs are **gossiped**, so non-voting and catching-up nodes finalise from verified proofs alone; the
committee **rotates each epoch** (Art. VI); and finality advances by range so it keeps pace with the head.
Validated multi-node (no fork; finality halts rather than forking when committee quorum is lost).

### Layers — the society itself (future)
On the trust substrate: **identity** (pseudonymous, Art. VII) · **two currencies** (a scarce store of
value + HLQ for daily use) · **justice** (juries by reputation, separated powers, Art. IX) ·
**market** (trade sustained by reputation) · **mutual aid** · **private chat / forum**.

### Client — the gathering place (future)
A **PWA** that is a **light client to the network**, not a front-end to a central server — so there is
no single point the State can seize or censor. The market and the forum live on the chain / P2P; the
app is only the window. Packageable as a native shell, never dependent on an app store.

Crucially, the client bundle is **served by the node network itself**: every Harlequin node hosts a
copy of the (static) client, **content-addressed and signed** so it can be fetched from any node and
verified by hash — a hostile node cannot serve a poisoned client. To silence Harlequin you would have
to shut down *every* node at once. Early on there are few nodes (perhaps one); the design treats that
first server as "the first node that also serves the client," not a special host, so hosting
decentralises on its own as the network grows.

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
