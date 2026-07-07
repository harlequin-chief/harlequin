//! Runtime API: the bridge from on-chain reputation to the node's Woven-Trust sortition.
//!
//! The node must run the committee sortition off the **real** reputation the chain has computed (not a
//! hardcoded stand-in). Reputation lives in the runtime (`pallet-reputation`, derived each epoch by
//! `reputation-core`); the node lives outside it. This runtime API is how the node asks the runtime, at
//! a given block, "what is everyone's consensus reputation right now?" — and then runs the sortition on
//! the answer. Returning raw account bytes + fixed-point reputation keeps the API free of runtime-only
//! types so the node can link it without pulling the whole runtime.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

sp_api::decl_runtime_apis! {
    /// Consensus-facing view of the chain. Exposes the per-account consensus reputation (the conservative
    /// min across the four suits, raw fixed-point i128) — what `elect_committee_fp` weights the sortition
    /// by — plus the **commit–reveal beacon** (§2.1): the un-grindable seed and the per-account committed
    /// sortition keys, so the node's committee draw stops keying on a freely-chosen `sk-{id}` against a
    /// predictable `epoch`/`slot` seed.
    pub trait HarlequinConsensusApi {
        /// Each account with positive consensus reputation → its reputation (fixed-point i128).
        /// Account is its raw 32-byte id.
        fn consensus_reputation() -> Vec<([u8; 32], i128)>;

        /// The current commit–reveal beacon seed (raw 32 bytes) — the un-grindable sortition seed (§2.1),
        /// unpredictable until after this epoch's keys were committed.
        fn beacon_seed() -> [u8; 32];

        /// Each account with an **active committed key** this epoch → its committed sortition key (hex).
        /// Account is its raw 32-byte id. Only accounts that committed AND revealed appear (a node that
        /// did not participate in the beacon this epoch is absent → not eligible for the committee draw).
        fn beacon_committed_keys() -> Vec<([u8; 32], String)>;

        /// Finality-vote delegation map: `(committee account, hot session-vote pubkey)`. The committee
        /// is keyed by the **cold** account (reputation); finality votes are signed by the **hot**
        /// session key. The node resolves a vote's signer (session pubkey) → its committee account
        /// through this map, IDENTICALLY on every node (read from on-chain state, never local config),
        /// so the cold sovereign key never has to sign. Absent entry → that account signs directly.
        fn vote_keys() -> Vec<([u8; 32], [u8; 32])>;

        /// §1.4b entrenchment guard: `true` when one entity has held > 1/3 of consensus reputation for the
        /// required run of epochs, so finality must FREEZE (the node seats an EMPTY committee) until the
        /// share dilutes back. Read at the epoch-pinned block alongside the committee draw, so every node
        /// freezes/resumes finality identically. `false` in the honest steady state.
        fn entrenchment_halted() -> bool;

        /// §1.4b RECOVERY ANCHOR (self-heal): for a finality-stuck `height`, the block `H_r` whose healthy,
        /// post-dilution reputation the node re-anchors that height's committee to — so finality crosses
        /// the frozen halt band validated by a SANE committee, instead of deadlocking on the band's
        /// `halted=true` pinned state. `None` if the height is in no recovered band (never halted there, or
        /// still frozen → committee stays empty). Range-aware (handles multiple halt cycles). Deterministic
        /// consensus state — the runtime resolves the band lookup so every node re-anchors identically.
        fn entrenchment_recovery_for(height: u32) -> Option<u32>;

        /// (F2 piece 6) Highest finalized height whose committee votes were already credited by the
        /// participation pallet (`LastCreditedHeight`). The node-side inherent provider reads it to
        /// know which canonical justifications to carry in the next block's record — heights AT or
        /// below it would be dropped on-chain anyway (exactly-once cursor).
        fn participation_cursor() -> u64;
    }
}
