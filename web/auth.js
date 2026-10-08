/* Harlequin — client auth flow (no-PII).  #611 / #617
 *
 * SCAFFOLDING — live PREVIEW, inert until launch. There is no backend pre-launch, and the cryptographic
 * core (key generation / derivation / signing / handle derivation) is designed and SECURITY-SWEPT JOINTLY
 * with the operator before it ships. This module cables the NON-crypto flow (forms, fetch, session) to the operator's
 * auth contract so the surface is ready; every crypto operation is delegated to `HlqCrypto` (see §0), which
 * stays a stub here until the joint design + sweep fills it with a VETTED library (WebAuthn/WebCrypto;
 * never homegrown — see the internal crypto design notes).
 *
 * Contract (the operator, #617; §8 converged 2026-06-23):
 *   identity   = sr25519 key SEED-DERIVED (BIP39 phrase -> chain-identity root key), client-side. The mask
 *                is the identity pubkey. Hard-derived SUBKEYS per domain: auth / wallet / chat. Login signs
 *                with the AUTH subkey, never the spending key. Passkey/WebAuthn (if used) is only a LOCAL
 *                unlock of the encrypted seed store — NOT the identity (device-bound, not seed-recoverable).
 *   handle     = "hlq-" + base32_rfc4648_lower_nopad(sha256(pubkey)[..16])  (26 chars; chain-canonical #650)
 *   register   → server gets ONLY { pubkey, handle, profile_blob? (encrypted) }. ZERO PII fields.
 *                seed phrase (BIP39) = the ONLY recovery, shown once, never sent.
 *   login      → POST /auth/challenge {pubkey} -> { nonce } (32B, TTL 60s, single-use; always returns a
 *                nonce so the endpoint does not enumerate accounts)
 *                client signs  "harlequin-login:" || nonce
 *                POST /auth/verify {pubkey, nonce, sig} -> sets session cookie (HttpOnly; Secure;
 *                SameSite=Strict; JWT TTL 15min, rotating refresh)
 *   recovery   = seed phrase only, client-side. No server reset. Lose the phrase = lose the account.
 *   server     = dumb portal: stores pubkey/handle/encrypted profile_blob/ephemeral nonce/session.
 *                NEVER the private key, the phrase, PII, IP or UA.
 */
(function (global) {
  "use strict";

  // The login message-domain PREFIX (scheme B, the operator 2026-07-01). Signing uses the STANDARD schnorrkel
  // context b"substrate" (same as the chain verifies extrinsics and finality votes), so domain separation is
  // by the MESSAGE, not a custom context: wallet_wasm.signLogin signs signing_context(b"substrate").bytes(
  // b"harlequin-login:v1:" || nonce). The prefix (19 ASCII bytes, incl. the trailing colon) makes the login
  // message unable to be a valid SCALE extrinsic, so a login signature can never cross to a tx. The prefix is
  // PREPENDED INSIDE the wasm (never in JS); the server verifies the same message with substrate-interface
  // (native b"substrate"). Kept as a const only to document the shared client↔server contract.
  var LOGIN_MSG_PREFIX = "harlequin-login:v1:";

  // Decode a hex string (optionally 0x-prefixed) to raw bytes. The server nonce travels as hex and is signed
  // as RAW bytes on both sides (client wasm + server verify), so it must be decoded before signing.
  function hexToBytes(hex) {
    var h = hex.indexOf("0x") === 0 ? hex.slice(2) : hex;
    if (h.length % 2) throw new Error("bad nonce hex");
    var out = new Uint8Array(h.length / 2);
    for (var i = 0; i < out.length; i++) out[i] = parseInt(h.substr(i * 2, 2), 16);
    return out;
  }
  var API = ""; // same-origin; the backend is not deployed pre-launch.

  // --- §0. Crypto interface (JOINT design + the operator sweep; vetted lib only) ---------------------------
  // Intentionally unimplemented here. The real impl (sr25519 seed-derived root + hard-derived domain
  // subkeys + BIP39 + blake2b/base32 handle; optional WebAuthn local store unlock) is reviewed and
  // security-swept with the operator before wiring. Anything calling these before that is a programming error,
  // hence the explicit throw — never a silent fallback that ships weak crypto.
  var HlqCrypto = global.HlqCrypto || {
    _stub: function (name) {
      throw new Error("HlqCrypto." + name + " not implemented — pending joint crypto design + the operator sweep");
    },
    // → { pubkey, handle, seedPhrase }   sr25519 identity root, generated on-device; subkeys hard-derived
    generateMask: function () { return this._stub("generateMask"); },
    // (seedPhrase) → { pubkey, handle }   deterministic recovery of the identity root, on-device
    deriveMaskFromSeed: function (seedPhrase) { return this._stub("deriveMaskFromSeed"); },
    // (nonceBytes: Uint8Array) → signature (hex, 64B)   signs signing_context(b"substrate").bytes(
    // b"harlequin-login:v1:"||nonce) INSIDE the vetted wasm (scheme B: standard context + message-domain;
    // the prefix is prepended in signLogin, never in JS; caller passes the RAW nonce bytes)
    signAuth: function (nonceBytes) { return this._stub("signAuth"); }
  };

  function postJSON(path, body) {
    return fetch(API + path, {
      method: "POST",
      headers: { "content-type": "application/json" },
      credentials: "same-origin",            // session cookie is HttpOnly; the client never reads it
      cache: "no-store",
      body: JSON.stringify(body)
    }).then(function (r) {
      if (!r.ok) throw new Error("http " + r.status + " " + path);
      return r.status === 204 ? null : r.json();
    });
  }

  // --- Registration ---------------------------------------------------------------------------------
  // Forges the mask on-device, registers ONLY public data, and returns the seed phrase to the caller so the
  // UI can show it once with a hard "this is your only recovery" warning. The phrase is NEVER sent.
  function register(opts) {
    opts = opts || {};
    var mask = HlqCrypto.generateMask();          // { pubkey, handle, seedPhrase } — all on-device
    var payload = { pubkey: mask.pubkey, handle: mask.handle };
    if (opts.profileBlob != null) payload.profile_blob = opts.profileBlob; // already client-encrypted; optional
    return postJSON("/auth/register", payload).then(function () {
      return { pubkey: mask.pubkey, handle: mask.handle, seedPhrase: mask.seedPhrase };
    });
  }

  // --- Login (challenge → sign → verify) ------------------------------------------------------------
  // Passwordless. The challenge endpoint always returns a nonce (no account enumeration); a wrong/unknown
  // pubkey simply fails at verify. The session is established by the server's HttpOnly cookie on success.
  function login(pubkey) {
    return postJSON("/auth/challenge", { pubkey: pubkey })
      .then(function (res) {
        var nonce = res && res.nonce;
        if (!nonce) throw new Error("no nonce");
        // Scheme B: the server nonce arrives as hex; decode it to RAW bytes and pass those to signAuth.
        // wallet_wasm.signLogin PREPENDS b"harlequin-login:v1:" and signs signing_context(b"substrate").bytes(
        // PREFIX||nonce) — standard context, message-domain separation — so the sig can't cross to a tx and
        // verifies natively server-side (substrate-interface). No prefix is concatenated in JS. Returns 64B hex.
        var sig = HlqCrypto.signAuth(hexToBytes(nonce));
        return postJSON("/auth/verify", { pubkey: pubkey, nonce: nonce, sig: sig });
      })
      .then(function () { return { ok: true, pubkey: pubkey }; });
  }

  // Recover the mask from the seed phrase (on-device), then log in with it. The phrase never leaves here.
  function loginWithSeed(seedPhrase) {
    var mask = HlqCrypto.deriveMaskFromSeed(seedPhrase);
    return login(mask.pubkey).then(function (s) { s.handle = mask.handle; return s; });
  }

  // --- Session ---------------------------------------------------------------------------------------
  // The cookie is HttpOnly, so the client cannot read it; it probes /auth/me to learn its own state.
  function me() {
    return fetch(API + "/auth/me", { credentials: "same-origin", cache: "no-store" })
      .then(function (r) { return r.ok ? r.json() : null; })
      .catch(function () { return null; });
  }
  function logout() {
    return postJSON("/auth/logout", {}).then(function () { return true; }).catch(function () { return false; });
  }

  global.HlqAuth = {
    LOGIN_MSG_PREFIX: LOGIN_MSG_PREFIX,
    setCrypto: function (impl) { HlqCrypto = impl; },  // joint crypto module injects the vetted impl here
    register: register,
    login: login,
    loginWithSeed: loginWithSeed,
    me: me,
    logout: logout
  };
})(typeof window !== "undefined" ? window : this);
