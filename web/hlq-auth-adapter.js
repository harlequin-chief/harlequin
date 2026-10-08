/* Harlequin — HlqAuth ⇄ HlqWallet adapter ("the mask-crypto binding").  Citizen rite, Acts II & IV.
 *
 * WHY THIS FILE. `auth.js` (HlqAuth) ships the no-PII login FLOW with its `HlqCrypto` left as an explicit
 * stub — to be filled by VETTED crypto after joint design + the operator sweep (#611/#617). `wallet-core.js`
 * (HlqWallet) IS that vetted crypto: one sr25519 mask over the operator's `wallet_wasm`. This module is the glue:
 * it implements `HlqCrypto` on top of `HlqWallet` and injects it via `HlqAuth.setCrypto`.
 *
 * MODEL — single-key + domain separation (scheme B, the operator OPSEC 2026-07-01):
 *   identity = ONE sr25519 key. mask pubkey == on-chain AccountId; handle = "hlq-"+base32(sha256(pubkey)[..16])
 *   (#650, what wallet.address() returns) — the display id, NOT SS58.
 *   Both signatures use the STANDARD schnorrkel context b"substrate" (what the chain verifies extrinsics and
 *   finality votes with); domain separation is by the MESSAGE, not a custom context:
 *     login  signs  signing_context(b"substrate").bytes(b"harlequin-login:v1:" || nonce)   via signLogin
 *                   (the 19-byte prefix is prepended INSIDE the wasm — never in JS — so the login message
 *                    can't be a valid SCALE extrinsic, and a login sig never crosses to a tx).
 *     spend  signs  signing_context(b"substrate").bytes(extrinsic_payload)                 via signTx.
 *   RESIDUAL (v1): login key == spend key → isolation is by MESSAGE, not by key. Leaking the login key leaks
 *   the spending key. Acceptable for v1/mock; hard-derived per-domain subkeys (#617) are the v2 hardening
 *   (#755), a declared BLOCKER of funds-flow-at-scale. Nothing here ever emits a seed/key/passphrase.
 *
 * CONTRACT SEAM (flagged to the operator): because the login domain tag lives in the wasm, `HlqCrypto.signAuth`
 * receives the RAW NONCE (not "prefix"||nonce). auth.js `login()` must pass the server nonce straight to
 * signAuth; the old `LOGIN_PREFIX + nonce` concatenation moves into wallet_wasm.signLogin. Until the operator
 * publishes signLogin, signAuth throws a clear pending-error rather than shipping a wrong-domain signature.
 *
 * Load order: wallet-core.js (module) + auth.js, then this. Binds on `hlqwallet-ready` (or immediately if
 * HlqWallet is already up). Fires `hlqauth-ready` once HlqCrypto is injected, for the rite UI to enable.
 */
(function () {
  "use strict";

  function bind() {
    var W = window.HlqWallet, A = window.HlqAuth;
    if (!W || !A || typeof A.setCrypto !== "function") return false;

    // HlqCrypto implemented over the vetted HlqWallet. Every operation is on-device; the server sees only
    // the public key, the handle, and (at login) a signature over a server-issued nonce.
    var adapter = {
      // Act II — forge a brand-new mask. `passphrase` seals the at-rest keystore (the LOCAL unlock secret,
      // NOT the identity). Returns the pubkey (== SS58 == on-chain identity) and the mnemonic to show ONCE.
      generateMask: function (passphrase) {
        return W.create(passphrase || "").then(function (r) {
          return { pubkey: W.pubkeyHex(), handle: r.handle, seedPhrase: r.mnemonic };
        });
      },

      // Recover the identity from its seed phrase and re-seal at rest under `passphrase`. On-device only;
      // the phrase never leaves here. (auth.js calls this with just the phrase — passphrase optional.)
      deriveMaskFromSeed: function (mnemonic, passphrase) {
        return W.restore(mnemonic, passphrase || "").then(function (r) {
          return { pubkey: W.pubkeyHex(), handle: r.handle };
        });
      },

      // Act IV — sign the login challenge. `nonceBytes` is the RAW nonce (Uint8Array; auth.js hex-decodes the
      // server nonce before calling). signLogin prepends b"harlequin-login:v1:" and signs under b"substrate",
      // so the signed bytes are exactly what the server verifies (substrate-interface). Needs an UNLOCKED wallet.
      signAuth: function (nonceBytes) {
        if (typeof W.signLogin !== "function") {
          throw new Error("signAuth pending: HlqWallet.signLogin facade not present yet (the operator wasm rebuild)");
        }
        return W.signLogin(nonceBytes);
      }
    };

    A.setCrypto(adapter);
    try { window.dispatchEvent(new Event("hlqauth-ready")); } catch (e) {}
    return true;
  }

  if (!bind()) {
    // HlqWallet initialises its wasm asynchronously; bind when it announces itself.
    window.addEventListener("hlqwallet-ready", bind, { once: true });
  }
})();
