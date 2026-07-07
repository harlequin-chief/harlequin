//! The Sphinx construction: build a layered packet for a path, and peel exactly one layer per hop.
//!
//! This is the *structural* core — fixed-size routing block with deterministic filler so every hop
//! sees a constant-size packet, per-hop MAC for integrity, ephemeral-key blinding for unlinkability.
//! All curve/cipher operations go through [`crate::crypto::OnionCrypto`] (see that module for the
//! prototype-vs-audited-library seam). The end-to-end *session* crypto (Noise) is a different,
//! GATED layer — see [`crate::session`] (#637); it is NOT used here.

use crate::crypto::{OnionCrypto, Key, LABEL_MU, LABEL_PAY, LABEL_RHO};
use crate::packet::*;
use alloc::vec;
use alloc::vec::Vec;

/// Build a fixed-size packet routed through `path` (relays in order) ending at the last entry, carrying
/// `payload` (zero-padded / truncated to `PAYLOAD_LEN`). `eph_seed` seeds the ephemeral key.
///
/// `path` holds the **public keys** of each hop, in forward order. `2 <= path.len <= MAX_HOPS`.
pub fn create_packet<C: OnionCrypto>(
    crypto: &C,
    path: &[[u8; 32]],
    payload: &[u8],
    eph_seed: &Key,
) -> Result<SphinxPacket, SphinxError> {
    let (header, shareds) = build_header(crypto, path, eph_seed)?;

    // Onion-wrap the payload (last hop inner-most).
    let mut pay = vec![0u8; PAYLOAD_LEN];
    let take = core::cmp::min(payload.len(), PAYLOAD_LEN);
    pay[..take].copy_from_slice(&payload[..take]);
    for i in (0..shareds.len()).rev() {
        let pk = crypto.subkey(&shareds[i], LABEL_PAY);
        let mut s = vec![0u8; PAYLOAD_LEN];
        crypto.stream(&pk, &mut s);
        for j in 0..PAYLOAD_LEN {
            pay[j] ^= s[j];
        }
    }

    Ok(SphinxPacket { header, payload: pay })
}

/// Build the Sphinx header (ephemeral + onion-encrypted routing + first-hop MAC) for `path`, returning
/// it alongside the per-hop shared secrets. Shared by [`create_packet`] and the SURB builder
/// ([`crate::surb`]) — the difference between a forward packet and a reply block is only what is done
/// with the payload, not how the header is laid out.
pub fn build_header<C: OnionCrypto>(
    crypto: &C,
    path: &[[u8; 32]],
    eph_seed: &Key,
) -> Result<(SphinxHeader, Vec<Key>), SphinxError> {
    let n = path.len();
    if n < 1 || n > MAX_HOPS {
        return Err(SphinxError::Malformed);
    }

    // Ephemeral key chain + per-hop shared secrets (blinding composes along the path).
    let (mut sk, eph0) = crypto.keypair_from_seed(eph_seed);
    let mut eph = eph0;
    let mut shareds: Vec<Key> = Vec::with_capacity(n);
    for hop_pub in path.iter() {
        let shared = crypto.shared_secret(&sk, hop_pub);
        shareds.push(shared);
        let bf = crypto.blinding_factor(&eph, &shared);
        sk = crypto.blind_secret(&sk, &bf);
        eph = crypto.blind_public(&eph, &bf);
    }

    // Filler: reproduces the zero-tail the relays XOR in as the routing block shifts.
    let filler = gen_filler(crypto, &shareds);

    // Routing block, built from the last hop backward.
    let mut beta = vec![0u8; ROUTING_LEN];
    let mut hmac_next = [0u8; MAC_LEN];
    for i in (0..n).rev() {
        let rho = crypto.subkey(&shareds[i], LABEL_RHO);
        if i == n - 1 {
            // Final hop record: FINAL flag, zero addr, zero mac.
            let mut b = vec![0u8; ROUTING_LEN];
            b[0] = FLAG_FINAL;
            let mut s = vec![0u8; ROUTING_LEN];
            crypto.stream(&rho, &mut s);
            for j in 0..ROUTING_LEN {
                b[j] ^= s[j];
            }
            // Overwrite the tail with the filler so the relay reconstruction lines up.
            let fl = filler.len();
            b[ROUTING_LEN - fl..].copy_from_slice(&filler);
            beta = b;
        } else {
            // Forward record: addr+mac of the NEXT hop, prepended; drop the tail (shift right).
            let mut rec = Vec::with_capacity(HOP_DATA_LEN);
            rec.push(FLAG_FORWARD);
            rec.extend_from_slice(&handle_bytes(&path[i + 1]));
            rec.extend_from_slice(&hmac_next);
            let mut nb = rec;
            nb.extend_from_slice(&beta);
            nb.truncate(ROUTING_LEN);
            let mut s = vec![0u8; ROUTING_LEN];
            crypto.stream(&rho, &mut s);
            for j in 0..ROUTING_LEN {
                nb[j] ^= s[j];
            }
            beta = nb;
        }
        let mu = crypto.subkey(&shareds[i], LABEL_MU);
        hmac_next = crypto.mac(&mu, &beta);
    }

    Ok((SphinxHeader { ephemeral: eph0, routing: beta, mac: hmac_next }, shareds))
}

/// Generate the deterministic filler (length `(n-1) * HOP_DATA_LEN`).
fn gen_filler<C: OnionCrypto>(crypto: &C, shareds: &[Key]) -> Vec<u8> {
    let n = shareds.len();
    let mut filler: Vec<u8> = Vec::new();
    for i in 0..n.saturating_sub(1) {
        let rho = crypto.subkey(&shareds[i], LABEL_RHO);
        let mut s = vec![0u8; ROUTING_LEN + HOP_DATA_LEN];
        crypto.stream(&rho, &mut s);
        // Grow filler by one hop record, then XOR with the stream tail.
        filler.extend(core::iter::repeat(0u8).take(HOP_DATA_LEN));
        let l = filler.len();
        let start = ROUTING_LEN + HOP_DATA_LEN - l;
        for j in 0..l {
            filler[j] ^= s[start + j];
        }
    }
    filler
}

/// Peel exactly one layer. `node_secret` is the relay's long-term secret key.
///
/// Pure function: takes a packet, returns either a (re-wrapped, same-size) packet to forward or the
/// delivered payload. It holds NO state and stores NOTHING — the zero-persistence invariant of a relay
/// is structural here (see [`crate::relay`]).
pub fn process_packet<C: OnionCrypto>(
    crypto: &C,
    node_secret: &Key,
    packet: &SphinxPacket,
) -> Result<ProcessResult, SphinxError> {
    if !packet.is_well_formed() {
        return Err(SphinxError::Malformed);
    }
    let shared = crypto.shared_secret(node_secret, &packet.header.ephemeral);

    // Integrity first: reject anything that does not authenticate for this hop.
    let mu = crypto.subkey(&shared, LABEL_MU);
    let expect = crypto.mac(&mu, &packet.header.routing);
    if expect != packet.header.mac {
        return Err(SphinxError::BadMac);
    }

    // Peel the routing block: XOR the stream over (routing || HOP_DATA_LEN zeros).
    let rho = crypto.subkey(&shared, LABEL_RHO);
    let mut s = vec![0u8; ROUTING_LEN + HOP_DATA_LEN];
    crypto.stream(&rho, &mut s);
    let mut b = vec![0u8; ROUTING_LEN + HOP_DATA_LEN];
    b[..ROUTING_LEN].copy_from_slice(&packet.header.routing);
    for j in 0..b.len() {
        b[j] ^= s[j];
    }
    let flags = b[0];
    let mut next_addr = [0u8; ADDR_LEN];
    next_addr.copy_from_slice(&b[FLAG_LEN..FLAG_LEN + ADDR_LEN]);
    let mut next_mac = [0u8; MAC_LEN];
    next_mac.copy_from_slice(&b[FLAG_LEN + ADDR_LEN..HOP_DATA_LEN]);
    let next_routing = b[HOP_DATA_LEN..].to_vec(); // length ROUTING_LEN

    // Peel one payload layer.
    let pk = crypto.subkey(&shared, LABEL_PAY);
    let mut ps = vec![0u8; PAYLOAD_LEN];
    crypto.stream(&pk, &mut ps);
    let mut pay = packet.payload.clone();
    for j in 0..PAYLOAD_LEN {
        pay[j] ^= ps[j];
    }

    if flags == FLAG_FINAL {
        Ok(ProcessResult::Deliver { payload: pay })
    } else {
        // Blind the ephemeral element for the next hop.
        let bf = crypto.blinding_factor(&packet.header.ephemeral, &shared);
        let next_eph = crypto.blind_public(&packet.header.ephemeral, &bf);
        Ok(ProcessResult::Forward {
            next: next_addr,
            packet: SphinxPacket {
                header: SphinxHeader { ephemeral: next_eph, routing: next_routing, mac: next_mac },
                payload: pay,
            },
        })
    }
}

#[cfg(all(test, feature = "insecure-toy-crypto"))]
mod tests {
    use super::*;
    use crate::crypto::InsecureToyOnionCrypto;

    /// Set up `n` relays with deterministic keys; return (secrets, publics).
    fn relays(c: &InsecureToyOnionCrypto, n: usize) -> (Vec<Key>, Vec<[u8; 32]>) {
        let mut sks = Vec::new();
        let mut pks = Vec::new();
        for i in 0..n {
            let (sk, pk) = c.keypair_from_seed(&[(i as u8) + 10; 32]);
            sks.push(sk);
            pks.push(pk);
        }
        (sks, pks)
    }

    #[test]
    fn roundtrip_three_hops() {
        let c = InsecureToyOnionCrypto;
        let (sks, pks) = relays(&c, 3);
        let msg = b"ring ring -- ephemeral call setup";
        let mut pkt = create_packet(&c, &pks, msg, &[42u8; 32]).unwrap();

        // Hop 0 and hop 1 forward to the next relay; the addresses must match the handles.
        for i in 0..2 {
            assert!(pkt.is_well_formed(), "size invariant holds at hop {i}");
            match process_packet(&c, &sks[i], &pkt).unwrap() {
                ProcessResult::Forward { next, packet } => {
                    assert_eq!(next, handle_bytes(&pks[i + 1]), "hop {i} routes to next relay");
                    pkt = packet;
                }
                ProcessResult::Deliver { .. } => panic!("hop {i} should forward, not deliver"),
            }
        }
        // Last hop delivers the original message.
        assert!(pkt.is_well_formed());
        match process_packet(&c, &sks[2], &pkt).unwrap() {
            ProcessResult::Deliver { payload } => {
                assert_eq!(&payload[..msg.len()], msg);
            }
            ProcessResult::Forward { .. } => panic!("final hop should deliver"),
        }
    }

    #[test]
    fn roundtrip_full_length() {
        let c = InsecureToyOnionCrypto;
        let (sks, pks) = relays(&c, MAX_HOPS);
        let mut pkt = create_packet(&c, &pks, b"max hops", &[1u8; 32]).unwrap();
        for i in 0..MAX_HOPS - 1 {
            match process_packet(&c, &sks[i], &pkt).unwrap() {
                ProcessResult::Forward { packet, .. } => pkt = packet,
                _ => panic!("forward expected"),
            }
        }
        match process_packet(&c, &sks[MAX_HOPS - 1], &pkt).unwrap() {
            ProcessResult::Deliver { payload } => assert_eq!(&payload[..8], b"max hops"),
            _ => panic!("deliver expected"),
        }
    }

    #[test]
    fn tampered_routing_fails_mac() {
        let c = InsecureToyOnionCrypto;
        let (sks, pks) = relays(&c, 3);
        let mut pkt = create_packet(&c, &pks, b"x", &[5u8; 32]).unwrap();
        pkt.header.routing[10] ^= 0xff;
        assert_eq!(process_packet(&c, &sks[0], &pkt), Err(SphinxError::BadMac));
    }

    #[test]
    fn wrong_relay_key_fails() {
        let c = InsecureToyOnionCrypto;
        let (_sks, pks) = relays(&c, 3);
        let pkt = create_packet(&c, &pks, b"x", &[5u8; 32]).unwrap();
        let (wrong_sk, _) = c.keypair_from_seed(&[200u8; 32]);
        assert_eq!(process_packet(&c, &wrong_sk, &pkt), Err(SphinxError::BadMac));
    }

    #[test]
    fn size_is_constant_across_hops() {
        let c = InsecureToyOnionCrypto;
        let (sks, pks) = relays(&c, 4);
        let mut pkt = create_packet(&c, &pks, b"const", &[9u8; 32]).unwrap();
        let len0 = (pkt.header.routing.len(), pkt.payload.len());
        for i in 0..3 {
            match process_packet(&c, &sks[i], &pkt).unwrap() {
                ProcessResult::Forward { packet, .. } => {
                    assert_eq!((packet.header.routing.len(), packet.payload.len()), len0);
                    pkt = packet;
                }
                _ => panic!("forward expected"),
            }
        }
    }
}
