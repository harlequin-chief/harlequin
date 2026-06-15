# Reputation pallet — design

How the validated `reputation-core` engine becomes an on-chain Substrate pallet. Design first (no
compile), so that once the node toolchain is in place the implementation is mechanical. Anchors:
`SPEC §1` (reputation), `§1.4` (two gates / genesis), `§2.2` (sortition), `§4` (justice/slashing).

## Core principle: inputs on-chain, reputation DERIVED
The chain stores the **verifiable inputs** — objective evidence per suit and the vouch graph — never a
reputation number someone asserts. Reputation is **recomputed from those inputs** by anyone, with
`reputation-core`. There is no trusted oracle of who is reputable: given the on-chain inputs, the
result is deterministic and independently checkable. *This is the anti-State stance in the data model:
no authority decrees standing; it is computed from provable acts.*

## Storage
- `Evidence: map (AccountId, Suit) -> Balance` — validated objective evidence per suit (§1.3a). The
  four suits are fixed (♦ commerce, ♣ technical, ♠ judicial, ♥ governance, LORE.md).
- `Vouches: map AccountId -> Vec<(AccountId, Suit, weight)>` — the attestation graph edges (§1.3b).
- `VouchRegistry: map AccountId -> Vec<Sponsorship>` — persistent sponsor↔protégé liability (§1.5c).
- `ReputationSnapshot: map (AccountId, Suit) -> Score` — the LAST epoch's computed reputation vector
  (the value consensus/justice read; see "epoch flow").
- `GenesisCohort: set AccountId` + `GenesisWeight` — the seed (§1.4), designed to dilute as the network
  grows (the weight schedule decays over epochs).
- `Epoch`, `Beacon` — current epoch and its public randomness (drives sortition rotation, §2.2).

## Extrinsics (calls)
- `submit_evidence(suit, proof)` — record validated work in a suit. Evidence verification is
  **pluggable** (a trait `EvidenceProof`), like personhood is pluggable in §1.5: a settled-deal proof,
  a completed-job attestation, an oracle, etc. The pallet does not hard-code what counts as work.
- `vouch(target, suit, weight)` / `revoke_vouch(target, suit)` — put reputation at stake on someone
  (§1.3b). `vouch` checks the caller's live-vouch quota (`vouch_quota`, sublinear, §1.5c).
- `graduate(protege)` — release a live vouch (frees quota; liability persists, §1.5c).
- `report_fraud(culprit, evidence)` — routed to the **justice pallet** (§4); on a guilty verdict it
  triggers `cascade_slashing` up the vouch chain (½ per hop, §1.7).

## Epoch flow (the heavy-compute problem, solved)
EigenTrust is `O(iterations × edges)` — too heavy to run on-chain every block. Design:
1. During an epoch, evidence/vouch extrinsics mutate the **inputs** on-chain (cheap, bounded writes).
2. At the epoch boundary, an **offchain worker** runs `reputation-core` over the on-chain inputs and
   submits the new `ReputationSnapshot` as a signed transaction. Because the inputs are on-chain and
   the computation is deterministic, **every node re-runs it and rejects a wrong snapshot** — no trust
   in the worker (verify-by-recompute, like a fraud-proof). Cost is amortised once per epoch.
3. Consensus committee election (`consensus-core::elect_committee`) and justice juries draw on the
   **previous** snapshot (reputation as of a finalised past epoch — it changes slowly, so using a
   one-epoch-old value is safe and avoids a circular dependency with the block being produced).

Rationale: this keeps the chain light (Art. VI / "runs on a phone") while the anti-collusion guarantees
of the engine hold, because they are properties of the inputs, recomputed in the open.

## How `reputation-core` plugs in
The pallet is a **thin wrapper**: it owns storage + extrinsics + the epoch hook; all reputation math is
`reputation-core` (already at parity with the validated prototype, 10/10 cross-validated). Required
refactor before it can be a pallet dependency:
- **`no_std`**: replace `std::collections::HashMap` with `alloc::collections::BTreeMap` (also makes
  iteration order deterministic — important for consensus reproducibility), gate `std` behind a feature.
- **Fixed-point or bounded f64**: floating point is non-deterministic across architectures. For
  consensus-critical values, port the EigenTrust iteration to **fixed-point** (or a deterministic
  rational), and cross-validate the fixed-point result against the f64 prototype within a tolerance.
  *(This is the one numerically delicate task; flagged for its own milestone.)*
- Sortition (`consensus-core`) is already integer-seat + a hashed VRF; the only float is the Poisson
  CDF, which must also go deterministic/fixed-point for on-chain use.

## Privacy / OPSEC
Accounts are **pseudonyms** (Art. VII); the chain never stores legal identity. The public reputation
profile (§4b) is per-pseudonym. Evidence proofs must not leak identity — the `EvidenceProof` trait is
where zero-knowledge / minimal-disclosure proofs plug in later (§1.8 open).

## Milestones (once toolchain is in)
1. `reputation-core` → `no_std` + deterministic arithmetic (fixed-point), cross-validated.
2. Minimal Substrate solochain (node template) + this pallet, genesis cohort seeded (§1.4).
3. Wire `consensus-core` sortition as the committee/validator selection.
4. Sub-sampled finality + light clients (§2.3).
