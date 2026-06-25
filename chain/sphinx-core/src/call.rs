//! Call setup — the transport-level handshake for an ephemeral call (CHAT-FORUM-SPEC §E).
//!
//! These messages ride *inside* Sphinx payloads through a circuit; the relays only forward layers. The
//! flow (the transport contract):
//!
//! ```text
//! caller  --CALL_REQ{circuit_id, rendezvous_tag, ephemeral_pub}-->  (k onion hops)  -->  callee
//! callee  --CALL_RING-------------------------------------------->  (rings)
//! callee  --CALL_ACCEPT{ephemeral_pub}-------------------------->   caller
//! then: ephemeral Noise channel  (GATED #637 — see crate::session)
//! ```
//!
//! Everything here is the *transport* envelope: circuit id, a rendezvous tag, and the two ephemeral
//! public keys exchanged. The bytes that turn those ephemerals into a forward-secret channel are the
//! session handshake — NOT here; that is `crate::session` and it is gated (#637). Building a call to the
//! point of a live secure channel therefore returns [`crate::session::SessionError::Gated637`] today.
//!
//! Zero persistence: these are messages in flight. No relay or this module stores them.

use crate::crypto::Key;
use crate::session::{Session, SessionCrypto, SessionError};

/// Opaque per-call circuit identifier (random; not linkable to identity).
pub type CircuitId = u64;

/// Mask→session binding (S2, #637 conformance). Binds the on-chain **mask** (sr25519) to the **Noise
/// static X25519** it commits to for this call — so the session authenticates the MASK, not just a bare
/// X25519 key. **All three fields are OPAQUE bytes here** (sphinx-core is dependency-free); the session
/// layer (`hlq-transport`) does the real sr25519 verification, whose S2 close condition is the TRIPLE:
///   1. `verify_sr25519(mask_pub, b"hlq-session-prekey-v1" || static_x25519, prekey_sig) == ok`,
///   2. `static_x25519 == the Noise-authenticated remote static` (else a MITM replays the victim's
///      signed-prekey bound to a different static), and
///   3. `handle(mask_pub) == expected peer` (`sha256(mask_pub)[..16]`).
/// Freshness is per-call (one signature per setup; no prekey store — pure S6).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MaskBinding {
    /// sr25519 mask public key (on-chain identity; handle = `sha256(mask_pub)[..16]`).
    pub mask_pub: [u8; 32],
    /// The Noise static X25519 this mask commits to for the call.
    pub static_x25519: [u8; 32],
    /// sr25519 signature over `b"hlq-session-prekey-v1" || static_x25519`. Verified by the session layer.
    pub prekey_sig: [u8; 64],
}

/// Caller → callee: request a call. Carries the caller's ephemeral public key and its mask binding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CallReq {
    pub circuit_id: CircuitId,
    /// Rendezvous tag (so the callee can correlate the reply circuit without revealing identities).
    pub rendezvous_tag: [u8; 32],
    pub ephemeral_pub: Key,
    /// Caller's mask→static binding (S2). Opaque to sphinx-core; verified by the session layer.
    pub binding: MaskBinding,
}

/// Callee → caller: the callee's device is ringing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CallRing {
    pub circuit_id: CircuitId,
}

/// Callee → caller: accepted. Carries the callee's ephemeral public key and its mask binding.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CallAccept {
    pub circuit_id: CircuitId,
    pub ephemeral_pub: Key,
    /// Callee's mask→static binding (S2). Opaque to sphinx-core; verified by the session layer.
    pub binding: MaskBinding,
}

/// Where a call is in the transport handshake. The crypto channel is a separate, gated step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CallPhase {
    Requested,
    Ringing,
    Accepted,
    /// A live forward-secret session — only reachable once #637 is cabled.
    Connected,
}

/// Drives the transport handshake. Holds only the ephemeral material needed to *attempt* the session;
/// the session itself is built by a `SessionCrypto` (gated).
pub struct CallSetup {
    pub circuit_id: CircuitId,
    pub phase: CallPhase,
    pub local_ephemeral_secret: Key,
    pub local_ephemeral_pub: Key,
    pub peer_ephemeral_pub: Option<Key>,
    /// The peer's mask binding from CALL_ACCEPT (S2). The session layer verifies it against the Noise
    /// remote static when opening the channel.
    pub peer_binding: Option<MaskBinding>,
}

impl CallSetup {
    /// Initiator side: produce the CALL_REQ to send through the circuit. `binding` is the caller's own
    /// mask→static binding (S2) to carry in the request.
    pub fn initiate(
        circuit_id: CircuitId,
        rendezvous_tag: [u8; 32],
        ephemeral_secret: Key,
        ephemeral_pub: Key,
        binding: MaskBinding,
    ) -> (Self, CallReq) {
        let setup = CallSetup {
            circuit_id,
            phase: CallPhase::Requested,
            local_ephemeral_secret: ephemeral_secret,
            local_ephemeral_pub: ephemeral_pub,
            peer_ephemeral_pub: None,
            peer_binding: None,
        };
        let req = CallReq { circuit_id, rendezvous_tag, ephemeral_pub, binding };
        (setup, req)
    }

    /// Initiator side: record the callee's CALL_ACCEPT (ephemeral + mask binding).
    pub fn on_accept(&mut self, accept: &CallAccept) {
        debug_assert_eq!(accept.circuit_id, self.circuit_id);
        self.peer_ephemeral_pub = Some(accept.ephemeral_pub);
        self.peer_binding = Some(accept.binding);
        self.phase = CallPhase::Accepted;
    }

    /// Attempt to open the forward-secret session. GATED: returns `Gated637` until #637 is cabled.
    ///
    /// TODO(#637): on success this transitions `phase` to `Connected` and returns the live `Session`.
    pub fn connect<S: SessionCrypto>(&mut self, session_crypto: &S) -> Result<Session, SessionError> {
        let peer = self.peer_ephemeral_pub.ok_or(SessionError::Failed)?;
        let session = session_crypto.open_session(&self.local_ephemeral_secret, &peer)?;
        self.phase = CallPhase::Connected;
        Ok(session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::GatedSession637;

    fn dummy_binding(seed: u8) -> MaskBinding {
        MaskBinding { mask_pub: [seed; 32], static_x25519: [seed.wrapping_add(1); 32], prekey_sig: [seed; 64] }
    }

    #[test]
    fn handshake_progresses_until_gated_crypto() {
        let (mut setup, req) = CallSetup::initiate(42, [1u8; 32], [2u8; 32], [3u8; 32], dummy_binding(4));
        assert_eq!(setup.phase, CallPhase::Requested);
        assert_eq!(req.circuit_id, 42);
        assert_eq!(req.binding, dummy_binding(4), "CALL_REQ carries the caller's mask binding");

        // Callee accepts (transport level works today): ephemeral + its mask binding.
        setup.on_accept(&CallAccept { circuit_id: 42, ephemeral_pub: [9u8; 32], binding: dummy_binding(7) });
        assert_eq!(setup.phase, CallPhase::Accepted);
        assert_eq!(setup.peer_binding, Some(dummy_binding(7)), "peer mask binding recorded for the session layer");

        // Opening the secure channel is gated.
        assert_eq!(setup.connect(&GatedSession637).err(), Some(SessionError::Gated637));
        assert_eq!(setup.phase, CallPhase::Accepted, "phase does NOT advance to Connected while gated");
    }
}
