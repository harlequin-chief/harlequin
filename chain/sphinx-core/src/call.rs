//! Call setup — the transport handshake for an ephemeral call (CHAT-FORUM-SPEC §E), now driving the
//! **interactive** session handshake (Noise XX, 3 messages) piggybacked on the call envelopes:
//!
//! ```text
//! caller  --CALL_REQ{circuit_id, rendezvous_tag, binding, noise_msg=msg1}-->  callee
//! callee  --CALL_ACCEPT{circuit_id, binding, noise_msg=msg2}-------------->   caller
//! caller  --CALL_FINISH{circuit_id, noise_msg=msg3}--------------------->     callee
//! then both hold a forward-secret session.
//! ```
//!
//! The `noise_msg` bytes are **opaque** to sphinx-core (dependency-free); the real Noise engine (snow, in
//! the node transport crate) produces/consumes them behind [`SessionHandshake`]. The **mask binding** (S2)
//! rides as an explicit field so the session impl can verify the signed-prekey static against the static
//! Noise authenticates. Until #637 is cabled, the handshake is [`GatedSession637`] and the call cannot
//! reach a live session (`Gated637`, fail-closed).

use crate::crypto::Key;
use crate::session::{SessionError, SessionHandshake};
use alloc::vec::Vec;

/// Opaque per-call circuit identifier (random; not linkable to identity).
pub type CircuitId = u64;

/// Mask→session binding (S2, #637). Binds the on-chain **mask** (sr25519) to the **Noise static X25519**
/// it commits to for this call. **Opaque bytes** here (sphinx-core is dependency-free); the session layer
/// does the real sr25519 verification, whose close condition is the TRIPLE:
///   1. `verify_sr25519(mask_pub, b"hlq-session-prekey-v1" || static_x25519, prekey_sig) == ok`,
///   2. `static_x25519 == the Noise-authenticated remote static` (else a MITM replays the victim's
///      signed-prekey bound to a different static), and
///   3. `handle(mask_pub) == expected peer` (`sha256(mask_pub)[..16]`).
/// Freshness is per-call (one signature per setup; no prekey store — pure ephemeral).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MaskBinding {
    /// sr25519 mask public key (on-chain identity; handle = `sha256(mask_pub)[..16]`).
    pub mask_pub: [u8; 32],
    /// The Noise static X25519 this mask commits to for the call.
    pub static_x25519: [u8; 32],
    /// sr25519 signature over `b"hlq-session-prekey-v1" || static_x25519`. Verified by the session layer.
    pub prekey_sig: [u8; 64],
}

/// Caller → callee: open a call. Carries the caller's mask binding and the first Noise message (msg1).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallReq {
    pub circuit_id: CircuitId,
    /// Rendezvous tag (so the callee can correlate the reply circuit without revealing identities).
    pub rendezvous_tag: [u8; 32],
    pub binding: MaskBinding,
    /// Opaque Noise handshake message 1.
    pub noise_msg: Vec<u8>,
}

/// Callee → caller: ringing (no payload).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct CallRing {
    pub circuit_id: CircuitId,
}

/// Callee → caller: accepted. Carries the callee's mask binding and Noise message 2.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallAccept {
    pub circuit_id: CircuitId,
    pub binding: MaskBinding,
    /// Opaque Noise handshake message 2.
    pub noise_msg: Vec<u8>,
}

/// Caller → callee: final handshake message (msg3) that completes the session.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallFinish {
    pub circuit_id: CircuitId,
    /// Opaque Noise handshake message 3.
    pub noise_msg: Vec<u8>,
}

/// Where a call is in the handshake.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CallPhase {
    /// Initiator sent CALL_REQ (msg1), awaiting CALL_ACCEPT.
    Requested,
    /// Responder sent CALL_ACCEPT (msg2), awaiting CALL_FINISH.
    Answering,
    /// A live forward-secret session — only reachable once #637 is cabled.
    Connected,
}

/// Drives the interactive call handshake over a [`SessionHandshake`] `H`. Holds the peer's mask binding
/// (for the session layer's S2 check). No long-term state, no persistence (ephemeral call).
pub struct CallSetup<H: SessionHandshake> {
    pub circuit_id: CircuitId,
    pub phase: CallPhase,
    pub handshake: H,
    /// The peer's mask binding (from the peer's CALL_REQ/ACCEPT). The session layer verifies it (S2).
    pub peer_binding: Option<MaskBinding>,
}

impl<H: SessionHandshake> CallSetup<H> {
    /// **Initiator.** Produce CALL_REQ (writes Noise msg1). `binding` is the caller's own signed prekey.
    pub fn initiate(
        circuit_id: CircuitId,
        rendezvous_tag: [u8; 32],
        binding: MaskBinding,
        mut handshake: H,
    ) -> Result<(Self, CallReq), SessionError> {
        let msg1 = handshake.write_message(&[])?;
        let setup = CallSetup { circuit_id, phase: CallPhase::Requested, handshake, peer_binding: None };
        let req = CallReq { circuit_id, rendezvous_tag, binding, noise_msg: msg1 };
        Ok((setup, req))
    }

    /// **Responder.** On receiving CALL_REQ: read msg1, produce CALL_ACCEPT (writes Noise msg2). Records
    /// the caller's binding. `binding` is the callee's own signed prekey.
    pub fn answer(
        binding: MaskBinding,
        mut handshake: H,
        req: &CallReq,
    ) -> Result<(Self, CallAccept), SessionError> {
        // Hand the caller's signed prekey to the engine up front; the responder's S2 fires when it learns
        // the caller's static at the msg3 read (`on_finish`). Idempotent + before that read.
        handshake.set_peer_binding(
            req.binding.mask_pub,
            req.binding.static_x25519,
            req.binding.prekey_sig,
        )?;
        handshake.read_message(&req.noise_msg)?;
        let msg2 = handshake.write_message(&[])?;
        let setup = CallSetup {
            circuit_id: req.circuit_id,
            phase: CallPhase::Answering,
            handshake,
            peer_binding: Some(req.binding),
        };
        let accept = CallAccept { circuit_id: req.circuit_id, binding, noise_msg: msg2 };
        Ok((setup, accept))
    }

    /// **Initiator.** On receiving CALL_ACCEPT: read msg2, produce CALL_FINISH (writes msg3). Records the
    /// callee's binding. After msg3 the initiator's handshake completes → `Connected`.
    pub fn on_accept(&mut self, accept: &CallAccept) -> Result<CallFinish, SessionError> {
        // The initiator learns the callee's static when it READS msg2 below → deliver the callee's signed
        // prekey FIRST so S2 (cond-2 static match + cond-1 sig + cond-3 handle) fires on that read.
        self.handshake.set_peer_binding(
            accept.binding.mask_pub,
            accept.binding.static_x25519,
            accept.binding.prekey_sig,
        )?;
        self.peer_binding = Some(accept.binding);
        self.handshake.read_message(&accept.noise_msg)?;
        let msg3 = self.handshake.write_message(&[])?;
        if self.handshake.is_complete() {
            self.phase = CallPhase::Connected;
        }
        Ok(CallFinish { circuit_id: self.circuit_id, noise_msg: msg3 })
    }

    /// **Responder.** On receiving CALL_FINISH: read msg3 → the handshake completes → `Connected`.
    pub fn on_finish(&mut self, finish: &CallFinish) -> Result<(), SessionError> {
        self.handshake.read_message(&finish.noise_msg)?;
        if self.handshake.is_complete() {
            self.phase = CallPhase::Connected;
        }
        Ok(())
    }

    /// Seal a transport frame once `Connected`.
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, SessionError> {
        self.handshake.seal(plaintext)
    }
    /// Open a transport frame once `Connected`.
    pub fn open(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, SessionError> {
        self.handshake.open(ciphertext)
    }
}

/// The label the mask signs over its static for S2 (re-exported convenience).
pub use crate::session::PREKEY_DOMAIN;
/// Convenience type for the X25519 static at the seam.
pub type StaticPub = Key;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::GatedSession637;

    fn binding(seed: u8) -> MaskBinding {
        MaskBinding { mask_pub: [seed; 32], static_x25519: [seed ^ 0xff; 32], prekey_sig: [seed; 64] }
    }

    /// A test double for a 3-message handshake (stands in for Noise XX). NOT crypto — it only exercises
    /// the call state machine: the real engine (snow) plugs in behind `SessionHandshake` in the transport.
    enum Role {
        Initiator,
        Responder,
    }
    struct MockHandshake {
        role: Role,
        step: u8,
        done: bool,
        /// Records the peer binding handed over by `CallSetup` (mask_pub, static, sig) — proves the seam
        /// delivers it to the engine (the real impl runs the S2 TRIPLE here; the mock only records).
        peer_binding: Option<([u8; 32], [u8; 32], [u8; 64])>,
    }
    impl MockHandshake {
        fn initiator() -> Self {
            MockHandshake { role: Role::Initiator, step: 0, done: false, peer_binding: None }
        }
        fn responder() -> Self {
            MockHandshake { role: Role::Responder, step: 0, done: false, peer_binding: None }
        }
    }
    impl SessionHandshake for MockHandshake {
        fn set_peer_binding(
            &mut self,
            mask_pub: [u8; 32],
            static_x25519: [u8; 32],
            prekey_sig: [u8; 64],
        ) -> Result<(), SessionError> {
            self.peer_binding = Some((mask_pub, static_x25519, prekey_sig));
            Ok(())
        }
        fn write_message(&mut self, _p: &[u8]) -> Result<Vec<u8>, SessionError> {
            match (&self.role, self.step) {
                (Role::Initiator, 0) => {
                    self.step = 1;
                    Ok(b"m1".to_vec())
                }
                (Role::Initiator, 2) => {
                    self.step = 3;
                    self.done = true; // initiator completes after sending msg3
                    Ok(b"m3".to_vec())
                }
                (Role::Responder, 1) => {
                    self.step = 2;
                    Ok(b"m2".to_vec())
                }
                _ => Err(SessionError::Failed),
            }
        }
        fn read_message(&mut self, msg: &[u8]) -> Result<(), SessionError> {
            match (&self.role, self.step) {
                (Role::Responder, 0) if msg == b"m1" => {
                    self.step = 1;
                    Ok(())
                }
                (Role::Responder, 2) if msg == b"m3" => {
                    self.step = 3;
                    self.done = true; // responder completes after reading msg3
                    Ok(())
                }
                (Role::Initiator, 1) if msg == b"m2" => {
                    self.step = 2;
                    Ok(())
                }
                _ => Err(SessionError::Failed),
            }
        }
        fn is_complete(&self) -> bool {
            self.done
        }
        fn seal(&mut self, p: &[u8]) -> Result<Vec<u8>, SessionError> {
            if !self.done {
                return Err(SessionError::NotEstablished);
            }
            Ok(p.to_vec()) // mock "encryption" = identity; real impl = AEAD
        }
        fn open(&mut self, c: &[u8]) -> Result<Vec<u8>, SessionError> {
            if !self.done {
                return Err(SessionError::NotEstablished);
            }
            Ok(c.to_vec())
        }
    }

    #[test]
    fn three_message_handshake_reaches_connected_both_sides() {
        // Initiator builds CALL_REQ (msg1).
        let (mut caller, req) =
            CallSetup::initiate(42, [7u8; 32], binding(1), MockHandshake::initiator()).unwrap();
        assert_eq!(caller.phase, CallPhase::Requested);
        assert_eq!(req.noise_msg, b"m1");

        // Responder answers (reads msg1, writes msg2).
        let (mut callee, accept) =
            CallSetup::answer(binding(2), MockHandshake::responder(), &req).unwrap();
        assert_eq!(callee.phase, CallPhase::Answering);
        assert_eq!(accept.noise_msg, b"m2");
        assert_eq!(callee.peer_binding, Some(binding(1))); // recorded caller's binding (S2 input)

        // Initiator on_accept (reads msg2, writes msg3) → completes.
        let finish = caller.on_accept(&accept).unwrap();
        assert_eq!(finish.noise_msg, b"m3");
        assert_eq!(caller.peer_binding, Some(binding(2)));
        assert_eq!(caller.phase, CallPhase::Connected);

        // Responder on_finish (reads msg3) → completes.
        callee.on_finish(&finish).unwrap();
        assert_eq!(callee.phase, CallPhase::Connected);

        // Both can now seal/open transport frames.
        let ct = caller.seal(b"hello").unwrap();
        assert_eq!(callee.open(&ct).unwrap(), b"hello");
    }

    #[test]
    fn peer_binding_delivered_to_handshake_both_sides() {
        // Drives the full 3-message exchange, then asserts each side's handshake actually RECEIVED the
        // peer's binding (the gap: CallSetup used to store it in `peer_binding` but never hand it to the
        // engine, so S2 could never run). Now the real engine has the binding before the completing read.
        let parts = |b: MaskBinding| (b.mask_pub, b.static_x25519, b.prekey_sig);

        let (mut caller, req) =
            CallSetup::initiate(9, [0u8; 32], binding(1), MockHandshake::initiator()).unwrap();
        let (mut callee, accept) =
            CallSetup::answer(binding(2), MockHandshake::responder(), &req).unwrap();
        // responder got the caller's binding at `answer`
        assert_eq!(callee.handshake.peer_binding, Some(parts(binding(1))));

        let finish = caller.on_accept(&accept).unwrap();
        // initiator got the callee's binding at `on_accept`, BEFORE reading msg2 (where S2 fires)
        assert_eq!(caller.handshake.peer_binding, Some(parts(binding(2))));

        callee.on_finish(&finish).unwrap();
        assert_eq!(caller.phase, CallPhase::Connected);
        assert_eq!(callee.phase, CallPhase::Connected);
    }

    #[test]
    fn gated_session_cannot_even_start() {
        // With the gated stub, the initiator cannot produce CALL_REQ (write_message fails closed).
        let r = CallSetup::initiate(1, [0u8; 32], binding(1), GatedSession637);
        assert_eq!(r.err(), Some(SessionError::Gated637));
    }

    #[test]
    fn seal_before_complete_is_rejected() {
        let (mut caller, _req) =
            CallSetup::initiate(1, [0u8; 32], binding(1), MockHandshake::initiator()).unwrap();
        // handshake not complete yet
        assert_eq!(caller.seal(b"x").err(), Some(SessionError::NotEstablished));
    }
}
