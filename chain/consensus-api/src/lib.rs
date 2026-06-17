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
use alloc::vec::Vec;

sp_api::decl_runtime_apis! {
    /// Consensus-facing view of the chain. v1 exposes the per-account consensus reputation (the
    /// conservative min across the four suits, raw fixed-point i128) — exactly what `elect_committee_fp`
    /// needs to weight the sortition.
    pub trait HarlequinConsensusApi {
        /// Each account with positive consensus reputation → its reputation (fixed-point i128).
        /// Account is its raw 32-byte id.
        fn consensus_reputation() -> Vec<([u8; 32], i128)>;
    }
}
