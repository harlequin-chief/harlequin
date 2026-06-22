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
    }
}
