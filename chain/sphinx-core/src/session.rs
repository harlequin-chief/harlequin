//! End-to-end session crypto — **GATED behind #637**. The handshake is **interactive** (Noise XX is a
//! 3-message exchange), so the seam is a small state machine, NOT a one-shot.
//!
//! THE CUT. Everything else in this crate (onion relaying, fixed-size packets, routing, call setup) is
//! buildable now. The session *handshake* between caller and callee — the ephemeral Noise exchange and the
//! forward-secret channel it derives — is gated (#637, cabled in a dedicated session, with a vetted
//! library). This module exposes the interface so the transport/UI can be written against it; the stub
//! [`GatedSession637`] implements it but **refuses to derive keys** — every method returns
//! [`SessionError::Gated637`]. There is deliberately NO toy fallback: a session must never run on fake
//! crypto. The real impl (Noise XX over an audited library) lives in the node transport crate and plugs
//! in behind this trait; sphinx-core stays dependency-free (handshake messages are opaque bytes here).

use crate::crypto::Key;
use alloc::vec::Vec;

/// Errors from the session layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionError {
    /// The real handshake/derivation is gated behind #637 (cabled in a dedicated session). Not yet built.
    Gated637,
    /// Handshake/transport failure in a real impl (bad tag, replay, identity mismatch, out of order).
    Failed,
    /// `seal`/`open` called before the handshake completed.
    NotEstablished,
}

/// An **interactive** session handshake (Noise XX = 3 messages). Drives the exchange and, once complete,
/// seals/opens transport frames. Implemented by the audited Noise impl in the transport crate; the bytes
/// (`write_message` out, `read_message` in) are opaque handshake messages carried piggyback in the
/// call-setup envelopes (`CALL_REQ` / `CALL_ACCEPT` / `CALL_FINISH`).
///
/// The mask↔Noise binding (S2) is verified by the implementation when it processes the peer's static:
/// the signed-prekey ([`crate::call::MaskBinding`]) must match the Noise-authenticated remote static.
pub trait SessionHandshake {
    /// Deliver the peer's signed prekey (its [`crate::call::MaskBinding`]) to the engine **before the
    /// read that completes S2**. The binding arrives in the call envelope (`CALL_REQ`/`CALL_ACCEPT`),
    /// out of band from the Noise transcript; `CallSetup` hands it over here so the engine can run the
    /// S2 TRIPLE when it learns the peer's Noise static (cond-1 sr25519 sig over `PREKEY_DOMAIN || rs`,
    /// cond-2 `static_x25519 == noise_remote_static`, cond-3 `handle(mask_pub) == expected`). Idempotent.
    /// Fails closed: `Gated637` while gated; `Failed` once the remote static is known and a check breaks.
    fn set_peer_binding(
        &mut self,
        mask_pub: [u8; 32],
        static_x25519: [u8; 32],
        prekey_sig: [u8; 64],
    ) -> Result<(), SessionError>;
    /// Produce the next outgoing handshake message (with our `payload`, e.g. nothing or app data).
    fn write_message(&mut self, payload: &[u8]) -> Result<Vec<u8>, SessionError>;
    /// Consume an incoming handshake message (advancing the state machine).
    fn read_message(&mut self, msg: &[u8]) -> Result<(), SessionError>;
    /// Whether the handshake has completed (transport keys derived).
    fn is_complete(&self) -> bool;
    /// Seal a transport frame. Errors with `NotEstablished` before the handshake completes.
    fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, SessionError>;
    /// Open a transport frame. Errors with `NotEstablished` before the handshake completes.
    fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, SessionError>;
}

/// Stub session handshake. Interface is real; key derivation is GATED (#637) → every call `Gated637`.
/// `is_complete` is always false, so a call driven by this stub can never reach a live session.
#[derive(Clone, Copy, Default)]
pub struct GatedSession637;

impl SessionHandshake for GatedSession637 {
    fn set_peer_binding(
        &mut self,
        _mask_pub: [u8; 32],
        _static_x25519: [u8; 32],
        _prekey_sig: [u8; 64],
    ) -> Result<(), SessionError> {
        Err(SessionError::Gated637)
    }
    fn write_message(&mut self, _payload: &[u8]) -> Result<Vec<u8>, SessionError> {
        Err(SessionError::Gated637)
    }
    fn read_message(&mut self, _msg: &[u8]) -> Result<(), SessionError> {
        Err(SessionError::Gated637)
    }
    fn is_complete(&self) -> bool {
        false
    }
    fn seal(&mut self, _plaintext: &[u8]) -> Result<Vec<u8>, SessionError> {
        Err(SessionError::Gated637)
    }
    fn open(&mut self, _ciphertext: &[u8]) -> Result<Vec<u8>, SessionError> {
        Err(SessionError::Gated637)
    }
}

/// Domain-separation label the mask's sr25519 key signs over its static X25519 (the signed prekey, S2).
/// Verified by the real session impl: `verify_sr25519(mask_pub, PREKEY_DOMAIN || static_x25519, sig)`.
pub const PREKEY_DOMAIN: &[u8] = b"hlq-session-prekey-v1";

/// A 32-byte X25519 static (the Noise static the mask commits to). Alias for clarity at the seam.
pub type StaticPub = Key;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gated_until_637() {
        let mut s = GatedSession637;
        assert_eq!(s.set_peer_binding([0; 32], [0; 32], [0; 64]).err(), Some(SessionError::Gated637));
        assert_eq!(s.write_message(b"").err(), Some(SessionError::Gated637));
        assert_eq!(s.read_message(b"x").err(), Some(SessionError::Gated637));
        assert!(!s.is_complete());
        assert_eq!(s.seal(b"x").err(), Some(SessionError::Gated637));
        assert_eq!(s.open(b"x").err(), Some(SessionError::Gated637));
    }
}
