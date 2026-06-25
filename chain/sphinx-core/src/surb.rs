//! SURB — Single-Use Reply Block: an anonymous return path for the call reply.
//!
//! The caller reaches the callee through a forward circuit, but the callee must be able to answer
//! (CALL_RING / CALL_ACCEPT) **without learning who the caller is or where they are**. A SURB solves
//! this: the caller pre-builds a Sphinx header that routes back to itself through relays *it* chose,
//! and hands that header to the callee inside the call. The callee drops its reply into the SURB and
//! sends it; the relays forward it like any other onion layer; only the caller — who kept the per-hop
//! keys — can peel the reply. The callee never sees the return path or the caller's address.
//!
//! "Single-use": each SURB encodes one ephemeral key chain, so reusing it is linkable — a fresh SURB
//! is built per reply slot.
//!
//! The construction reuses [`crate::sphinx::build_header`]: a SURB is just a header whose final hop is
//! the caller itself. The only difference from a forward packet is that the payload is filled by the
//! *replier*, not pre-wrapped by the sender — so the caller peels the relay layers afterwards with the
//! retained keys. Same crypto seam ([`crate::crypto::OnionCrypto`]); dependency-free.

use crate::crypto::{Key, OnionCrypto, LABEL_PAY};
use crate::packet::*;
use crate::sphinx::build_header;
use alloc::vec;
use alloc::vec::Vec;

/// A reply block the caller hands to the callee. Opaque routing — reveals nothing about the caller.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Surb {
    /// Address of the first relay the reply enters.
    pub first_hop: Addr,
    /// Pre-built header routing back to the caller.
    pub header: SphinxHeader,
}

/// The secrets the caller retains to peel a reply that comes back through a [`Surb`]. Never shared.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SurbReplyKeys {
    /// Shared secrets for the full return path, in forward order, INCLUDING the caller's own final hop.
    /// The replier pre-wraps nothing, so every hop (relays + the caller's own `process`) XORs one
    /// payload layer; the caller must peel all of them — its own included — to recover the plaintext.
    secrets: Vec<Key>,
}

/// Build a SURB whose return path runs through `relays` (in order) and terminates at `caller_pub`
/// (the caller's mask key, the FINAL hop). Returns the SURB to hand out and the keys to keep.
///
/// `relays.len() + 1 <= MAX_HOPS`. `eph_seed` must be fresh per SURB (single-use).
pub fn create_surb<C: OnionCrypto>(
    crypto: &C,
    relays: &[[u8; 32]],
    caller_pub: &[u8; 32],
    eph_seed: &Key,
) -> Result<(Surb, SurbReplyKeys), SphinxError> {
    if relays.is_empty() {
        return Err(SphinxError::Malformed);
    }
    // Path = relays..., then the caller as the final hop.
    let mut path: Vec<[u8; 32]> = relays.to_vec();
    path.push(*caller_pub);

    let (header, shareds) = build_header(crypto, &path, eph_seed)?;
    // Keep every shared secret on the return path (relays + the caller's own final hop) — see the
    // `SurbReplyKeys` doc for why the caller's own layer must also be peeled.
    Ok((
        Surb { first_hop: handle_bytes(&relays[0]), header },
        SurbReplyKeys { secrets: shareds },
    ))
}

/// Replier side: drop a reply into a SURB, producing the packet to send to `surb.first_hop`.
///
/// The replier supplies only the plaintext; it cannot read the routing and never learns the caller.
pub fn apply_surb(surb: &Surb, reply: &[u8]) -> SphinxPacket {
    let mut payload = vec![0u8; PAYLOAD_LEN];
    let take = core::cmp::min(reply.len(), PAYLOAD_LEN);
    payload[..take].copy_from_slice(&reply[..take]);
    SphinxPacket { header: surb.header.clone(), payload }
}

/// Caller side: recover the reply plaintext from the payload delivered through the SURB.
///
/// As the reply traversed the return path, each relay XORed one payload layer (during normal
/// forwarding) and the caller's own final-hop processing peeled the last. This removes the remaining
/// relay layers with the retained keys, yielding the replier's plaintext (still `PAYLOAD_LEN`, padded).
pub fn open_surb_reply<C: OnionCrypto>(
    crypto: &C,
    keys: &SurbReplyKeys,
    delivered_payload: &[u8],
) -> Vec<u8> {
    let mut pay = vec![0u8; PAYLOAD_LEN];
    let take = core::cmp::min(delivered_payload.len(), PAYLOAD_LEN);
    pay[..take].copy_from_slice(&delivered_payload[..take]);
    for shared in &keys.secrets {
        let pk = crypto.subkey(shared, LABEL_PAY);
        let mut s = vec![0u8; PAYLOAD_LEN];
        crypto.stream(&pk, &mut s);
        for j in 0..PAYLOAD_LEN {
            pay[j] ^= s[j];
        }
    }
    pay
}

#[cfg(all(test, feature = "insecure-toy-crypto"))]
mod tests {
    use super::*;
    use crate::crypto::InsecureToyOnionCrypto;
    use crate::sphinx::process_packet;

    fn keyset(c: &InsecureToyOnionCrypto, n: usize, base: u8) -> (Vec<Key>, Vec<[u8; 32]>) {
        let mut sks = Vec::new();
        let mut pks = Vec::new();
        for i in 0..n {
            let (sk, pk) = c.keypair_from_seed(&[base + i as u8; 32]);
            sks.push(sk);
            pks.push(pk);
        }
        (sks, pks)
    }

    #[test]
    fn reply_routes_back_and_decrypts() {
        let c = InsecureToyOnionCrypto;
        // Two return relays + the caller as final hop.
        let (relay_sks, relay_pks) = keyset(&c, 2, 30);
        let (caller_sk, caller_pk) = c.keypair_from_seed(&[99u8; 32]);

        let (surb, keys) = create_surb(&c, &relay_pks, &caller_pk, &[7u8; 32]).unwrap();
        assert_eq!(surb.first_hop, handle_bytes(&relay_pks[0]));

        // Callee drops a reply into the SURB (knows nothing about the caller).
        let reply = b"CALL_ACCEPT: ephemeral_pub=...";
        let mut pkt = apply_surb(&surb, reply);

        // Relays forward it like any onion packet.
        for i in 0..2 {
            match process_packet(&c, &relay_sks[i], &pkt).unwrap() {
                ProcessResult::Forward { next, packet } => {
                    if i == 0 {
                        assert_eq!(next, handle_bytes(&relay_pks[1]));
                    } else {
                        assert_eq!(next, handle_bytes(&caller_pk), "last relay routes to caller");
                    }
                    pkt = packet;
                }
                _ => panic!("relay {i} should forward"),
            }
        }

        // Caller is the final hop: peel its own layer, then the retained relay layers.
        match process_packet(&c, &caller_sk, &pkt).unwrap() {
            ProcessResult::Deliver { payload } => {
                let plain = open_surb_reply(&c, &keys, &payload);
                assert_eq!(&plain[..reply.len()], reply);
            }
            _ => panic!("caller should deliver"),
        }
    }

    #[test]
    fn callee_cannot_read_routing() {
        // The SURB header's routing is onion-encrypted: it is not the relay handles in the clear.
        let c = InsecureToyOnionCrypto;
        let (_sk, relay_pks) = keyset(&c, 2, 40);
        let (_csk, caller_pk) = c.keypair_from_seed(&[50u8; 32]);
        let (surb, _keys) = create_surb(&c, &relay_pks, &caller_pk, &[1u8; 32]).unwrap();
        // The caller's handle does not appear verbatim in the routing block.
        let needle = handle_bytes(&caller_pk);
        let hay = &surb.header.routing;
        let found = hay.windows(needle.len()).any(|w| w == needle);
        assert!(!found, "caller address must not be in the clear in the SURB");
    }

    #[test]
    fn too_long_return_path_rejected() {
        let c = InsecureToyOnionCrypto;
        let (_sk, relay_pks) = keyset(&c, MAX_HOPS, 60); // relays == MAX_HOPS, +caller overflows
        let (_csk, caller_pk) = c.keypair_from_seed(&[80u8; 32]);
        assert_eq!(create_surb(&c, &relay_pks, &caller_pk, &[1u8; 32]), Err(SphinxError::Malformed));
    }
}
