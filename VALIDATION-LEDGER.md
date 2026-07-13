# Harlequin — Validation Ledger (public edition)

> A reproducible record of the tests and validations behind the Harlequin chain, kept so the claims in
> the whitepaper can be cited and re-run. Each entry states **what is claimed**, **how it was tested**,
> **the result**, and **how to reproduce it** (command, artifact hash, where it ran).
>
> **Environments.** *Environment A* = the build/dev station (referred to as "local"). *Environment B* =
> an independently operated hardware devnet on separate machines and a separate network — the stronger
> evidence. Multi-node claims were validated in both.
>
> **Evidence inventory.** Reproduction commands below refer to this repository's layout
> (`reputation-engine/`, `consensus-simulator/`, `chain/`). A few raw hardware-run outputs and
> validation scripts still live in the operators' private evidence archive pending metadata scrubbing;
> where that is the case, the artifact is committed here by its **sha256**, so any later publication is
> verifiable against this ledger.
>
> Scope note: numeric dials marked PLACEHOLDER in the spec (e.g. the decay retention ρ, jury size) are
> tuned before mainnet; the validations below test the **mechanisms**, which do not depend on the final
> numbers.

---

## 1. Reputation engine — attack resistance (prototype)

- **Claim.** Reputation is anchored in objective evidence and resists Sybil, collusion, reputation
  laundering, and entrenchment; power tracks a *fraction of reputation*, not a count of identities.
- **Method.** Pure-stdlib Python prototype (`reputation-engine/`) running EigenTrust-with-anti-collusion
  -damping over crafted adversarial graphs: Sybil farms, dense and scattered collusion rings, *adaptive*
  ring fragmentation, asymmetric PageRank funnels, temporal decay, vouch-edge aging, protégé graduation.
- **Result.** Sybil ≈ 0% power; collusion ring ≈ 0%; adaptive fragmentation evades the community label but
  is strangled by the conservative aggregate; asymmetric funnel cut ~70–80% while honest-popular accounts
  keep ≥85%; laundering loses everything; cascade slashing propagates. **17/17 tests pass.**
- **Reproduce.** `cd reputation-engine && python3 tests/test_engine.py` (suite) and `python3 run_all.py`
  (regenerates `RESULTS.md`).
- **Status.** Validated, environment A. Detail: `reputation-engine/RESULTS.md`.

## 2. Consensus simulator — safety/liveness under attack

- **Claim.** The security threshold is a **fraction of reputation, not of nodes**; the protocol keeps
  *safety* under partition and message loss, degrading only *liveness*.
- **Method.** Discrete-event simulator (`consensus-simulator/`): reputation-weighted sub-sampled voting;
  independence cap per correlated cluster; network partition; message loss; VRF sortition committees with
  per-epoch rotation over an asynchronous network.
- **Result.** A Sybil holding 93% of *nodes* does not break the network; capture needs ⌈α/cap⌉ independent
  clusters; partition → the isolated group **stalls instead of forking** (fork ~100%→~0%); loss tightens
  the liveness bound `α≤k(1−p)²` but capture stays 0%; committees rotate; sybils get 0 seats. **11/11
  testrig + 13/13 property/analysis tests.** Parameters and rationale: `consensus-simulator/PARAMETERS.md`,
  `consensus-simulator/ANALYSIS-safety-liveness.md`.
- **Reproduce.** `cd consensus-simulator && python3 run_consensus.py`; `python3 -m pytest` in its testrig.
- **Status.** Validated, environment A.

## 3. Rust cores — cross-validation against the prototype

- **Claim.** The Rust engine that runs inside the runtime computes **exactly** what the validated prototype
  does, deterministically (fixed-point), and builds `no_std` (wasm).
- **Method.** `reputation-core` and `consensus-core` carry oracle tests that compare the Rust output
  against the Python results on the same inputs; the fixed-point path is checked against the f64 path.
- **Result.** `reputation-core` 16/16 at exact parity + fully deterministic fixed-point; `consensus-core`
  **13/13 cross-validation** vs the test-rig (independence, partition + network-quorum, adaptive adversary,
  loss, safety/liveness sweeps); **35/35** total core tests; both compile `no_std`/wasm32.
- **Reproduce.** `cd chain && cargo test` (lean core workspace).
- **Status.** Validated, environment A.

## 4. Finality gadget — multi-node, two environments

- **Claim.** Blocks are finalized by **signed committee vote** (not a trivial flag), Byzantine-safe, with
  proofs verified on import and distributed so late joiners catch up; quorum loss halts (never forks).
- **Method.** `chain/consensus-core` (hash↔Snowball) + `chain/node` (gossip protocol): sr25519-signed
  votes + committee filter; proof packed + `verify_proof` on import; proof distribution so a keyless
  follower finalizes via imported proof; per-epoch committee rotation; range-finalization liveness. Run on
  multiple nodes in environment A and on environment B hardware.
- **Result.** `finality.rs` 6/6; `verify_proof` 6/6; committee rotation 8/8. Multi-node: 3/6/7 nodes
  finalize the same height/hash; followers finalize "via imported proof"; **validated in both
  environments**. Adversarial: quorum loss → halt (no fork); late-joiner catch-up holds.
- **Reproduce.** Build with `chain/node/build-node.sh`; run a devnet with `--consensus woven-trust-<ms>`
  and `woven-trust-voteonly-<ms>` modes across N nodes.
- **Status.** Validated, environments A **and B**.

## 5. Decay / anti-entrenchment — "the pioneers' advantage dilutes"

- **Claim.** Reputation is a **leaky integrator**: validated work adds at full weight, old work evaporates,
  so standing held without new contribution **decays toward the mean** — for everyone, founders included.
- **Method (sim).** The canonical engine with per-epoch evidence aging (`E←ρ·E`) and a growing population;
  worst case = a single founder with a strong head-start in all four domains who then **sleeps**. Sweep of
  decay rates (half-life 5/2/1 years) × growth {baseline, slow}.
- **Method (hardware).** Environment B devnet running the wired runtime (binary `afbe8e98…`): seed
  evidence to an account, advance epochs **without re-injecting**, observe storage by RPC; a control
  account re-injects each epoch.
- **Result (sim).** Founder share 1.0 → ~0 in **every** cell, including the worst case; committee capture
  closes by population growth regardless of the rate → the dilution is a *mechanical truth*, not prose.
- **Result (hardware).** Inactive account evidence 1000→900→810→…→429, **ratio 0.90/epoch exact**
  (= the configured retention, integer floor); the re-injecting account holds and rises to steady state.
- **Status.** Validated, sim + hardware (environment B).

## 6. Justice — jury by sortition with interest-exclusion

- **Claim.** Disputes are judged by a reputation-weighted sortition jury from which **interested parties
  are excluded** (a party, or anyone trust-tied to one), enforced in depth; a supermajority verdict only
  certifies the fact and triggers the reputational consequence — no force.
- **Method.** `chain/pallet-justice` with triple-guarded exclusion (at the draw, at the vote, at the tally)
  and a substantive interest test (a party, or a direct vouch edge either direction). Unit tests +
  on-chain validation on an environment B devnet node (binary `ddbdd5c6…`).
- **Result.** Unit: `pallet-justice` 8/8, `pallet-reputation` 14/14. Hardware **9/9**: parties never
  drawn; a voucher of a party is **excluded**; a 2/3 guilty verdict resolves the case and slashes for real
  (1000→900); a party trying to vote is rejected; the depth-1/2 boundary behaves as designed; the justice
  origin is not root.
- **Reproduce.** `cd chain/pallet-justice && cargo test`; on-chain: open_case → inspect `Justice.Jury` →
  cast_vote (2/3) → close_case → inspect the defendant's evidence/reputation snapshot.
- **Status.** Validated, unit + hardware (environment B).

## 7. No-king reputation recount

- **Claim.** The reputation recount has **no privileged trigger**: it runs by the chain's own clock, every
  node recomputing the same deterministic result — there is no single button whose theft would control
  everyone's standing.
- **Method.** The root `advance_epoch` extrinsic was removed; the recompute (decay + EigenTrust + snapshot
  + epoch++) runs in `on_initialize` at every `EpochLength` boundary. Unit test + on-chain validation on an
  environment B devnet node (binary `df85528b…`).
- **Result.** Unit: `pallet-reputation` 15/15 (incl. `epoch_advances_automatically_with_no_root_trigger`).
  Hardware: `advance_epoch` **absent from the metadata**; epoch advanced ep1→ep7 with **nothing signed**;
  an inactive account's evidence decayed 0.90×/tick driven by the block clock, not a button.
- **Reproduce.** `cd chain/pallet-reputation && cargo test`; on-chain: run a node, seed evidence once,
  observe `Epoch` + `Evidence` move by themselves every `EpochLength` blocks.
- **Status.** Validated, unit + hardware (environment B).

## 8. Production network parameters

- **Claim.** The production consensus/cadence parameters are network-modelled, not guessed: they keep
  safety and liveness at the validated thresholds and each carries a reason.
- **Method.** Re-ran the validated consensus simulator (13/13 + 9/9, async lossy network,
  reputation-weighted VRF committees, per-epoch rotation) to fix `block_time`, the finality step, committee
  rotation cadence, and confirm `k/τ/β` under realistic latency/loss.
- **Result.** `block_time=6s` (real loss ~1% ≪ the stall wall p≈0.16); `k=20`, `α/k=0.70`, `β=12`, `τ=60`
  (capture 0% for f≤0.20); finality step 10 (time-to-finality ~1–2 min); two decoupled epochs — reputation
  recompute = **1 day** (14400 blocks, ρ=0.999052 for the 24-month half-life) and finality committee
  rotation = **1 hour** (600 blocks). Wired behind the `mainnet` cargo feature (testnet keeps fast values).
- **Status.** Modelled + sim-validated + **on-chain validated** (mainnet binary `be1350bd`): runtime
  metadata confirmed (ρ=999052, reputation epoch=14400, sponsor liability=25%, jury=12, guilt=2/3);
  no-king intact; finality operating. Network-model write-up committed by sha256
  `0f19378b61d81b04adefa3376d0f0bbf84ef989c67b6a1fc6dca35e7de684803`.

## 9. Constitutional layer — adversarial audit

- **Claim.** The founding document has no exploitable back-doors; the freeze is safe to apply.
- **Method.** Three-party adversarial audit of all 12 articles + non-contemplated cases (devil's advocate
  on every clause), stress-testing each finding before deciding.
- **Result.** 12/12 issues + non-contemplated cases resolved; a single change to the sealed text (the
  capacity/consent clause). Mutable calibrations moved to the technical spec.
- **Status.** Closed (text); freeze gates tracked separately.

## 10. Cold-start capture / expiration gate — adversarial sweep

- **Claim.** The genesis cold-start window resists capture by a competent external cabal, and the
  dispersion-expiration gate does not fire while a cabal is still dominant.
- **Method.** Adversarial parameter sweep (environment B), three tranches: (a) no-work Sybil injection;
  (b) competent cabal validating at t0 (ring, founder-vouched); (c) with rate × opportunity head-start.
  Measure max cabal consensus-share and the day cabal-share drops below 0.30 across cabal sizes K and
  honest onboarding rates.
- **Result.** No-work Sybil: max share 0.67%, 0 reach the "established" floor. Competent cabal captures
  the start (max share 68% K10 / 86% K30 / 95% K100); the dominant head-start is **mass present at
  genesis**, not validation opportunity. Dispersal below 0.30 depends on the onboarding-rate race; a cabal
  matching onboarding capacity never disperses by mechanism alone.
- **Conclusions** (cross-checked against an independent 4-model external review): cold-start is a **race**
  (honest arrival rate vs cabal mass); the automatic concentration-ceiling **halt is the only hard
  backstop**; the expiration gate must be a **conjunction** (max-cluster share < 0.30 AND ≥ M established
  independents); the lever that bounds K is external/ceremonial personhood at entry; the residual is
  declared, not hidden. Sweep write-ups + scripts committed by sha256:
  `5ebd3b2e687a785b622ef447f2a0e04cf44d16577026c8190e175a61b70cd4b4` (results),
  `af437d07b4d23579cb6d538f984f7d05865699447492e7e615236793b9620c18`,
  `8a610d9f074e8c9e091e533edb995d088ebf182b99163c751bda8d175027aac6` (scripts).
- **Status.** Closed (mechanism-bounded + residual declared).

## 11. Jury capture by 2-hop collusion

- **Claim.** Depth-1 interest-exclusion + community-brake keeps a 2-hop collusion ring off juries.
- **Method.** Monte-Carlo at scale (environment B), network ~150, jury 12, guilty ≥9/12, ≥4 acquit votes
  block a verdict, 8–10k trials per configuration. Two ring types: no-work Sybil ring at depth-2, and a
  **real-reputation** ring (genuine reputed members loyal to a party) at depth-2. Sweep ring size A.
- **Result.** No-work Sybil ring: sortition weight 0.0, 0 seats, P(block)=0% up to A=80. Real-reputation
  ring: depth-1 alone is NOT enough — P(block): A=10→2%, A=20→12%, A=40→44%, A=80→81%. Hard depth-2
  exclusion drives P(block)→0% but shrinks the jury pool ~35% and opens pool-poisoning.
- **Fix (implemented).** Depth-2 **sortition-weight penalty** (not a hard cut) + capped party-named
  interested lists + post-verdict accountability event. Implementation: bounded undirected BFS
  (`vouch_distance`, anti-DoS visit cap) in `chain/reputation-core` (16/16), `relation_depth` in
  `chain/pallet-reputation` (18/18), Permill weight penalty + `Depth2JurorsSeated` accountability event in
  `chain/pallet-justice` (10/10) — fixed-point, no float. **Hardware-validated on an environment B devnet,
  8/8**: depth-1 triple guard intact; a depth-2 juror is down-weighted exactly ×0.10 (event-carried value
  matches Permill math EXACT, two runs, different reputations); the BFS also catches the common-sponsor
  pattern; 0 spurious relations. Binary sha256
  `ffa39ee8454e9284b43b8208a2f141c65d025562ba79dee7c17ec67a28097760` (verified identical across every
  transfer hop before the run). Monte-Carlo write-up + script committed by sha256
  `9850db7570c0df12eafe2d3c2f3f40c37d7213600b4bfa700193ee32340dfc68`,
  `adf9d7975c53fca4eda5d96bd19d776f0fdf12c9fe4d2a3445f2741246eed54e`.
- **Status.** Sybil containment closed; reputed-ring residual quantified, penalty mechanism implemented and
  hardware-validated; the remaining residual (assembling many loyal *reputed* members) is bounded by social
  cost and declared.

## 12. Sortition anti-grinding — commit–reveal beacon

- **Claim.** A freely chosen sortition key + predictable seed is grindable; a per-epoch **commit–reveal**
  discipline removes the grind with no new dependencies: a node commits `c = SHA-256(s)` BEFORE the
  deferred beacon seed is fixed, then reveals `s` after; the sortition value is `SHA-256(s | seed)` over
  the committed `s`. The reputation anchor stays the real Sybil defence (zero reputation → zero seats).
- **Method.** Dep-free `no_std` module `chain/consensus-core/src/beacon.rs` (commit/verify_reveal,
  RANDAO-style deferred fold, `BeaconState` two-phase pipeline) + `chain/pallet-beacon` (on-chain state,
  no-king roll on the epoch hook) + the jury draw re-seated on the beacon + both node-side draws (finality
  committee and block production) rewired to committed keys + beacon seed, with a genesis fallback (before
  the pipeline fills, the draw degrades to the prior reputation-only path — no halt).
- **Result.** **70 unit tests** across the three layers (consensus-core 52 incl. 11 beacon, pallet-beacon
  8, pallet-justice 10). Hardware (environment B): jury plumbing **7/7** (phase separation, no poisoning,
  no-king roll — binary `517214d4…`); consensus committee on a 5-node devnet (binary `fafb7b6d…`):
  (a) fallback finalizes sustained with the beacon empty, (b) after 4 founders commit+reveal over 2 epochs
  the seed goes non-zero and the committee draws off the beacon, (c) the founder who did not reveal is
  absent from the committed keys → not in the committee. Config floor found (not a bug): the round period
  must give the finality gadget time to converge — 2 s stalls, **6 s finalizes sustained** (the mainnet
  value).
- **Reproduce.** `cd chain/consensus-core && cargo test beacon`; `beacon.rs` sha256
  `a9437401fa6ffc35075dd29fe24b348a7b00398af7eb19474a7a6471c27cc71b`.
- **Status.** Closed end-to-end and hardware-validated. Remaining: node-side auto commit/reveal each epoch
  (production convenience; the mechanism is proven by the by-hand path above).

## 13. Launch bootstrap — "founders from zero without halt"

- **Claim.** The launch genesis is "founders, no sudo, no balances, founders start near zero". A literal
  zero-reputation start would have no committee at block 0 → halt. The chosen fix is a **small DECLARED
  evidence seed** that decays: non-zero reputation at genesis so a committee forms, while the pioneer
  advantage still washes out.
- **Method.** `--chain launch` preset on a 5-node environment B devnet (6 s rounds), founder validators.
- **Result.** (a) Startup, no halt: the committee seats at block 0 and finalizes sustained (committee 3–4,
  α 3/3–3/4). (b) Guards sane: the founders sit at exactly 20% each, none > 1/3 → the entrenchment guard
  never trips. Design note: the reputation snapshot is a *normalized share*, so the seed's absolute decay
  keeps shares flat among equally active founders; dilution is delivered by network growth and by
  inactivity (validated in row 5) — working ≠ entrenching, and no eternal anchor exists.
- **Status.** Validated, hardware (environment B). The final genesis (real founder keys + sealed manifesto
  hash + external entropy anchor) is the launch ceremony, still ahead.

## 14–23. Wired-society increments (summary)

| # | Component | Tests | Env. A | Env. B (hardware) |
|---|---|---|---|---|
| 14 | Objective evidence-origin + auto-expiring bootstrap key | reputation 20/20 | ✅ | ✅ 6/6 + 8/8 |
| 15 | Incapacity adjudication (consent nullified; no slash/keys/guardian) | justice 11/11 | ✅ | ✅ 5 scenarios |
| 16 | Concave consensus aggregate (softens "MIN as a weapon", λ=0.5) | reputation 20/20 | ✅ | ✅ specialist→0; one-blow −47% vs MIN −100% |
| 17 | Cascade per-incident cap (anti-kamikaze, 50%) | reputation 21/21 | ✅ | ✅ sponsor capped at 50%, culprit→0 |
| 18 | Cascade probation window (anti vouch-bomb) | reputation 22/22 | ✅ | ✅ fresh vouch shielded; aged vouch cascades (capped) |
| 19 | Society integration — 7 pallets in one runtime | integration 5/5 | ✅ | ✅ binary `430d42da`; cross-pallet identity consistent 4 ways |
| 20 | Content addressing (CIDv1 + canonical body) | content-id 7/7 + forum-core 9/9 | ✅ | ✅ 2-band independent recompute; validated vs real `ipfs add` |
| 21 | Onion transport — Sphinx structural reference + handle derivation | sphinx-core 24/24 | ✅ | ✅ conformance 16/16 |
| 22 | Session crypto — mask↔Noise prekey binding | prekey 6/6 (suite 23/23) | — | ✅ fail-closed on tamper/impersonation/domain-sep |
| 23 | Typed token balance read API | pallet-tokens 14/14 + live RPC | ✅ | ✅ API==storage on real seeded balances; invalid selectors → 0, no panic |

## 24–33. Relaunch hardening + audit (summary)

| # | Claim | Tests | Env. A | Env. B iron (2-band) |
|---|---|---|---|---|
| 24 | Disinflationary emission is king-less: new coin enters only via the participation feed + curve, never a privileged origin | pallet-tokens sweep + service-feed | ✅ | ✅ era advances, SOV mints+distributes with no Root; HLQ split coin-agnostic |
| 25 | Author-seal is a soft-drop (block author check cannot halt production) | pallet test (both branches) | ✅ | — (in candidate metadata) |
| 26 | Era cadence pinned: `EraLength = 1_317_600` blocks (183 × 7200) | metadata assertion | ✅ | ✅ verified in binary metadata |
| 27 | Mainnet candidate verified multiple independent ways (byte-exact diffs vs source; iron 3/3; metadata: SS58, **no Sudo**, king-less token calls, era length, participation hook) | — | ✅ | ✅ (full reproducible-rebuild sha-compare still pending) |
| 28 | Anti-entrenchment guard is live-wired, not dead code | reputation unit | ✅ | ✅ counter/halt storage moves; rogue >1/3 halts at epoch 7; dispersal resets |
| 29 | Consensus/finality semantic re-audit — verdict SHIP | — | ✅ | ✅ deterministic election (no HashMap on the critical path), liveness floor closes the empty-committee wedge, quorum identical worker↔verify, votes re-verified on import, epoch race closed, 0 safety findings |
| 30 | Public repo build integrity | `cargo check --workspace` | ✅ exit 0 | ✅ independent re-check, exit 0 |
| 31 | Rep-farming magnitude cap holds | reputation unit 39/39 | ✅ | ✅ pre-fix repro 13.0× stake-cap → post-fix 1.97× (<2× empirical) |
| 32 | Dual-ledger fee fix: a token-funded fresh account can pay fees (sufficients materialized) | pallet-tokens 21/21 | ✅ | ✅ fresh account spends for real |
| 33 | Finality epoch-boundary race closed (stale committee pruned + refreshed) | node-side | ✅ | ✅ root cause captured under trace; KILL-1-of-6 PASS + REJOIN PASS |

**Current relaunch note (honest).** An internal audit found the finality-vote **gossip layer relayed
votes without verifying their sr25519 signature** before propagation (tallying still verified, so
finality safety was unaffected; the vector was amplification). The fix — verify before relay — is in the
source and in the current mainnet candidate binary, whose independent hardware validation (2-band iron)
is in progress; the candidate supersedes every earlier one. This ledger gains a row when that iron run
lands.

---

*Every claim in the whitepaper's evaluation section (§11 / Appendix A) resolves to a row of this ledger.
Where an artifact is not yet published, its sha256 above is the commitment; publication follows the
genesis materials.*
