//! Harlequin solochain runtime — the program a node executes.
//!
//! WIP: composes `frame_system` + the reputation pallet via `#[frame::runtime]`. Still to add for a
//! node-runnable runtime: `impl_runtime_apis!` (Core/BlockBuilder/…), consensus (Aura/Grandpa or manual
//! seal), and `substrate-wasm-builder` for the wasm blob. Reference: polkadot-sdk solochain template.
//!
//! Sovereign chain, Woven-Trust consensus: the committee is elected by reputation
//! (`consensus-core::elect_committee_fp`), not stake. See `../PALLET-DESIGN.md`.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
use alloc::vec::Vec;

use frame::prelude::*;
use frame::runtime::prelude::*;

/// The block type the runtime operates on. `BlockOf` (frame's common types) builds a real block —
/// header + opaque extrinsic — usable in `no_std`/wasm, unlike the std-only `MockBlock`.
pub type Block = frame::runtime::types_common::BlockOf<Runtime>;

#[frame_construct_runtime]
mod runtime {
    #[runtime::runtime]
    #[runtime::derive(
        RuntimeCall,
        RuntimeEvent,
        RuntimeError,
        RuntimeOrigin,
        RuntimeTask,
        RuntimeHoldReason,
        RuntimeFreezeReason,
    )]
    pub struct Runtime;

    #[runtime::pallet_index(0)]
    pub type System = frame_system::Pallet<Runtime>;

    #[runtime::pallet_index(1)]
    pub type Reputation = pallet_reputation::Pallet<Runtime>;
}

#[derive_impl(frame_system::config_preludes::SolochainDefaultConfig)]
impl frame_system::Config for Runtime {
    type Block = Block;
}

impl pallet_reputation::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    /// Evidence authority: root for now (a testnet placeholder for the pluggable proof system).
    type EvidenceOrigin = EnsureRoot<Self::AccountId>;
    type JusticeOrigin = EnsureRoot<Self::AccountId>;
    type MaxVouches = ConstU32<128>;
}
