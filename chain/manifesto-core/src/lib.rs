//! The manifesto seal — Harlequin's immutable founding vessel (Art. XII, *Permanence*).
//!
//! The manifesto is the constitution of the society. Its *permanence* is not a promise of good
//! behaviour; it is a property of the chain: the canonical text and its SHA-256 are fixed in the
//! **genesis block (block 0)** and there is **no extrinsic that can mutate them**. Anyone can recompute
//! the hash from the text and check it against what genesis sealed — so the State (or a future captured
//! majority) cannot quietly rewrite the founding pact. This crate is the dependency-free, `no_std`
//! *vessel*: the integrity primitive that the runtime embeds in genesis. The runtime/pallet enforces
//! the "no mutator" half (FRAME genesis-only storage); this crate guarantees the "text matches hash"
//! half, with the same hand-rolled SHA-256 the consensus uses (one hash across the whole chain).
//!
//! **The final text enters only at launch, after the adversarial audit and the freeze** (the vessel is
//! buildable now; the words are sealed last — see `project_manifesto_genesis_freeze`). Until then any
//! bytes can be sealed and verified; the mechanism does not depend on the content.

#![cfg_attr(not(feature = "std"), no_std)]

use consensus_core::sha256::sha256;

/// Length of the seal digest (SHA-256).
pub const HASH_LEN: usize = 32;

/// The immutable seal embedded in genesis: the SHA-256 of the canonical manifesto bytes. Plain data,
/// `Copy`, trivially encodable into `genesis_config`. The text itself is stored alongside it in the
/// runtime; this is the fingerprint that makes any later tampering detectable by anyone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ManifestoSeal {
    /// SHA-256 of the canonical manifesto bytes.
    pub hash: [u8; HASH_LEN],
}

impl ManifestoSeal {
    /// Seal the given canonical manifesto bytes (compute the genesis fingerprint).
    pub fn seal(text: &[u8]) -> Self {
        ManifestoSeal { hash: sha256(text) }
    }

    /// Construct a seal from a known digest (e.g. one decoded from genesis storage).
    pub fn from_hash(hash: [u8; HASH_LEN]) -> Self {
        ManifestoSeal { hash }
    }

    /// True iff `text` is exactly the manifesto this seal fixed — the integrity check any node can run
    /// against the genesis-sealed hash.
    pub fn verify(&self, text: &[u8]) -> bool {
        sha256(text) == self.hash
    }

    /// Lowercase hex of the digest (64 ASCII bytes), for display / logs / the public panel.
    pub fn to_hex(&self) -> [u8; HASH_LEN * 2] {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = [0u8; HASH_LEN * 2];
        let mut i = 0;
        while i < HASH_LEN {
            out[i * 2] = HEX[(self.hash[i] >> 4) as usize];
            out[i * 2 + 1] = HEX[(self.hash[i] & 0x0f) as usize];
            i += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLACEHOLDER: &[u8] = b"Harlequin manifesto -- placeholder vessel; final text sealed at launch.";

    #[test]
    fn seal_then_verify_roundtrips() {
        let seal = ManifestoSeal::seal(PLACEHOLDER);
        assert!(seal.verify(PLACEHOLDER));
    }

    #[test]
    fn any_tampering_is_detected() {
        let seal = ManifestoSeal::seal(PLACEHOLDER);
        // A single flipped byte must fail verification.
        let mut tampered = PLACEHOLDER.to_vec();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        assert!(!seal.verify(&tampered));
        // Truncation too.
        assert!(!seal.verify(&PLACEHOLDER[..PLACEHOLDER.len() - 1]));
    }

    #[test]
    fn seal_is_deterministic() {
        assert_eq!(ManifestoSeal::seal(PLACEHOLDER), ManifestoSeal::seal(PLACEHOLDER));
    }

    #[test]
    fn from_hash_reconstructs_the_same_seal() {
        let seal = ManifestoSeal::seal(PLACEHOLDER);
        let rebuilt = ManifestoSeal::from_hash(seal.hash);
        assert_eq!(seal, rebuilt);
        assert!(rebuilt.verify(PLACEHOLDER));
    }

    #[test]
    fn hex_matches_known_empty_vector() {
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        let seal = ManifestoSeal::seal(b"");
        let hex = seal.to_hex();
        assert_eq!(
            &hex,
            b"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
