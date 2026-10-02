//! Harlequin self-custody wallet core (#664, 2026-06-25).
//!
//! The cryptographic heart of the standalone wallet app: a mask (sr25519 keypair) is the account; its
//! address is the canonical handle `hlq-…` = `sha256(pubkey)[..16]` (same identity as the chain #650/#652
//! and the `hlq-transport` signed prekey). The wallet **holds the key, signs transactions, and never
//! lets the private key leave in the clear** — self-custody, no server, no custodian.
//!
//! ## Design choices (pro)
//! - **RNG-free:** key creation takes caller-supplied 32-byte entropy ([`Wallet::from_entropy`]). The host
//!   app provides OS entropy; tests provide fixed seeds. The crate pulls no RNG → deterministic + testable,
//!   and the entropy source is the host's explicit responsibility.
//! - **Domain-separated signing:** every signature is over an explicit context label, so a signature for
//!   one purpose (e.g. a transaction) can never be replayed as another (e.g. a session prekey). The
//!   session prekey label lives in `hlq-transport`; transactions use [`TX_DOMAIN`] here.
//! - **At-rest encryption is a SEPARATE module** (passphrase → KDF → AEAD), added next; this core deals in
//!   live key material only and zeroizes via schnorrkel's own `Drop`.

pub mod atrest;
pub mod mnemonic;

use data_encoding::BASE32_NOPAD;
use schnorrkel::{signing_context, ExpansionMode, Keypair, MiniSecretKey, PublicKey, Signature};
use sha2::{Digest, Sha256};

/// sr25519 signing context = the Substrate standard `b"substrate"`. The CHAIN verifies every sr25519
/// signature (extrinsics via MultiSignature, finality votes via `sp_core::sr25519::Pair::verify`) under this
/// context; a custom context would fail on-chain verification. So the wallet MUST use it too, and domain
/// separation is done in the MESSAGE, not the context (see [`LOGIN_PREFIX`]).
pub const SUBSTRATE_CTX: &[u8] = b"substrate";

/// Villa login domain label (#637, contract frozen 2026-07-21). The login message is
/// `label(18) ++ nonce(32) ++ genesis_hash(32) ++ issued_at(u64 LE, 8) = 90 bytes`, signed under
/// [`SUBSTRATE_CTX`]. Domain label + fixed layout: a login signature can never be replayed as a
/// transaction, another chain's login (genesis differs), or an expired challenge (issued_at differs).
/// The mediator verifier builds the byte-identical message — see REF-VECTORS-637-837.md.
pub const VILLA_LOGIN_LABEL: &[u8; 18] = b"hlq-villa-login-v1";

/// Node↔mask binding PoP domain label (#837, contract frozen 2026-07-21). The proof-of-possession
/// message is `label(16) ++ mask_account_id(32 raw bytes, NOT ss58) = 48 bytes`, signed by the NODE's
/// session key under [`SUBSTRATE_CTX`] (pattern = `pallet_session::set_keys` proof). No genesis_hash:
/// the runtime cannot verify it (block_hash(0) pruned); cross-chain replay is killed by `CheckGenesis`
/// on the signed extrinsic envelope.
pub const NODE_BIND_LABEL: &[u8; 16] = b"hlq-node-bind-v1";

#[derive(Debug, PartialEq, Eq)]
pub enum WalletError {
    /// Entropy/seed was not a valid sr25519 mini-secret.
    BadEntropy,
    /// A public key blob did not decode.
    BadPublicKey,
    /// A signature blob did not decode.
    BadSignature,
    /// Signature verification failed.
    VerifyFailed,
}

/// A self-custody wallet: one sr25519 mask. Holds the keypair; the private half never leaves in clear.
pub struct Wallet {
    keypair: Keypair,
}

impl Wallet {
    /// Create a wallet from 32 bytes of caller-supplied entropy (OS RNG in the app; fixed seed in tests).
    /// Deterministic: the same entropy always yields the same mask.
    pub fn from_entropy(entropy: &[u8; 32]) -> Result<Self, WalletError> {
        let mini = MiniSecretKey::from_bytes(entropy).map_err(|_| WalletError::BadEntropy)?;
        Ok(Wallet { keypair: mini.expand_to_keypair(ExpansionMode::Ed25519) })
    }

    /// The 32-byte sr25519 public key (the on-chain account identity).
    pub fn public_key(&self) -> [u8; 32] {
        self.keypair.public.to_bytes()
    }

    /// The wallet address = the canonical mask handle `hlq-…` (`sha256(pubkey)[..16]`, base32 nopad lower).
    /// Identical derivation to the chain directory (#650) and the transport prekey handle — one identity.
    pub fn address(&self) -> String {
        let digest = Sha256::digest(self.public_key());
        let b32 = BASE32_NOPAD.encode(&digest[..16]).to_lowercase();
        format!("hlq-{b32}")
    }

    /// Sign a transaction blob under the wallet TX domain. The signature authenticates `tx_bytes` to this
    /// mask and cannot be replayed for any other domain.
    pub fn sign_tx(&self, tx_bytes: &[u8]) -> [u8; 64] {
        let ctx = signing_context(SUBSTRATE_CTX);
        self.keypair.sign(ctx.bytes(tx_bytes)).to_bytes()
    }

    /// Sign an extrinsic's `SignedPayload` for on-chain governance (propose/approve/apply_upgrade, #837).
    /// This is the SUBSTRATE-STANDARD extrinsic signature — NO domain prefix — over exactly the bytes the
    /// runtime verifies. It mirrors `sp_runtime::generic::SignedPayload`: the payload is SCALE
    /// `(call || extra || additional)`; if it exceeds 256 bytes it is `blake2_256`-hashed FIRST, else
    /// signed as-is (the `apply_upgrade` payload carries the >1 MB WASM → the hash path is load-bearing;
    /// omitting it makes the signature fail on-chain). The founder mask signs LOCALLY; only the 64-byte
    /// signature leaves — the seed never does.
    pub fn sign_extrinsic(&self, signed_payload: &[u8]) -> [u8; 64] {
        let ctx = signing_context(SUBSTRATE_CTX);
        if signed_payload.len() > 256 {
            use blake2::{digest::consts::U32, Blake2b, Digest as _};
            let hash = Blake2b::<U32>::digest(signed_payload);
            self.keypair.sign(ctx.bytes(&hash)).to_bytes()
        } else {
            self.keypair.sign(ctx.bytes(signed_payload)).to_bytes()
        }
    }

    /// Verify an extrinsic signature (parity/tests; the CHAIN is the real verifier). Applies the same
    /// >256→`blake2_256` rule as [`Wallet::sign_extrinsic`].
    pub fn verify_extrinsic(
        pubkey: &[u8; 32],
        signed_payload: &[u8],
        sig: &[u8; 64],
    ) -> Result<(), WalletError> {
        let pk = PublicKey::from_bytes(pubkey).map_err(|_| WalletError::BadPublicKey)?;
        let signature = Signature::from_bytes(sig).map_err(|_| WalletError::BadSignature)?;
        let ctx = signing_context(SUBSTRATE_CTX);
        let ok = if signed_payload.len() > 256 {
            use blake2::{digest::consts::U32, Blake2b, Digest as _};
            let hash = Blake2b::<U32>::digest(signed_payload);
            pk.verify(ctx.bytes(&hash), &signature)
        } else {
            pk.verify(ctx.bytes(signed_payload), &signature)
        };
        ok.map_err(|_| WalletError::VerifyFailed)
    }

    /// Verify a transaction signature against a mask public key (any party can check; no secret needed).
    pub fn verify_tx(
        pubkey: &[u8; 32],
        tx_bytes: &[u8],
        sig: &[u8; 64],
    ) -> Result<(), WalletError> {
        let pk = PublicKey::from_bytes(pubkey).map_err(|_| WalletError::BadPublicKey)?;
        let signature = Signature::from_bytes(sig).map_err(|_| WalletError::BadSignature)?;
        let ctx = signing_context(SUBSTRATE_CTX);
        pk.verify(ctx.bytes(tx_bytes), &signature).map_err(|_| WalletError::VerifyFailed)
    }

    /// Sign a villa login challenge (#637) over [`villa_login_message`] under the Substrate context.
    /// Proves control of the mask NOW without revealing the key or any PII. The message layout is baked
    /// in here so the host layer passes the raw challenge fields and cannot induce a different domain.
    pub fn sign_villa_login(
        &self,
        nonce: &[u8; 32],
        genesis_hash: &[u8; 32],
        issued_at: u64,
    ) -> [u8; 64] {
        let ctx = signing_context(SUBSTRATE_CTX);
        self.keypair
            .sign(ctx.bytes(&villa_login_message(nonce, genesis_hash, issued_at)))
            .to_bytes()
    }

    /// Verify a villa login signature against a mask public key (the mediator does the equivalent with
    /// substrate-interface). A TX signature will NOT verify here — the domain label makes the message differ.
    pub fn verify_villa_login(
        pubkey: &[u8; 32],
        nonce: &[u8; 32],
        genesis_hash: &[u8; 32],
        issued_at: u64,
        sig: &[u8; 64],
    ) -> Result<(), WalletError> {
        let pk = PublicKey::from_bytes(pubkey).map_err(|_| WalletError::BadPublicKey)?;
        let signature = Signature::from_bytes(sig).map_err(|_| WalletError::BadSignature)?;
        let ctx = signing_context(SUBSTRATE_CTX);
        pk.verify(
            ctx.bytes(&villa_login_message(nonce, genesis_hash, issued_at)),
            &signature,
        )
        .map_err(|_| WalletError::VerifyFailed)
    }
}

/// The villa login message (#637): `label(18) ++ nonce(32) ++ genesis(32) ++ issued_at(u64 LE, 8)` = 90 B.
/// Built byte-identically here, in the mediator verifier and in the reference vectors.
pub fn villa_login_message(nonce: &[u8; 32], genesis_hash: &[u8; 32], issued_at: u64) -> [u8; 90] {
    let mut m = [0u8; 90];
    m[..18].copy_from_slice(VILLA_LOGIN_LABEL);
    m[18..50].copy_from_slice(nonce);
    m[50..82].copy_from_slice(genesis_hash);
    m[82..].copy_from_slice(&issued_at.to_le_bytes());
    m
}

/// The node↔mask binding PoP message (#837): `label(16) ++ mask_account_id(32 raw)` = 48 B. The NODE's
/// session key signs this; the wallet only needs to build it (to hand to the node) and to verify the
/// returned `pop_sig` before letting the mask sign the `set_vote_key` extrinsic.
pub fn node_bind_message(mask_account_id: &[u8; 32]) -> [u8; 48] {
    let mut m = [0u8; 48];
    m[..16].copy_from_slice(NODE_BIND_LABEL);
    m[16..].copy_from_slice(mask_account_id);
    m
}

/// Verify a node's binding proof-of-possession (#837): `pop_sig` must be a valid sr25519 signature by
/// `sk_pub` (the session key) over [`node_bind_message`] of this mask. Fail-closed, mirrors the pallet
/// check — lets the wallet reject a bad PoP BEFORE the mask signs anything.
pub fn verify_node_pop(
    sk_pub: &[u8; 32],
    mask_account_id: &[u8; 32],
    pop_sig: &[u8; 64],
) -> Result<(), WalletError> {
    let pk = PublicKey::from_bytes(sk_pub).map_err(|_| WalletError::BadPublicKey)?;
    let signature = Signature::from_bytes(pop_sig).map_err(|_| WalletError::BadSignature)?;
    let ctx = signing_context(SUBSTRATE_CTX);
    pk.verify(ctx.bytes(&node_bind_message(mask_account_id)), &signature)
        .map_err(|_| WalletError::VerifyFailed)
}

/// Derive the canonical address/handle from any 32-byte public key, without a wallet (for display of a
/// counterparty). Same derivation as [`Wallet::address`].
pub fn address_of(pubkey: &[u8; 32]) -> String {
    let digest = Sha256::digest(pubkey);
    let b32 = BASE32_NOPAD.encode(&digest[..16]).to_lowercase();
    format!("hlq-{b32}")
}

/// SS58 network prefix used on the wire with the mediator (#637). The LIVE chain declares 1728
/// (runtime `ConstU16<1728>` = 12³, Chief 2026-07-05; `system_properties.ss58Format` confirms) and the
/// mediator rejects any other prefix — 42 would 400 every login AND show the user a non-Harlequin address.
pub const SS58_PREFIX: u16 = 1728;

/// SS58-encode a 32-byte public key under [`SS58_PREFIX`]. Prefixes 64..=16383 use the substrate
/// TWO-byte ident form (sp-core `to_ss58check_with_version`): `first = ((ident & 0xFC) >> 2) | 0x40`,
/// `second = (ident >> 8) | ((ident & 0x03) << 6)`, then `base58(ident_bytes ++ pubkey ++
/// blake2b512("SS58PRE" ++ ident_bytes ++ pubkey)[..2])`. Transport format for
/// `GET /api/villa/challenge?mask=<ss58>`; not an identity of its own — the pubkey is.
pub fn ss58_of(pubkey: &[u8; 32]) -> String {
    use blake2::{Blake2b512, Digest as _};
    let ident = SS58_PREFIX & 0b0011_1111_1111_1111;
    let mut data = Vec::with_capacity(36);
    if ident < 64 {
        data.push(ident as u8);
    } else {
        data.push(((ident & 0b0000_0000_1111_1100) as u8) >> 2 | 0b0100_0000);
        data.push((ident >> 8) as u8 | ((ident & 0b0000_0000_0000_0011) as u8) << 6);
    }
    data.extend_from_slice(pubkey);
    let mut h = Blake2b512::new();
    h.update(b"SS58PRE");
    h.update(&data);
    let checksum = h.finalize();
    data.extend_from_slice(&checksum[..2]);
    bs58::encode(data).into_string()
}

// ── Pacts: private contracts signed on chain (design/PACTOS-CONTRATOS-PRIVADOS-2026-10-01.md) ──────────────
// The chain only ever sees a SALTED commitment to the document; the 32-byte salt stays with the parties, so a
// guessed text (a standard template) cannot be confirmed against it. Off-chain pact signatures carry their own
// domain label and the genesis hash: they can never be replayed as a transaction, a login or on another chain.

/// Commitment label: `c = blake2_256(PACT_LABEL ++ salt(32) ++ sha256(document)(32))`.
pub const PACT_LABEL: &[u8; 11] = b"hlq-pact-v1";
/// Off-chain signature label: the mask signs `PACT_SIGN_LABEL ++ genesis_hash(32) ++ c(32)` (80 bytes).
pub const PACT_SIGN_LABEL: &[u8; 16] = b"hlq-pact-sign-v1";
/// Sealed-mode key label: `k = blake2_256(PACT_SEALED_LABEL ++ c ++ sha256(sorted signatures))`.
pub const PACT_SEALED_LABEL: &[u8; 18] = b"hlq-pact-sealed-v1";

fn blake2_256(parts: &[&[u8]]) -> [u8; 32] {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest as _};
    let mut h = Blake2b::<U32>::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// sha256 of a document (what the parties compare before signing).
pub fn document_digest(document: &[u8]) -> [u8; 32] {
    Sha256::digest(document).into()
}

/// The salted commitment the chain stores for a pact.
pub fn pact_commitment(salt: &[u8; 32], document_sha256: &[u8; 32]) -> [u8; 32] {
    blake2_256(&[PACT_LABEL, salt, document_sha256])
}

/// The exact 80 bytes a party signs off chain for a pact.
pub fn pact_sign_message(genesis_hash: &[u8; 32], commitment: &[u8; 32]) -> [u8; 80] {
    let mut m = [0u8; 80];
    m[..16].copy_from_slice(PACT_SIGN_LABEL);
    m[16..48].copy_from_slice(genesis_hash);
    m[48..].copy_from_slice(commitment);
    m
}

/// Sealed-mode key over a commitment and the parties' off-chain signatures (order-independent).
pub fn pact_sealed_key(commitment: &[u8; 32], signatures: &[[u8; 64]]) -> [u8; 32] {
    let mut sigs: Vec<[u8; 64]> = signatures.to_vec();
    sigs.sort();
    let mut h = Sha256::new();
    for s in &sigs {
        h.update(s);
    }
    let sigs_digest: [u8; 32] = h.finalize().into();
    blake2_256(&[PACT_SEALED_LABEL, commitment, &sigs_digest])
}

impl Wallet {
    /// Sign a pact commitment off chain (for sealed mode, or to hand a signed package to the other party).
    pub fn sign_pact(&self, genesis_hash: &[u8; 32], commitment: &[u8; 32]) -> [u8; 64] {
        let ctx = signing_context(SUBSTRATE_CTX);
        self.keypair.sign(ctx.bytes(&pact_sign_message(genesis_hash, commitment))).to_bytes()
    }

    /// Verify a party's off-chain pact signature.
    pub fn verify_pact(
        pubkey: &[u8; 32],
        genesis_hash: &[u8; 32],
        commitment: &[u8; 32],
        sig: &[u8; 64],
    ) -> Result<(), WalletError> {
        let pk = PublicKey::from_bytes(pubkey).map_err(|_| WalletError::BadPublicKey)?;
        let signature = Signature::from_bytes(sig).map_err(|_| WalletError::BadSignature)?;
        pk.verify(signing_context(SUBSTRATE_CTX).bytes(&pact_sign_message(genesis_hash, commitment)), &signature)
            .map_err(|_| WalletError::VerifyFailed)
    }
}

#[cfg(test)]
mod pact_tests {
    use super::*;

    #[test]
    fn commitment_depends_on_salt_and_document() {
        let d1 = document_digest(b"I lend you my cart until spring.");
        let d2 = document_digest(b"I lend you my cart until summer.");
        let (s1, s2) = ([1u8; 32], [2u8; 32]);
        assert_ne!(pact_commitment(&s1, &d1), pact_commitment(&s1, &d2), "another text is another pact");
        assert_ne!(pact_commitment(&s1, &d1), pact_commitment(&s2, &d1), "without the salt the text cannot be confirmed");
        assert_eq!(pact_commitment(&s1, &d1), pact_commitment(&s1, &d1));
    }

    #[test]
    fn pact_signature_roundtrip_and_domains() {
        let a = Wallet::from_entropy(&[31u8; 32]).unwrap();
        let b = Wallet::from_entropy(&[32u8; 32]).unwrap();
        let (g, g2) = ([7u8; 32], [8u8; 32]);
        let c = pact_commitment(&[3u8; 32], &document_digest(b"pact"));
        let sig = a.sign_pact(&g, &c);
        assert!(Wallet::verify_pact(&a.public_key(), &g, &c, &sig).is_ok());
        assert!(Wallet::verify_pact(&b.public_key(), &g, &c, &sig).is_err(), "not B's signature");
        assert!(Wallet::verify_pact(&a.public_key(), &g2, &c, &sig).is_err(), "other chain");
        let other = pact_commitment(&[3u8; 32], &document_digest(b"another pact"));
        assert!(Wallet::verify_pact(&a.public_key(), &g, &other, &sig).is_err(), "other text");
        // a pact signature is not a transaction signature over the same bytes
        assert!(Wallet::verify_tx(&a.public_key(), &c, &sig).is_err());
    }

    #[test]
    fn sealed_key_is_order_independent() {
        let c = [5u8; 32];
        let (x, y) = ([1u8; 64], [2u8; 64]);
        assert_eq!(pact_sealed_key(&c, &[x, y]), pact_sealed_key(&c, &[y, x]));
        assert_ne!(pact_sealed_key(&c, &[x]), pact_sealed_key(&c, &[x, y]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_entropy_is_deterministic() {
        let w1 = Wallet::from_entropy(&[3u8; 32]).unwrap();
        let w2 = Wallet::from_entropy(&[3u8; 32]).unwrap();
        assert_eq!(w1.public_key(), w2.public_key());
        assert_eq!(w1.address(), w2.address());
    }

    #[test]
    fn distinct_entropy_distinct_mask() {
        let a = Wallet::from_entropy(&[1u8; 32]).unwrap();
        let b = Wallet::from_entropy(&[2u8; 32]).unwrap();
        assert_ne!(a.public_key(), b.public_key());
        assert_ne!(a.address(), b.address());
    }

    #[test]
    fn address_shape_matches_canonical_handle() {
        let w = Wallet::from_entropy(&[7u8; 32]).unwrap();
        let a = w.address();
        assert!(a.starts_with("hlq-"));
        assert_eq!(a.len(), 30, "hlq- (4) + 26 base32 chars (128-bit handle)");
        assert!(a[4..].chars().all(|c| "abcdefghijklmnopqrstuvwxyz234567".contains(c)));
        // address_of(pubkey) must equal the wallet's own address (same derivation).
        assert_eq!(address_of(&w.public_key()), a);
    }

    #[test]
    fn sign_verify_roundtrip() {
        let w = Wallet::from_entropy(&[9u8; 32]).unwrap();
        let tx = b"transfer 100 HLQ to hlq-...";
        let sig = w.sign_tx(tx);
        assert_eq!(Wallet::verify_tx(&w.public_key(), tx, &sig), Ok(()));
    }

    #[test]
    fn tampered_tx_fails() {
        let w = Wallet::from_entropy(&[9u8; 32]).unwrap();
        let tx = b"transfer 100 HLQ";
        let sig = w.sign_tx(tx);
        assert_eq!(Wallet::verify_tx(&w.public_key(), b"transfer 999 HLQ", &sig), Err(WalletError::VerifyFailed));
    }

    #[test]
    fn other_key_cannot_verify() {
        let w = Wallet::from_entropy(&[9u8; 32]).unwrap();
        let other = Wallet::from_entropy(&[10u8; 32]).unwrap();
        let tx = b"transfer 100 HLQ";
        let sig = w.sign_tx(tx);
        assert_eq!(Wallet::verify_tx(&other.public_key(), tx, &sig), Err(WalletError::VerifyFailed));
    }

    #[test]
    fn tx_domain_separation() {
        // A signature made under a different context must not verify as a wallet tx.
        let w = Wallet::from_entropy(&[9u8; 32]).unwrap();
        let tx = b"transfer 100 HLQ";
        let other_ctx = signing_context(b"hlq-some-other-domain");
        let sig = w.keypair.sign(other_ctx.bytes(tx)).to_bytes();
        assert_eq!(Wallet::verify_tx(&w.public_key(), tx, &sig), Err(WalletError::VerifyFailed));
    }

    // ---- #837 governance extrinsic signing (ceremony of the Key) ----
    #[test]
    fn sign_extrinsic_roundtrip_both_size_regimes() {
        let w = Wallet::from_entropy(&[7u8; 32]).unwrap();
        // ≤256B (propose/approve): signed as-is.
        let small = vec![0xabu8; 194];
        let s1 = w.sign_extrinsic(&small);
        assert!(Wallet::verify_extrinsic(&w.public_key(), &small, &s1).is_ok());
        // >256B (apply_upgrade carries the WASM): blake2_256 path.
        let big = vec![0xcdu8; 300_000];
        let s2 = w.sign_extrinsic(&big);
        assert!(Wallet::verify_extrinsic(&w.public_key(), &big, &s2).is_ok());
        // A >256 signature verified against the RAW (non-hashed) bytes under a naive verifier must differ:
        // proves the hash path is actually taken (sig over blake2, not over the 300k blob directly).
        let ctx = signing_context(SUBSTRATE_CTX);
        let sig_obj = Signature::from_bytes(&s2).unwrap();
        let pk = PublicKey::from_bytes(&w.public_key()).unwrap();
        assert!(pk.verify(ctx.bytes(&big), &sig_obj).is_err(), "big must be signed via blake2, not raw");
        // Wrong mask never verifies.
        let w2 = Wallet::from_entropy(&[8u8; 32]).unwrap();
        assert!(Wallet::verify_extrinsic(&w2.public_key(), &small, &s1).is_err());
    }

    // ---- #637 villa login (contract frozen 2026-07-21, REF-VECTORS-637-837.md) ----

    /// //Alice pubkey (substrate dev account) — anchor for the cross-implementation vectors.
    const ALICE_PUB: [u8; 32] = hex_lit("d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d");
    const REF_GENESIS: [u8; 32] = hex_lit("a1ab5e0b11ccadccb1f8f90692f4c588190b421532ab0cee9f56ad615da1fed6");

    const fn hex_val(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("bad hex"),
        }
    }
    const fn hex_lit<const N: usize>(s: &str) -> [u8; N] {
        let b = s.as_bytes();
        assert!(b.len() == N * 2);
        let mut out = [0u8; N];
        let mut i = 0;
        while i < N {
            out[i] = hex_val(b[i * 2]) * 16 + hex_val(b[i * 2 + 1]);
            i += 1;
        }
        out
    }

    #[test]
    fn villa_login_message_matches_reference_vector() {
        // REF vector: nonce = 0x11*32, issued_at = 1753000000 → the exact 90-byte msg_hex.
        let msg = villa_login_message(&[0x11u8; 32], &REF_GENESIS, 1_753_000_000);
        let expected: [u8; 90] = hex_lit(
            "686c712d76696c6c612d6c6f67696e2d76311111111111111111111111111111111111111111111111111111111111111111a1ab5e0b11ccadccb1f8f90692f4c588190b421532ab0cee9f56ad615da1fed640a87c6800000000",
        );
        assert_eq!(msg, expected, "villa login message must be byte-identical to REF-VECTORS");
    }

    #[test]
    fn villa_login_reference_signature_verifies() {
        // Signature produced by substrate-interface (//Alice) over the reference message: an independent
        // implementation triangulation. schnorrkel MUST accept it.
        let sig: [u8; 64] = hex_lit(
            "58b156eeee5fe72d1040283abf6b74cd8b4d8c030c0c814f9c3b57a6054bb349684fd19486fc6b25df680850ccbfd6f6184332c89aa95bb1503ad1d1370b908b",
        );
        assert_eq!(
            Wallet::verify_villa_login(&ALICE_PUB, &[0x11u8; 32], &REF_GENESIS, 1_753_000_000, &sig),
            Ok(())
        );
        // Tamper any field → reject (fail-closed on nonce, genesis, issued_at).
        assert!(Wallet::verify_villa_login(&ALICE_PUB, &[0x12u8; 32], &REF_GENESIS, 1_753_000_000, &sig).is_err());
        assert!(Wallet::verify_villa_login(&ALICE_PUB, &[0x11u8; 32], &[0u8; 32], 1_753_000_000, &sig).is_err());
        assert!(Wallet::verify_villa_login(&ALICE_PUB, &[0x11u8; 32], &REF_GENESIS, 1_753_000_001, &sig).is_err());
    }

    #[test]
    fn villa_login_roundtrip_and_cross_domain() {
        let w = Wallet::from_entropy(&[11u8; 32]).unwrap();
        let nonce = [0xABu8; 32];
        let sig = w.sign_villa_login(&nonce, &REF_GENESIS, 42);
        assert_eq!(Wallet::verify_villa_login(&w.public_key(), &nonce, &REF_GENESIS, 42, &sig), Ok(()));
        // Load-bearing: a login proof must not verify as a transaction over different bytes. (Both
        // domains share SUBSTRATE_CTX by contract — separation lives in the message LAYOUT: a login
        // message always starts with the 18-byte label, which can never be a valid SCALE extrinsic
        // signing-payload prefix on this chain.)
        assert_eq!(Wallet::verify_tx(&w.public_key(), b"some extrinsic payload", &sig),
            Err(WalletError::VerifyFailed),
            "a login signature MUST NOT verify as a transaction");
    }

    // ---- #837 node↔mask binding PoP ----

    #[test]
    fn ss58_matches_chain_prefix_1728_references() {
        // Ground truth generated with substrate-interface ss58_encode(pub, 1728) (2026-07-21).
        assert_eq!(ss58_of(&ALICE_PUB), "rLa85tAjefPb7WzwzfvGZwBAuRzBHtYLg1GhM1A4HoLdZ24KH");
        let seq: [u8; 32] = core::array::from_fn(|i| i as u8);
        assert_eq!(ss58_of(&seq), "rLVKr7dRCCXa1bgHvyFD4wih7vpsNQAz4B6LrF1jST2n8gpeo");
    }

    #[test]
    fn node_bind_message_matches_reference_vector() {
        let msg = node_bind_message(&ALICE_PUB);
        let expected: [u8; 48] = hex_lit(
            "686c712d6e6f64652d62696e642d7631d43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d",
        );
        assert_eq!(msg, expected, "bind PoP message must be byte-identical to REF-VECTORS / the devnet driver");
    }

    #[test]
    fn node_pop_roundtrip_and_wrong_mask_rejects() {
        // Simulate the node's session key with a wallet keypair (same sr25519 + substrate ctx).
        let session = Wallet::from_entropy(&[21u8; 32]).unwrap();
        let mask = Wallet::from_entropy(&[22u8; 32]).unwrap();
        let other_mask = Wallet::from_entropy(&[23u8; 32]).unwrap();
        let pop = session.sign_tx(&node_bind_message(&mask.public_key()));
        assert_eq!(verify_node_pop(&session.public_key(), &mask.public_key(), &pop), Ok(()));
        // PoP bound to mask A must not validate for mask B (steals-someone's-node attack).
        assert_eq!(
            verify_node_pop(&session.public_key(), &other_mask.public_key(), &pop),
            Err(WalletError::VerifyFailed)
        );
        // And a PoP signed by a key you don't possess must reject.
        assert_eq!(
            verify_node_pop(&other_mask.public_key(), &mask.public_key(), &pop),
            Err(WalletError::VerifyFailed)
        );
    }

    #[test]
    fn malformed_blobs_fail_closed() {
        let w = Wallet::from_entropy(&[9u8; 32]).unwrap();
        let tx = b"x";
        let sig = w.sign_tx(tx);
        assert_eq!(Wallet::verify_tx(&[0xFFu8; 32], tx, &sig), Err(WalletError::BadPublicKey));
        assert!(matches!(
            Wallet::verify_tx(&w.public_key(), tx, &[0xFFu8; 64]),
            Err(WalletError::BadSignature) | Err(WalletError::VerifyFailed)
        ));
    }
}
