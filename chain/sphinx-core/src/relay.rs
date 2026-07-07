//! A relay: forwards one onion layer and stores NOTHING.
//!
//! ZERO-PERSISTENCE INVARIANT (the reviewer, hard guardrail). A relay must never retain ciphertext, payloads,
//! routing, or peer state — its only capability is "peel one layer, forward the rest". That is enforced
//! *structurally* here, not by comment:
//!   - [`Relay`] is a zero-sized type (no fields) — it has nowhere to store anything. A unit test
//!     asserts `size_of::<Relay> == 0`.
//!   - The only method, [`Relay::relay`], is a pure function over its inputs returning a
//!     [`ProcessResult`]; it owns no buffer and no handle to one.
//! There is deliberately no `store`, `cache`, `log`, or `history` API on this type.

use crate::crypto::{Key, OnionCrypto};
use crate::packet::{ProcessResult, SphinxError, SphinxPacket};
use crate::sphinx::process_packet;

/// A stateless onion relay. Zero-sized: structurally incapable of persisting anything.
#[derive(Clone, Copy, Default)]
pub struct Relay;

impl Relay {
    /// Peel one layer and decide forward-vs-deliver. Pure: no state crosses calls, nothing is stored.
    pub fn relay<C: OnionCrypto>(
        &self,
        crypto: &C,
        node_secret: &Key,
        packet: &SphinxPacket,
    ) -> Result<ProcessResult, SphinxError> {
        process_packet(crypto, node_secret, packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_holds_no_state() {
        // The zero-persistence invariant, asserted in code: a relay has nowhere to keep ciphertext.
        assert_eq!(core::mem::size_of::<Relay>(), 0);
    }
}
