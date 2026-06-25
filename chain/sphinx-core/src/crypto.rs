//! Per-hop crypto for the onion relay layer — the `OnionCrypto` trait is the seam.
//!
//! THE CONTRACT. The Sphinx packet logic in [`crate::sphinx`] never touches a curve or a cipher
//! directly: it asks an `OnionCrypto` for shared secrets, blinding, a keystream and a MAC. That keeps
//! the *structure* (fixed-size layering, filler, integrity) independent of the primitives.
//!
//! THE STUB. [`StubOnionCrypto`] implements `OnionCrypto` over a **toy multiplicative group mod a
//! 61-bit Mersenne prime**. It is a real Diffie–Hellman group (so blinding composes correctly and the
//! end-to-end round-trip in the tests actually works), but it is **cryptographically WORTHLESS** — a
//! 61-bit group is broken by hand. Its only job is to exercise the routing control-flow.
//!
//! AT INTEGRATION (the audited library is swept in): swap `StubOnionCrypto` for an *audited* Sphinx primitive set
//! (X25519 group, ChaCha20 keystream, a real MAC). Nothing else in this crate changes — that is the
//! point of the seam. No external crate is pulled onto the isolated station here; the audited library
//! is vetted and integrated downstream, not committed from the build box.

#[cfg(feature = "insecure-toy-crypto")]
use consensus_core::sha256::sha256;

/// 32-byte field — public key, secret scalar, shared secret. Wide enough for X25519 at integration.
pub type Key = [u8; 32];

/// Domain-separation labels for sub-key derivation (distinct per use).
pub const LABEL_RHO: &[u8] = b"hlq-onion-rho"; // routing-block keystream
pub const LABEL_MU: &[u8] = b"hlq-onion-mu"; // per-hop MAC key
pub const LABEL_PAY: &[u8] = b"hlq-onion-pay"; // payload keystream

/// Primitives the onion layer needs from *whatever* group/cipher backs it.
///
/// Implementations must satisfy the Diffie–Hellman relation that makes onion routing work:
/// `shared_secret(a, derive_public(b)) == shared_secret(b, derive_public(a))`, and blinding must
/// compose: `derive_public(blind_secret(s, f)) == blind_public(derive_public(s), f)`.
pub trait OnionCrypto {
    /// Deterministic keypair from a seed (test/setup helper). Returns `(secret, public)`.
    fn keypair_from_seed(&self, seed: &Key) -> (Key, Key);
    /// Public element for a secret scalar.
    fn derive_public(&self, secret: &Key) -> Key;
    /// Diffie–Hellman shared secret.
    fn shared_secret(&self, secret: &Key, peer_public: &Key) -> Key;
    /// Blinding factor derived from the (public, shared) pair at a hop.
    fn blinding_factor(&self, public: &Key, shared: &Key) -> Key;
    /// Apply a blinding factor to a public element.
    fn blind_public(&self, public: &Key, factor: &Key) -> Key;
    /// Apply a blinding factor to a secret scalar.
    fn blind_secret(&self, secret: &Key, factor: &Key) -> Key;
    /// Derive a labelled sub-key from a shared secret.
    fn subkey(&self, shared: &Key, label: &[u8]) -> Key;
    /// Fill `out` with a keystream from `key` (XORed over routing block / payload).
    fn stream(&self, key: &Key, out: &mut [u8]);
    /// Message authentication tag over `data`.
    fn mac(&self, key: &Key, data: &[u8]) -> Key;
}

// ---------------------------------------------------------------------------------------------------
// Toy group: multiplicative group mod P = 2^61 - 1. INSECURE — prototype only.
//
// SAFETY GUARDRAIL: the toy lives behind the `insecure-toy-crypto` feature, which is NOT in
// the default set. A plain `cargo build` (release or debug) compiles NONE of this — there is no build
// path that links the toy into anything network-facing. Tests opt in with `--features insecure-toy-crypto`.
// `INSECURE_DO_NOT_SHIP` is a loud, asserted marker (see the test below).
// ---------------------------------------------------------------------------------------------------

/// Compile-time scream: this group is not safe for any real traffic. Asserted by a unit test.
#[cfg(feature = "insecure-toy-crypto")]
pub const INSECURE_DO_NOT_SHIP: bool = true;

#[cfg(feature = "insecure-toy-crypto")]
const P: u128 = (1u128 << 61) - 1; // Mersenne prime 2_305_843_009_213_693_951
#[cfg(feature = "insecure-toy-crypto")]
const ORDER: u128 = P - 1; // exponent arithmetic is mod (P-1)
#[cfg(feature = "insecure-toy-crypto")]
const G: u128 = 37; // fixed base

#[cfg(feature = "insecure-toy-crypto")]
#[inline]
fn modpow(mut base: u128, mut exp: u128, modulus: u128) -> u128 {
    let mut r = 1u128;
    base %= modulus;
    while exp > 0 {
        if exp & 1 == 1 {
            r = r * base % modulus;
        }
        exp >>= 1;
        base = base * base % modulus;
    }
    r
}

#[cfg(feature = "insecure-toy-crypto")]
#[inline]
fn enc(x: u64) -> Key {
    let mut o = [0u8; 32];
    o[..8].copy_from_slice(&x.to_le_bytes());
    o
}

#[cfg(feature = "insecure-toy-crypto")]
#[inline]
fn dec(b: &Key) -> u64 {
    let mut t = [0u8; 8];
    t.copy_from_slice(&b[..8]);
    u64::from_le_bytes(t)
}

/// **INSECURE TOY** prototype `OnionCrypto`. A 61-bit DH group — broken by hand. Exists ONLY to make
/// the routing structure testable end-to-end. Feature-gated (`insecure-toy-crypto`, non-default) so no
/// release build can link it. See module docs. Replaced by an audited primitive set at integration.
#[cfg(feature = "insecure-toy-crypto")]
#[derive(Clone, Copy, Default)]
pub struct InsecureToyOnionCrypto;

#[cfg(feature = "insecure-toy-crypto")]
impl OnionCrypto for InsecureToyOnionCrypto {
    fn keypair_from_seed(&self, seed: &Key) -> (Key, Key) {
        let mut h = sha256(&[b"hlq-onion-sk", &seed[..]].concat());
        // reduce to a nonzero scalar in [1, ORDER)
        let mut s = (dec(&h) as u128 % ORDER) as u64;
        if s == 0 {
            h = sha256(&h);
            s = (dec(&h) as u128 % (ORDER - 1)) as u64 + 1;
        }
        let sk = enc(s);
        let pk = self.derive_public(&sk);
        (sk, pk)
    }

    fn derive_public(&self, secret: &Key) -> Key {
        let s = dec(secret) as u128 % ORDER;
        enc(modpow(G, s, P) as u64)
    }

    fn shared_secret(&self, secret: &Key, peer_public: &Key) -> Key {
        let s = dec(secret) as u128 % ORDER;
        let p = dec(peer_public) as u128 % P;
        enc(modpow(p, s, P) as u64)
    }

    fn blinding_factor(&self, public: &Key, shared: &Key) -> Key {
        let h = sha256(&[b"hlq-onion-blind".as_ref(), &public[..], &shared[..]].concat());
        let mut v = dec(&h) as u128 % ORDER;
        if v == 0 {
            v = 1;
        }
        enc(v as u64)
    }

    fn blind_public(&self, public: &Key, factor: &Key) -> Key {
        let p = dec(public) as u128 % P;
        let b = dec(factor) as u128 % ORDER;
        enc(modpow(p, b, P) as u64)
    }

    fn blind_secret(&self, secret: &Key, factor: &Key) -> Key {
        let s = dec(secret) as u128 % ORDER;
        let b = dec(factor) as u128 % ORDER;
        enc(((s * b) % ORDER) as u64)
    }

    fn subkey(&self, shared: &Key, label: &[u8]) -> Key {
        sha256(&[label, &shared[..]].concat())
    }

    fn stream(&self, key: &Key, out: &mut [u8]) {
        // SHA-256 counter mode: block_i = sha256(key || i_le_u32).
        let mut counter: u32 = 0;
        let mut off = 0;
        while off < out.len() {
            let block = sha256(&[&key[..], &counter.to_le_bytes()].concat());
            let n = core::cmp::min(32, out.len() - off);
            out[off..off + n].copy_from_slice(&block[..n]);
            off += n;
            counter = counter.wrapping_add(1);
        }
    }

    fn mac(&self, key: &Key, data: &[u8]) -> Key {
        sha256(&[&key[..], data].concat())
    }
}

#[cfg(all(test, feature = "insecure-toy-crypto"))]
mod tests {
    use super::*;

    #[test]
    fn toy_is_flagged_insecure() {
        // The guardrail marker exists and screams. A release build (no feature) never compiles it.
        assert!(INSECURE_DO_NOT_SHIP);
    }

    #[test]
    fn dh_is_symmetric() {
        let c = InsecureToyOnionCrypto;
        let (sa, pa) = c.keypair_from_seed(&[1u8; 32]);
        let (sb, pb) = c.keypair_from_seed(&[2u8; 32]);
        assert_eq!(c.shared_secret(&sa, &pb), c.shared_secret(&sb, &pa));
    }

    #[test]
    fn blinding_composes() {
        // derive_public(blind_secret(s,f)) == blind_public(derive_public(s), f)
        let c = InsecureToyOnionCrypto;
        let (s, p) = c.keypair_from_seed(&[7u8; 32]);
        let f = c.blinding_factor(&p, &[9u8; 32]);
        assert_eq!(c.derive_public(&c.blind_secret(&s, &f)), c.blind_public(&p, &f));
    }

    #[test]
    fn stream_is_deterministic_and_long() {
        let c = InsecureToyOnionCrypto;
        let k = [3u8; 32];
        let mut a = [0u8; 100];
        let mut b = [0u8; 100];
        c.stream(&k, &mut a);
        c.stream(&k, &mut b);
        assert_eq!(a, b);
        // not all-zero (keystream actually filled past one block)
        assert!(a[64..].iter().any(|&x| x != 0));
    }
}
