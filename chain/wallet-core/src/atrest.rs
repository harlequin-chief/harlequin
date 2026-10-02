//! At-rest key encryption for the wallet (#664, 2026-06-25).
//!
//! The wallet's secret is the 32-byte entropy that seeds the sr25519 mask ([`crate::Wallet::from_entropy`]).
//! On disk it must NEVER be plaintext. This module seals it under a user passphrase:
//!
//!   key   = Argon2id(passphrase, salt)            — memory-hard KDF, resists brute force
//!   blob  = XChaCha20-Poly1305(key, nonce, entropy)  — AEAD: tamper/ wrong-pass → fails closed
//!
//! Vetted primitives only (RustCrypto `argon2` + `chacha20poly1305`, pinned + audited). RNG-free like the
//! rest of the core: the caller supplies `salt` (16B) and `nonce` (24B) from the host OS RNG — so the crate
//! pulls no randomness and stays deterministically testable, and the entropy source is the host's explicit
//! responsibility. The derived key is zeroized after use.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use zeroize::Zeroize;

/// Argon2id cost parameters. Fixed + stored so a keystore is always reproducible. ~19 MiB, 2 passes
/// (OWASP-leaning interactive defaults; the app may raise them later via a versioned keystore).
const M_COST_KIB: u32 = 19_456;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

#[derive(Debug, PartialEq, Eq)]
pub enum AtRestError {
    /// KDF failed (bad params).
    Kdf,
    /// AEAD open failed: wrong passphrase, tampered blob, or wrong salt/nonce. Fails closed.
    Decrypt,
}

/// An encrypted wallet keystore. Self-describing (salt + nonce + ciphertext); safe to store on disk.
/// The plaintext (the 32-byte entropy) is never present here.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EncryptedKeystore {
    pub salt: [u8; 16],
    pub nonce: [u8; 24],
    /// XChaCha20-Poly1305 ciphertext of the 32-byte entropy = 32 + 16-byte tag = 48 bytes.
    pub ciphertext: Vec<u8>,
}

fn derive_key(passphrase: &[u8], salt: &[u8; 16]) -> Result<[u8; 32], AtRestError> {
    let params = Params::new(M_COST_KIB, T_COST, P_COST, Some(32)).map_err(|_| AtRestError::Kdf)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon.hash_password_into(passphrase, salt, &mut key).map_err(|_| AtRestError::Kdf)?;
    Ok(key)
}

/// Seal the 32-byte wallet entropy under `passphrase`. `salt`/`nonce` come from the host OS RNG.
pub fn seal(
    entropy: &[u8; 32],
    passphrase: &[u8],
    salt: &[u8; 16],
    nonce: &[u8; 24],
) -> Result<EncryptedKeystore, AtRestError> {
    let mut key = derive_key(passphrase, salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(nonce), entropy.as_slice())
        .map_err(|_| AtRestError::Kdf)?;
    key.zeroize();
    Ok(EncryptedKeystore { salt: *salt, nonce: *nonce, ciphertext })
}

/// Open a keystore with `passphrase`. Wrong passphrase / tampered blob → `Decrypt` (fail closed).
pub fn open(ks: &EncryptedKeystore, passphrase: &[u8]) -> Result<[u8; 32], AtRestError> {
    let mut key = derive_key(passphrase, &ks.salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let plaintext = cipher
        .decrypt(XNonce::from_slice(&ks.nonce), ks.ciphertext.as_slice())
        .map_err(|_| AtRestError::Decrypt);
    key.zeroize();
    let mut pt = plaintext?;
    if pt.len() != 32 {
        pt.zeroize();
        return Err(AtRestError::Decrypt);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&pt);
    pt.zeroize();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Wallet;

    const SALT: [u8; 16] = [5u8; 16];
    const NONCE: [u8; 24] = [6u8; 24];

    #[test]
    fn seal_open_roundtrip() {
        let entropy = [9u8; 32];
        let ks = seal(&entropy, b"correct horse battery", &SALT, &NONCE).unwrap();
        assert_ne!(ks.ciphertext.as_slice(), entropy.as_slice(), "ciphertext is not the plaintext");
        assert_eq!(ks.ciphertext.len(), 48, "32 entropy + 16 tag");
        let got = open(&ks, b"correct horse battery").unwrap();
        assert_eq!(got, entropy);
    }

    #[test]
    fn roundtrip_reconstructs_same_wallet() {
        let entropy = [11u8; 32];
        let w1 = Wallet::from_entropy(&entropy).unwrap();
        let ks = seal(&entropy, b"passphrase", &SALT, &NONCE).unwrap();
        let recovered = open(&ks, b"passphrase").unwrap();
        let w2 = Wallet::from_entropy(&recovered).unwrap();
        assert_eq!(w1.address(), w2.address(), "same passphrase recovers the same mask/wallet");
    }

    #[test]
    fn wrong_passphrase_fails_closed() {
        let ks = seal(&[9u8; 32], b"right", &SALT, &NONCE).unwrap();
        assert_eq!(open(&ks, b"wrong"), Err(AtRestError::Decrypt));
    }

    #[test]
    fn tampered_ciphertext_fails_closed() {
        let mut ks = seal(&[9u8; 32], b"pass", &SALT, &NONCE).unwrap();
        ks.ciphertext[0] ^= 1;
        assert_eq!(open(&ks, b"pass"), Err(AtRestError::Decrypt));
    }

    #[test]
    fn wrong_salt_fails_closed() {
        let mut ks = seal(&[9u8; 32], b"pass", &SALT, &NONCE).unwrap();
        ks.salt[0] ^= 1; // different salt → different KDF key → AEAD fails
        assert_eq!(open(&ks, b"pass"), Err(AtRestError::Decrypt));
    }

    #[test]
    fn deterministic_given_salt_and_nonce() {
        // Same inputs → same blob (no hidden RNG inside the crate).
        let a = seal(&[9u8; 32], b"pass", &SALT, &NONCE).unwrap();
        let b = seal(&[9u8; 32], b"pass", &SALT, &NONCE).unwrap();
        assert_eq!(a, b);
    }
}
