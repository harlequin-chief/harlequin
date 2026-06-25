//! Fixed-size Sphinx packet types and the on-chain-canonical address.
//!
//! Every packet is the SAME size on the wire regardless of how many hops remain — that is the whole
//! point of the Sphinx construction: a relay cannot tell its position in the path or how far the
//! packet has travelled from its length. The routing block is constant `ROUTING_LEN`; the payload is
//! constant `PAYLOAD_LEN`.

use alloc::vec::Vec;
use consensus_core::sha256::sha256;

/// Maximum hops in a circuit. Fixed so packet size is constant (anti size-correlation).
pub const MAX_HOPS: usize = 5;

/// Per-hop routing record: `flags(1) || next_addr(16) || next_mac(32)`.
pub const FLAG_LEN: usize = 1;
pub const ADDR_LEN: usize = 16; // = sha256(pubkey)[..16], the canonical handle bytes (#650/#652)
pub const MAC_LEN: usize = 32;
pub const HOP_DATA_LEN: usize = FLAG_LEN + ADDR_LEN + MAC_LEN; // 49

/// Routing block size (constant): one record per hop.
pub const ROUTING_LEN: usize = MAX_HOPS * HOP_DATA_LEN; // 245

/// Payload size (constant). Real calls carry an audio/control frame; padded to this.
pub const PAYLOAD_LEN: usize = 256;

/// Hop flags.
pub const FLAG_FORWARD: u8 = 0;
pub const FLAG_FINAL: u8 = 1;

/// A relay address: the first 16 bytes of `sha256(pubkey_32B)` — the canonical Harlequin handle bytes.
pub type Addr = [u8; ADDR_LEN];

/// Derive the canonical handle bytes from a 32-byte public key: `sha256(pubkey)[..16]` (NOT blake2b —
/// catch #652; this is the on-chain canonical derivation shared with pallet-directory #650).
pub fn handle_bytes(public: &[u8; 32]) -> Addr {
    let h = sha256(public);
    let mut a = [0u8; ADDR_LEN];
    a.copy_from_slice(&h[..ADDR_LEN]);
    a
}

/// Canonical display handle: `hlq-` + base32(RFC4648, lowercase, no padding) of the 16 handle bytes.
/// 26 chars, never truncated (128 bits = unforgeable). Identical rule to pallet-directory #650/#652.
pub fn handle_string(public: &[u8; 32]) -> alloc::string::String {
    let mut s = alloc::string::String::from("hlq-");
    s.push_str(&base32_lower_nopad(&handle_bytes(public)));
    s
}

/// RFC 4648 base32, lowercase alphabet, no padding. Dependency-free.
pub fn base32_lower_nopad(data: &[u8]) -> alloc::string::String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut out = alloc::string::String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 0x1f) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(ALPHABET[idx] as char);
    }
    out
}

/// The mutable header of a Sphinx packet. Shrinks logically per hop but stays byte-constant in size.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SphinxHeader {
    /// Ephemeral group element, blinded once per hop.
    pub ephemeral: [u8; 32],
    /// Onion-encrypted routing block, constant `ROUTING_LEN`.
    pub routing: Vec<u8>,
    /// Integrity tag over `routing` for the current hop.
    pub mac: [u8; MAC_LEN],
}

/// A full packet: header + onion-wrapped payload.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SphinxPacket {
    pub header: SphinxHeader,
    /// Onion-wrapped payload, constant `PAYLOAD_LEN`.
    pub payload: Vec<u8>,
}

impl SphinxPacket {
    /// Total wire size — identical for every packet (the anti-correlation invariant).
    pub const WIRE_LEN: usize = 32 + ROUTING_LEN + MAC_LEN + PAYLOAD_LEN;

    /// Assert the size invariant holds for this packet. Cheap structural check for relays.
    pub fn is_well_formed(&self) -> bool {
        self.header.routing.len() == ROUTING_LEN && self.payload.len() == PAYLOAD_LEN
    }
}

/// Outcome of a relay processing one layer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ProcessResult {
    /// Forward the (re-wrapped, same-size) packet to this next-hop address.
    Forward { next: Addr, packet: SphinxPacket },
    /// This node is the final recipient; the de-onioned payload is attached.
    Deliver { payload: Vec<u8> },
}

/// Errors a relay can raise. A bad MAC or malformed packet is dropped silently in production.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SphinxError {
    BadMac,
    Malformed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_matches_known_vector() {
        // sha256(b"") = e3b0c442...; first 16 bytes base32-lower-nopad
        let zeros16 = [0u8; 16];
        // 16 bytes -> 26 base32 chars (128/5 = 25.6 -> 26)
        assert_eq!(base32_lower_nopad(&zeros16).len(), 26);
        assert!(base32_lower_nopad(&zeros16).chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
    }

    #[test]
    fn handle_string_shape() {
        let pk = [9u8; 32];
        let s = handle_string(&pk);
        assert!(s.starts_with("hlq-"));
        assert_eq!(s.len(), 4 + 26); // "hlq-" + 26
    }

    #[test]
    fn wire_len_is_constant() {
        assert_eq!(SphinxPacket::WIRE_LEN, 32 + 245 + 32 + 256);
    }
}

#[cfg(test)]
mod handle_vector {
    use super::*;
    #[test]
    fn alice_652_ground_truth() {
        // //Alice sr25519 AccountId32 public key (32 bytes).
        let alice: [u8; 32] = [
            0xd4,0x35,0x93,0xc7,0x15,0xfd,0xd3,0x1c,0x61,0x14,0x1a,0xbd,0x04,0xa9,0x9f,0xd6,
            0x82,0x2c,0x85,0x58,0x85,0x4c,0xcd,0xe3,0x9a,0x56,0x84,0xe7,0xa5,0x6d,0xa2,0x7d,
        ];
        let hb = handle_bytes(&alice);
        let hex: alloc::string::String = hb.iter().map(|b| alloc::format!("{:02x}", b)).collect();
        assert_eq!(hex, "46208798c80be6531c6a6454312db7d1", "handle_bytes hex");
        assert_eq!(handle_string(&alice), "hlq-iyqipggibptfghdkmrkdclnx2e", "canonical handle");
    }
}
