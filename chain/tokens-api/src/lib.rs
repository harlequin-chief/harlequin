//! Runtime API: the typed bridge from on-chain token balances to anything outside the runtime.
//!
//! Balances live in the runtime (`pallet-tokens`, two coins HLQ + SOV — SPEC §3). The node, the
//! wallet-UI behind it, and the operator console all live *outside* the runtime. This API is how they
//! ask the runtime, at a given block, "what does this account hold?" — a typed read off real chain
//! state, not a guess and not a hand-rolled storage decode.
//!
//! The account is its raw 32-byte id and the coin is a `u8` selector (`0` = HLQ, `1` = SOV); both keep
//! the API free of runtime-only types so the node can link it without pulling the whole FRAME tree
//! (same discipline as `harlequin-consensus-api`). An unknown coin selector reads as `0`.

#![cfg_attr(not(feature = "std"), no_std)]

/// Coin selector for [`TokensApi::balance_of`]: HLQ (daily-use, "La Bolsa").
pub const COIN_HLQ: u8 = 0;
/// Coin selector for [`TokensApi::balance_of`]: SOV (Soberano reserve, "El Cofre").
pub const COIN_SOV: u8 = 1;

sp_api::decl_runtime_apis! {
    /// Token-facing view of the chain: per-account balances of the two coins (SPEC §3).
    pub trait TokensApi {
        /// Balance of `account` (raw 32-byte id) in `coin` (`0` = HLQ, `1` = SOV; any other value ⇒ `0`).
        /// An account that holds nothing — or has never been seen — reads as `0` (balances are
        /// `ValueQuery`, absent ⇒ zero).
        fn balance_of(account: [u8; 32], coin: u8) -> u128;
    }
}
