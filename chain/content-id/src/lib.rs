//! # content-id — deterministic content addressing for forum bodies (#651)
//!
//! `pallet-forum` stores a post's body as a 32-byte hash, not the text — the text lives off-chain on
//! IPFS. For that to be *verifiable* (and censorship-resistant: anyone can re-fetch and re-check), the
//! client and the chain must agree, byte-for-byte, on how a body maps to its address. This crate fixes
//! that mapping:
//!
//! - [`digest`] — `sha256(body)`, the 32 bytes that go on-chain (`Post.body`).
//! - [`cid_v1_raw`] — the IPFS **CIDv1** (raw codec `0x55`, sha256 multihash, multibase base32-lower)
//!   for the *same* body, i.e. what the client hands to IPFS to fetch it. Built from the same digest,
//!   so on-chain hash and IPFS address are two encodings of one fact.
//!
//! Body addressing is **single-block raw** (no UnixFS/dag-pb framing): the body is one content-
//! addressed blob, hashed whole. This keeps the derivation trivial and reproducible on-chain (no
//! chunking parameters to agree on). A raw-codec sha256 CIDv1 always renders as `bafkrei…`.
//!
//! Dependency-free: reuses the workspace SHA-256; base32 is inlined. `no_std` (alloc only) without the
//! default `std` feature.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::String;
use consensus_core::sha256::sha256;

/// The 32-byte content digest stored on-chain (`pallet-forum` `Post.body`). `sha256(body)`.
pub fn digest(body: &[u8]) -> [u8; 32] {
    sha256(body)
}

// CIDv1 prefix bytes for a raw-codec, sha256-multihash content id:
//   version 0x01 | codec 0x55 (raw) | multihash code 0x12 (sha2-256) | digest length 0x20 (32)
const CID_PREFIX: [u8; 4] = [0x01, 0x55, 0x12, 0x20];

/// The IPFS CIDv1 (raw codec, sha256, base32-lower multibase) for `body`. Renders as `bafkrei…`.
///
/// This is the address the client uses to `ipfs get` the body; verifying the fetched bytes is exactly
/// re-running [`digest`] and comparing to the on-chain hash.
pub fn cid_v1_raw(body: &[u8]) -> String {
    cid_v1_raw_from_digest(&digest(body))
}

/// Build the CIDv1 string directly from a 32-byte digest (when the digest is already known on-chain).
pub fn cid_v1_raw_from_digest(digest: &[u8; 32]) -> String {
    let mut bytes = [0u8; 4 + 32];
    bytes[..4].copy_from_slice(&CID_PREFIX);
    bytes[4..].copy_from_slice(digest);
    // Multibase prefix 'b' = base32 (RFC4648, lowercase, no padding).
    let mut out = String::from("b");
    base32_lower_nopad_into(&bytes, &mut out);
    out
}

/// Recover the 32-byte digest from a well-formed raw-codec sha256 CIDv1 string. Returns `None` if the
/// string is not a `b`-multibase, raw-codec, sha256 CIDv1.
pub fn digest_from_cid(cid: &str) -> Option<[u8; 32]> {
    let body = cid.strip_prefix('b')?;
    let bytes = base32_lower_nopad_decode(body)?;
    if bytes.len() != 4 + 32 || bytes[..4] != CID_PREFIX {
        return None;
    }
    let mut d = [0u8; 32];
    d.copy_from_slice(&bytes[4..]);
    // Reject non-canonical encodings (e.g. trailing-bit malleability): a valid CID must round-trip to
    // exactly the same string. This makes the CID->digest mapping injective.
    if cid_v1_raw_from_digest(&d) != cid {
        return None;
    }
    Some(d)
}

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn base32_lower_nopad_into(data: &[u8], out: &mut String) {
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in data {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
}

fn base32_lower_nopad_decode(s: &str) -> Option<alloc::vec::Vec<u8>> {
    let mut out = alloc::vec::Vec::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for c in s.bytes() {
        let val = match c {
            b'a'..=b'z' => c - b'a',
            b'2'..=b'7' => c - b'2' + 26,
            _ => return None,
        } as u32;
        buffer = (buffer << 5) | val;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_cid_has_canonical_prefix() {
        // Every raw-codec sha256 CIDv1 renders with the well-known "bafkrei" prefix.
        let cid = cid_v1_raw(b"hello world");
        assert!(cid.starts_with("bafkrei"), "got {cid}");
    }

    #[test]
    fn digest_matches_onchain_field() {
        // What the client computes for IPFS must equal what goes on-chain.
        let body = b"a forum post body";
        assert_eq!(digest(body), sha256(body));
        assert_eq!(cid_v1_raw(body), cid_v1_raw_from_digest(&digest(body)));
    }

    #[test]
    fn cid_roundtrips_to_digest() {
        let body = b"verify me against the chain";
        let cid = cid_v1_raw(body);
        assert_eq!(digest_from_cid(&cid), Some(digest(body)));
    }

    #[test]
    fn cid_length_is_fixed() {
        // 36 bytes -> ceil(36*8/5) = 58 base32 chars, + 'b' multibase = 59.
        assert_eq!(cid_v1_raw(b"x").len(), 59);
    }

    #[test]
    fn rejects_malformed_cid() {
        assert!(digest_from_cid("Qmnotbase32").is_none()); // CIDv0, wrong multibase
        assert!(digest_from_cid("bzzzz").is_none()); // too short
        assert!(digest_from_cid("xbafkrei").is_none()); // wrong multibase prefix
    }

    #[test]
    fn digest_from_cid_accepts_canonical_only() {
        // Invariant the hardening guarantees: any accepted CID is exactly the canonical encoding of the
        // digest it yields. So a non-canonical string for the same digest can never be accepted.
        let cid = cid_v1_raw(b"canonical only");
        let d = digest_from_cid(&cid).unwrap();
        assert_eq!(cid_v1_raw_from_digest(&d), cid);
        // Construct a non-canonical encoding of the SAME digest by setting the padding bits non-zero:
        // the last base32 symbol carries 3 data bits + 2 padding bits; bump it past the canonical value.
        let mut s = cid.clone();
        let last = s.pop().unwrap();
        // canonical last symbol has padding bits 0; the next symbol up encodes the same data bits with a
        // non-zero padding bit, decoding to the same digest but non-canonically.
        const ALPH: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
        let idx = ALPH.iter().position(|&c| c as char == last).unwrap();
        s.push(ALPH[(idx + 1) % 32] as char);
        if let Some(d2) = digest_from_cid(&s) {
            // if it parsed, it must be a *different* canonical CID, never this digest non-canonically
            assert_eq!(cid_v1_raw_from_digest(&d2), s);
            assert_ne!(d2, d, "non-canonical encoding of the same digest must be rejected");
        }
    }
}

#[cfg(test)]
mod vectors {
    use super::*;
    #[test]
    fn known_ipfs_raw_vectors() {
        // `ipfs add --raw-leaves --cid-version=1` of empty input (sha256 of "").
        assert_eq!(cid_v1_raw(b""), "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku");
    }
}
