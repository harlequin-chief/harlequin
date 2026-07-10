//! Runtime API: the typed bridge from on-chain reputation to anything outside the runtime.
//!
//! Reputation lives in the runtime (`pallet-reputation`: the per-epoch `ReputationSnapshot`, one raw
//! fixed-point value per (account, suit) — SPEC §1.2b). The node, the panel's mediating backend
//! (`/api/standing`, the mediated read contract), and the operator console all live *outside* the runtime.
//! This API is how they ask the runtime, at a given block, "where does this mask stand?" — a typed
//! read off real chain state, not a guess and not a hand-rolled storage decode.
//!
//! The account is its raw 32-byte id, and suit values come back as a fixed `[i128; 4]` in the
//! canonical suit order (`Suit::ALL`): commerce ♦, technical_contribution ♣, judicial_function ♠,
//! governance ♥. Values are RAW fixed-point (`reputation_core::FP_SCALE`-scaled) — presentation
//! scaling is the caller's business. An account absent from the snapshot reads as all-zeros
//! ("you start at zero" is a state, not an error). Both calls keep the API free of runtime-only
//! types so the node can link it without pulling the whole FRAME tree (same discipline as
//! `harlequin-tokens-api` / `harlequin-consensus-api`).

#![cfg_attr(not(feature = "std"), no_std)]

/// Index of commerce (♦) in the `[i128; 4]` returned by [`ReputationApi::suits_of`].
pub const SUIT_COMMERCE: usize = 0;
/// Index of technical contribution (♣).
pub const SUIT_TECHNICAL: usize = 1;
/// Index of judicial function (♠).
pub const SUIT_JUDICIAL: usize = 2;
/// Index of governance (♥).
pub const SUIT_GOVERNANCE: usize = 3;

sp_api::decl_runtime_apis! {
    /// Reputation-facing view of the chain: the four-suit vector and the consensus aggregate
    /// (SPEC §1.2b / §1.2c) per account.
    pub trait ReputationApi {
        /// Last epoch's reputation snapshot of `account` (raw 32-byte id) per suit, raw fixed-point
        /// i128, canonical order [♦ commerce, ♣ technical, ♠ judicial, ♥ governance]. An account
        /// never seen reads as `[0, 0, 0, 0]`.
        fn suits_of(account: [u8; 32]) -> [i128; 4];

        /// The consensus standing of `account`: the §1.2c concave conservative aggregate
        /// `(1−λ)·min + λ·median` over the four suits — the same single definition the sortition
        /// and the attester floor read. Raw fixed-point i128; ≤ 0 means no structural weight.
        fn standing_of(account: [u8; 32]) -> i128;
    }
}
