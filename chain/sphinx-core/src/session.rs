//! End-to-end session crypto — **GATED behind #637**.
//!
//! THE CUT. Everything else in this crate (onion relaying, fixed-size packets, routing, call setup) is
//! buildable now. The one piece that is NOT is the *session handshake* between caller and callee: the
//! ephemeral-key Noise handshake and the forward-secret channel it establishes. Gated (#637): the real key derivation is cabled in a dedicated session, with a vetted library.
//!
//! This module exposes the clean interface (`open_session` / `seal` / `open`) so the transport and UI
//! can be written against it today. The stub implements the interface but **refuses to derive keys** —
//! every call returns [`SessionError::Gated637`]. There is deliberately NO toy fallback here: a session
//! must never silently run on fake crypto.

use crate::crypto::Key;
use alloc::vec::Vec;

/// Errors from the session layer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionError {
    /// The real handshake/derivation is gated behind #637 (cabled in a dedicated session). Not implemented yet.
    Gated637,
    /// Reserved for the real implementation (bad tag, replay, etc.).
    Failed,
}

/// An established forward-secret session. Opaque; the real type lands with #637.
pub struct Session {
    _private: (),
}

/// The session handshake + AEAD. `seal`/`open` carry call frames once a session is open.
///
/// TODO(#637): replace the stub with a Noise (XK/IK) handshake over an audited library, deriving
/// ephemeral keys with forward secrecy. The interface here is the contract that swap must satisfy.
pub trait SessionCrypto {
    /// Initiator side: begin a session toward `peer_static_pub` using our `ephemeral_secret`.
    fn open_session(
        &self,
        ephemeral_secret: &Key,
        peer_static_pub: &Key,
    ) -> Result<Session, SessionError>;

    /// Encrypt+authenticate a frame within an open session.
    fn seal(&self, session: &mut Session, plaintext: &[u8]) -> Result<Vec<u8>, SessionError>;

    /// Decrypt+verify a frame within an open session.
    fn open(&self, session: &mut Session, ciphertext: &[u8]) -> Result<Vec<u8>, SessionError>;
}

/// Stub session layer. Interface is real; key derivation is GATED (#637) → always `Gated637`.
#[derive(Clone, Copy, Default)]
pub struct GatedSession637;

impl SessionCrypto for GatedSession637 {
    fn open_session(&self, _e: &Key, _p: &Key) -> Result<Session, SessionError> {
        // TODO(#637): Noise handshake + forward secrecy, cabled in a dedicated session.
        Err(SessionError::Gated637)
    }
    fn seal(&self, _s: &mut Session, _p: &[u8]) -> Result<Vec<u8>, SessionError> {
        Err(SessionError::Gated637)
    }
    fn open(&self, _s: &mut Session, _c: &[u8]) -> Result<Vec<u8>, SessionError> {
        Err(SessionError::Gated637)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_is_gated_until_637() {
        let s = GatedSession637;
        assert_eq!(s.open_session(&[1u8; 32], &[2u8; 32]).err(), Some(SessionError::Gated637));
    }
}
