//! wallet-core → WASM bindings (#664 / M3 wallet-UI). Thin `wasm-bindgen` wrapper exposing wallet-core's
//! self-custody primitives to the browser, CLIENT-SIDE. Keys are generated and used 100% in the browser
//! (privacy arch #617 adversary A1: a server never holds a key). This crate adds NO cryptography — it
//! delegates to the audited wallet-core and only marshals types across the JS boundary.
//!
//! ## RNG discipline (load-bearing)
//! wallet-core is RNG-FREE; so is this wrapper. The browser MUST supply all randomness explicitly via
//! `crypto.getRandomValues` and pass it in: 32-byte `entropy` for a new wallet, 16-byte `salt` + 24-byte
//! `nonce` for at-rest sealing. This keeps the entropy source inspectable in the devtools (no hidden RNG).
//!
//! ## Memory
//! `Wallet` holds the live sr25519 keypair; call `free()` (wasm-bindgen) on lock/close to drop it.
//! Decrypted entropy returned by `openKeystore` is a `Uint8Array` the UI must zero after deriving the
//! wallet (JS side can't be force-zeroized as hard as Rust, so minimize its lifetime).

use wasm_bindgen::prelude::*;
use wallet_core::atrest::{self, EncryptedKeystore};
use wallet_core::mnemonic;
use wallet_core::{address_of as core_address_of, Wallet as CoreWallet};

fn arr32(b: &[u8], what: &str) -> Result<[u8; 32], JsError> {
    b.try_into().map_err(|_| JsError::new(&format!("{what} must be 32 bytes")))
}

/// A self-custody wallet = one sr25519 key that is BOTH the on-chain mask and the wallet (address == handle).
#[wasm_bindgen]
pub struct Wallet {
    inner: CoreWallet,
}

#[wasm_bindgen]
impl Wallet {
    /// Build a wallet from 32 bytes of host entropy (browser: `crypto.getRandomValues(new Uint8Array(32))`).
    #[wasm_bindgen(js_name = fromEntropy)]
    pub fn from_entropy(entropy: &[u8]) -> Result<Wallet, JsError> {
        let e = arr32(entropy, "entropy")?;
        let inner = CoreWallet::from_entropy(&e).map_err(|_| JsError::new("bad entropy"))?;
        Ok(Wallet { inner })
    }

    /// The 32-byte sr25519 public key.
    #[wasm_bindgen(js_name = publicKey)]
    pub fn public_key(&self) -> Vec<u8> {
        self.inner.public_key().to_vec()
    }

    /// The canonical address == mask handle ("hlq-…", base32 of sha256(pub)[..16]). Same as the chain (#650).
    pub fn address(&self) -> String {
        self.inner.address()
    }

    /// Sign a transaction (domain `hlq-wallet-tx-v1`). Returns the 64-byte sr25519 signature.
    #[wasm_bindgen(js_name = signTx)]
    pub fn sign_tx(&self, tx_bytes: &[u8]) -> Vec<u8> {
        self.inner.sign_tx(tx_bytes).to_vec()
    }

    /// Sign an extrinsic's SignedPayload for on-chain governance (#837 ceremony: propose/approve/
    /// apply_upgrade). SUBSTRATE-STANDARD extrinsic signature (no domain prefix), over exactly the bytes
    /// the runtime verifies. The host builds the SignedPayload `(call || extra || additional)`; this
    /// applies the >256→blake2_256 rule and signs sr25519 with the mask. Seed stays local; returns 64B.
    #[wasm_bindgen(js_name = signExtrinsicPayload)]
    pub fn sign_extrinsic(&self, signed_payload: &[u8]) -> Vec<u8> {
        self.inner.sign_extrinsic(signed_payload).to_vec()
    }

    /// Sign a villa login challenge (#637, contract 2026-07-21). Pass the RAW fields from
    /// `GET /api/villa/challenge` — the 90-byte message (`hlq-villa-login-v1` ++ nonce ++ genesis ++
    /// issued_at LE) is assembled inside; do NOT build it in JS. Returns the 64-byte sr25519 signature.
    /// A login signature can never be replayed as a transaction, on another chain, or past its window.
    #[wasm_bindgen(js_name = signVillaLogin)]
    pub fn sign_villa_login(
        &self,
        nonce: &[u8],
        genesis_hash: &[u8],
        issued_at: u64,
    ) -> Result<Vec<u8>, JsError> {
        Ok(self
            .inner
            .sign_villa_login(&arr32(nonce, "nonce")?, &arr32(genesis_hash, "genesis_hash")?, issued_at)
            .to_vec())
    }

    /// Sign a pact commitment off chain (`hlq-pact-sign-v1` ++ genesis ++ commitment). 64-byte signature.
    #[wasm_bindgen(js_name = signPact)]
    pub fn sign_pact(&self, genesis_hash: &[u8], commitment: &[u8]) -> Result<Vec<u8>, JsError> {
        Ok(self.inner.sign_pact(&arr32(genesis_hash, "genesis_hash")?, &arr32(commitment, "commitment")?).to_vec())
    }
}

/// sha256 of a document — what the parties compare before signing a pact.
#[wasm_bindgen(js_name = documentDigest)]
pub fn document_digest(document: &[u8]) -> Vec<u8> {
    wallet_core::document_digest(document).to_vec()
}

/// The salted pact commitment the chain stores: blake2_256(`hlq-pact-v1` ++ salt ++ sha256(document)).
#[wasm_bindgen(js_name = pactCommitment)]
pub fn pact_commitment(salt: &[u8], document_sha256: &[u8]) -> Result<Vec<u8>, JsError> {
    Ok(wallet_core::pact_commitment(&arr32(salt, "salt")?, &arr32(document_sha256, "document_sha256")?).to_vec())
}

/// Verify a party's off-chain pact signature.
#[wasm_bindgen(js_name = verifyPact)]
pub fn verify_pact(pubkey: &[u8], genesis_hash: &[u8], commitment: &[u8], sig: &[u8]) -> bool {
    let (Ok(pk), Ok(g), Ok(c)) = (arr32(pubkey, "pubkey"), arr32(genesis_hash, "genesis_hash"), arr32(commitment, "commitment")) else {
        return false;
    };
    let Ok(sig): Result<[u8; 64], _> = sig.try_into() else { return false };
    CoreWallet::verify_pact(&pk, &g, &c, &sig).is_ok()
}

/// Sealed-mode key over a commitment and the parties' 64-byte signatures (concatenated), order-independent.
#[wasm_bindgen(js_name = pactSealedKey)]
pub fn pact_sealed_key(commitment: &[u8], signatures: &[u8]) -> Result<Vec<u8>, JsError> {
    if signatures.is_empty() || signatures.len() % 64 != 0 {
        return Err(JsError::new("signatures must be a non-empty concatenation of 64-byte signatures"));
    }
    let sigs: Vec<[u8; 64]> = signatures.chunks(64).map(|c| c.try_into().expect("64")).collect();
    Ok(wallet_core::pact_sealed_key(&arr32(commitment, "commitment")?, &sigs).to_vec())
}

/// Verify a villa login signature (the mediator does the real check; exposed for parity/tests).
#[wasm_bindgen(js_name = verifyVillaLogin)]
pub fn verify_villa_login(
    pubkey: &[u8],
    nonce: &[u8],
    genesis_hash: &[u8],
    issued_at: u64,
    sig: &[u8],
) -> bool {
    let (Ok(pk), Ok(n), Ok(g)) = (
        arr32(pubkey, "pubkey"),
        arr32(nonce, "nonce"),
        arr32(genesis_hash, "genesis_hash"),
    ) else {
        return false;
    };
    let s: [u8; 64] = match sig.try_into() {
        Ok(x) => x,
        Err(_) => return false,
    };
    CoreWallet::verify_villa_login(&pk, &n, &g, issued_at, &s).is_ok()
}

/// Build the node↔mask binding PoP message (#837): `hlq-node-bind-v1` ++ mask_account_id = 48 bytes.
/// Hand this to the node so its session key signs it (`pop_sig`).
#[wasm_bindgen(js_name = nodeBindMessage)]
pub fn node_bind_message(mask_account_id: &[u8]) -> Result<Vec<u8>, JsError> {
    Ok(wallet_core::node_bind_message(&arr32(mask_account_id, "mask_account_id")?).to_vec())
}

/// Verify a node's binding PoP BEFORE the mask signs `set_vote_key`. Fail-closed, mirrors the pallet.
#[wasm_bindgen(js_name = verifyNodePop)]
pub fn verify_node_pop(sk_pub: &[u8], mask_account_id: &[u8], pop_sig: &[u8]) -> bool {
    let (Ok(sk), Ok(acc)) = (arr32(sk_pub, "sk_pub"), arr32(mask_account_id, "mask_account_id")) else {
        return false;
    };
    let s: [u8; 64] = match pop_sig.try_into() {
        Ok(x) => x,
        Err(_) => return false,
    };
    wallet_core::verify_node_pop(&sk, &acc, &s).is_ok()
}

/// Derive the canonical address/handle from any 32-byte public key (display a peer's address, no wallet).
#[wasm_bindgen(js_name = addressOf)]
pub fn address_of(pubkey: &[u8]) -> Result<String, JsError> {
    Ok(core_address_of(&arr32(pubkey, "pubkey")?))
}

/// SS58-encode a 32-byte public key (Harlequin prefix 1728) — the wire format for `/api/villa/challenge?mask=`.
#[wasm_bindgen(js_name = ss58Of)]
pub fn ss58_of(pubkey: &[u8]) -> Result<String, JsError> {
    Ok(wallet_core::ss58_of(&arr32(pubkey, "pubkey")?))
}

/// Verify a transaction signature. Returns `true` iff valid (never throws on length — returns false).
#[wasm_bindgen(js_name = verifyTx)]
pub fn verify_tx(pubkey: &[u8], tx_bytes: &[u8], sig: &[u8]) -> bool {
    let pk: [u8; 32] = match pubkey.try_into() {
        Ok(x) => x,
        Err(_) => return false,
    };
    let s: [u8; 64] = match sig.try_into() {
        Ok(x) => x,
        Err(_) => return false,
    };
    CoreWallet::verify_tx(&pk, tx_bytes, &s).is_ok()
}

/// Verify an extrinsic signature (parity/tests; the CHAIN is the real verifier). Same >256→blake2_256
/// rule as `signExtrinsic`. Returns `true` iff valid.
#[wasm_bindgen(js_name = verifyExtrinsic)]
pub fn verify_extrinsic(pubkey: &[u8], signed_payload: &[u8], sig: &[u8]) -> bool {
    let pk: [u8; 32] = match pubkey.try_into() {
        Ok(x) => x,
        Err(_) => return false,
    };
    let s: [u8; 64] = match sig.try_into() {
        Ok(x) => x,
        Err(_) => return false,
    };
    CoreWallet::verify_extrinsic(&pk, signed_payload, &s).is_ok()
}

/// Seal the 32-byte entropy under a passphrase (Argon2id → XChaCha20-Poly1305). `salt` (16B) and `nonce`
/// (24B) come from the host RNG. Returns an 88-byte self-describing blob `salt(16)||nonce(24)||ct(48)` to
/// persist in IndexedDB. The plaintext entropy is never in the blob.
#[wasm_bindgen(js_name = sealKeystore)]
pub fn seal_keystore(entropy: &[u8], passphrase: &[u8], salt: &[u8], nonce: &[u8]) -> Result<Vec<u8>, JsError> {
    let e = arr32(entropy, "entropy")?;
    let s: [u8; 16] = salt.try_into().map_err(|_| JsError::new("salt must be 16 bytes"))?;
    let n: [u8; 24] = nonce.try_into().map_err(|_| JsError::new("nonce must be 24 bytes"))?;
    let ks = atrest::seal(&e, passphrase, &s, &n).map_err(|_| JsError::new("seal failed"))?;
    let mut out = Vec::with_capacity(16 + 24 + ks.ciphertext.len());
    out.extend_from_slice(&ks.salt);
    out.extend_from_slice(&ks.nonce);
    out.extend_from_slice(&ks.ciphertext);
    Ok(out)
}

/// Open an 88-byte keystore blob with a passphrase. Wrong passphrase / tampered blob → error (fail closed).
/// Returns the 32-byte entropy (the UI derives the wallet then should drop it promptly).
#[wasm_bindgen(js_name = openKeystore)]
pub fn open_keystore(blob: &[u8], passphrase: &[u8]) -> Result<Vec<u8>, JsError> {
    if blob.len() != 88 {
        return Err(JsError::new("keystore blob must be 88 bytes (salt16||nonce24||ct48)"));
    }
    let ks = EncryptedKeystore {
        salt: blob[0..16].try_into().unwrap(),
        nonce: blob[16..40].try_into().unwrap(),
        ciphertext: blob[40..88].to_vec(),
    };
    let entropy = atrest::open(&ks, passphrase).map_err(|_| JsError::new("wrong passphrase or tampered keystore"))?;
    Ok(entropy.to_vec())
}

/// 32-byte entropy → 24-word BIP-39 mnemonic (shown ONCE at onboarding for the user to custody).
#[wasm_bindgen(js_name = entropyToPhrase)]
pub fn entropy_to_phrase(entropy: &[u8]) -> Result<String, JsError> {
    Ok(mnemonic::entropy_to_phrase(&arr32(entropy, "entropy")?))
}

/// 24-word BIP-39 mnemonic → 32-byte entropy (recovery). Invalid phrase/checksum → error.
#[wasm_bindgen(js_name = phraseToEntropy)]
pub fn phrase_to_entropy(phrase: &str) -> Result<Vec<u8>, JsError> {
    Ok(mnemonic::phrase_to_entropy(phrase)
        .map_err(|_| JsError::new("invalid mnemonic"))?
        .to_vec())
}
