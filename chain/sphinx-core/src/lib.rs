//! # sphinx-core — Harlequin onion transport (structural prototype)
//!
//! Metadata-resistant, **ephemeral** calls over the Harlequin node network (chat/forum #648 §E). A
//! caller reaches a callee through `k` relays chosen by reputation; each relay peels exactly one layer
//! of a **fixed-size** Sphinx packet and forwards the rest, learning nothing about position, length, or
//! the parties. No relay stores anything (zero-persistence, [`relay`]).
//!
//! ## Status — executable specification, NOT the shipped implementation (Path 2)
//! The exposed transport uses Nym's audited `sphinx-packet` for *both* construction and crypto. This
//! crate is the **conformance contract + test vectors** that pin the protocol shape; the hand-rolled
//! construction here is a **reference**, not exposure-grade (unaudited filler/MAC-chain/blinding; the
//! toy `OnionCrypto` is insecure). The dangerous part of Sphinx is the construction, not the primitives
//! — so it is settled by an audited library, per Harlequin's "no homegrown crypto" rule. See
//! `CONFORMANCE.md` for the invariants the integration must satisfy.
//!
//! ## What this crate is — and is NOT
//! It is the **dependency-free structural reference** that lives on the isolated station inside the
//! `chain/` workspace (OPSEC: no external crates pulled here). It nails down the *structure*: packet
//! format, fixed-size layering + filler, integrity, reputation-weighted routing, and the call-setup
//! envelope. Two crypto seams are left as traits:
//!
//! - [`crypto::OnionCrypto`] — the per-hop relay primitives. The bundled impl is an **INSECURE TOY**
//!   ([`crypto::InsecureToyOnionCrypto`], feature `insecure-toy-crypto`, non-default) that only exists
//!   to test the routing end-to-end. At integration this seam is filled by an *audited* Sphinx library
//!   (Nym `sphinx-packet`), in the node's transport crate — **not** in `chain/`.
//! - [`session::SessionCrypto`] — the end-to-end session handshake (Noise + forward secrecy). This is
//!   **GATED behind #637** (cabled in a dedicated session); the stub refuses to derive keys
//!   ([`session::SessionError::Gated637`]). There is no toy fallback for the session.
//!
//! ## Module map
//! - [`packet`] — fixed-size types, the canonical `hlq-` handle, base32.
//! - [`sphinx`] — `create_packet` / `process_packet` (the layering core).
//! - [`relay`] — the stateless relay (zero-persistence asserted in code).
//! - [`route`] — reputation-weighted path selection.
//! - [`call`] — CALL_REQ / CALL_RING / CALL_ACCEPT transport handshake.
//! - [`crypto`] / [`session`] — the two trait seams above.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod call;
pub mod crypto;
pub mod packet;
pub mod relay;
pub mod route;
pub mod session;
pub mod sphinx;
pub mod surb;

pub use packet::{
    handle_bytes, handle_string, Addr, ProcessResult, SphinxError, SphinxHeader, SphinxPacket,
    MAX_HOPS,
};
pub use relay::Relay;
pub use route::{select_path, Candidate};
pub use sphinx::{build_header, create_packet, process_packet};
pub use surb::{apply_surb, create_surb, open_surb_reply, Surb, SurbReplyKeys};
