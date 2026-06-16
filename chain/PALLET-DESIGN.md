# Reputation pallet — design

How the validated `reputation-core` engine becomes an on-chain Substrate pallet. Anchors:
`SPEC §1` (reputation), `§1.4` (two gates / genesis), `§2.2` (sortition), `§4` (justice/slashing).

**Status (2026-06-16): the pallet is BUILT.** `pallet-reputation/` implements this design on
`polkadot-sdk-frame` (Chief authorised pulling it): storage (Evidence, Vouches, ReputationSnapshot,
Epoch, LastReport), calls (`submit_evidence`, `vouch`/`revoke_vouch` with the sublinear all-suits quota,
`advance_epoch` = recompute + telemetry, `report_fraud` = cascade slashing), and a genesis cohort. It
runs `reputation-core` on the deterministic fixed-point path, builds `std` + `no_std`/`wasm32`, and
passes 9/9 mock-runtime tests. The `runtime/` crate composes it into a Substrate runtime. What remains is
making the runtime node-runnable (`impl_runtime_apis!` + consensus + wasm-builder) and a booting node —
the in-runtime recompute here is a testnet trigger; production moves it to the offchain worker below.

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

## How `reputation-core` plugs in — prerequisites DONE
The pallet is a **thin wrapper**: it owns storage + extrinsics + the epoch hook; all the math is in the
crates, already deterministic and `no_std`. The refactors this section once flagged are complete:
- ✅ **`no_std`**: `reputation-core` and `consensus-core` build for `wasm32-unknown-unknown` with
  `default-features = false`. `BTreeMap` throughout (deterministic iteration). The f64 oracle paths sit
  behind a `std` feature; the runtime links only the integer paths.
- ✅ **Fixed-point**: the whole EigenTrust path (factors + iteration) runs in i128 fixed-point,
  bit-identical across architectures, cross-validated against f64. The pallet calls
  **`reputation_dimension_fully_fixed_fp`** (raw i128 reputation vector).
- ✅ **Sortition fixed-point**: `consensus-core::elect_committee_fp` + `sortition_fp` (the Poisson CDF's
  `e^{-lam}` done by integer range-reduction). **`vouch_quota_fp` / `mentor_dividend_fp` /
  `cascade_slashing_fp`** likewise (the `log2` done in integers). No floats, no libm anywhere on the
  runtime path.
- ✅ **Epoch flow implemented**: `protocol-core` is the host reference state machine — genesis cohort →
  per-epoch fixed-point recompute → `elect_committee_fp` → telemetry. The pallet mirrors this exactly
  (the difference is only WHERE state lives: pallet storage vs in-memory). It even elects the **same
  deterministic committee** the chain will (test: `committee_is_deterministic_across_runs`).

## Privacy / OPSEC
Accounts are **pseudonyms** (Art. VII); the chain never stores legal identity. The public reputation
profile (§4b) is per-pseudonym. Evidence proofs must not leak identity — the `EvidenceProof` trait is
where zero-knowledge / minimal-disclosure proofs plug in later (§1.8 open).

## Milestones
1. ✅ `reputation-core` + `consensus-core` → `no_std`/`wasm32` + deterministic fixed-point (engine,
   sortition, vouch scoring), all cross-validated against the f64 prototype.
2. ✅ Epoch state machine (`protocol-core`) — genesis → recompute → committee → telemetry, deterministic.
3. ⛔ **DECISION FOR CHIEF:** pull `polkadot-sdk` onto the station (OPSEC — external supply chain on the
   isolated admin disk). Until then the pallet code can be written but not compiled.
4. Minimal Substrate solochain (node template) + this pallet, genesis cohort seeded (§1.4), reusing the
   `*_fp` APIs above (wrap, don't reimplement).
5. Wire `elect_committee_fp` as committee/validator selection; offchain-worker recompute per the epoch
   flow above.
6. Sub-sampled finality + light clients (§2.3). The async voting behaviour is already validated in the
   Python test-rig (`prototipos/consenso/testrig/`, 11/11); the production transport is Substrate/libp2p,
   not a re-port of the simulator.
