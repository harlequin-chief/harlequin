/* Harlequin — wallet binding ("HlqWallet").  M3 #664/#666  [ES MODULE]
 *
 * Binds the on-device wallet facade to the VETTED wallet-core WASM (pkg/wallet_wasm.js, the operator). Keys live
 * on the device only (privacy arch #617, adversary A1); nothing here ever sends a private key, a passphrase
 * or a seed. The wallet IS the mask: one sr25519 key, address == handle == on-chain identity (#650).
 *
 * Rules (the operator 2026-06-26): entropy/salt/nonce come from the HOST RNG (crypto.getRandomValues) — wallet-core
 * is RNG-free and inspectable; `Wallet.free()` releases the key on lock; the entropy from openKeystore is
 * dropped (zeroed) right after deriving the wallet. At-rest = 88-byte keystore blob in IndexedDB.
 *
 * Loaded as a module (deferred); on init it sets `window.HlqWallet` and fires `hlqwallet-ready` so the
 * classic render layer (wallet.js) can bind + re-render.
 *
 * Follow-up with the operator (decision-2 tail): live balances = pallet-tokens read (storage-key or a tokens RPC)
 * and send = extrinsic assembly + author_submitExtrinsic. Until wired: honest zero balances, send disabled.
 */
import init, {
  Wallet, addressOf, verifyTx, sealKeystore, openKeystore, entropyToPhrase, phraseToEntropy, ss58Of
} from "./pkg/wallet_wasm.js";

const DB = "hlq-wallet", STORE = "keystore", KEY = "default", DB_VERSION = 2;

function idbOpen() {
  return new Promise((res, rej) => {
    // v2 (2026-09-25): the villa and the rite used to open this database in v1 WITHOUT creating the store;
    // if the villa got there first, the database was left in v1 empty and onupgradeneeded here no longer
    // fired → the rite "sealed" without saving anything. Every door now opens at DB_VERSION and creates the
    // store if missing (repairs the broken ones).
    const r = indexedDB.open(DB, DB_VERSION);
    r.onupgradeneeded = () => { if (!r.result.objectStoreNames.contains(STORE)) r.result.createObjectStore(STORE); };
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
}
async function idbGet() {
  const db = await idbOpen();
  return new Promise((res, rej) => {
    const t = db.transaction(STORE, "readonly").objectStore(STORE).get(KEY);
    t.onsuccess = () => res(t.result || null);
    t.onerror = () => rej(t.error);
  });
}
async function idbPut(blob) {
  const db = await idbOpen();
  return new Promise((res, rej) => {
    const t = db.transaction(STORE, "readwrite").objectStore(STORE).put(blob, KEY);
    t.onsuccess = () => res();
    t.onerror = () => rej(t.error);
  });
}

const enc = (s) => new TextEncoder().encode(s);
const zero = (u8) => { if (u8 && u8.fill) u8.fill(0); };

let wallet = null;          // live Wallet (holds the key) while unlocked
// The mnemonic is NEVER retained in module memory — create() returns it for the caller to show ONCE.

const Hlq = {
  SIGN_DOMAIN: "hlq-wallet-tx-v1",

  async hasKeystore() { return !!(await idbGet()); },
  isUnlocked() { return !!wallet; },
  address() { return wallet ? wallet.address() : null; },
  addressOf(pubkey) { return addressOf(pubkey); },          // a peer's address, no wallet
  ss58Of(pubkey) { return ss58Of(pubkey); },                // a peer's SS58 address (1728), no wallet — 7-oct
  // The wallet's own 32-byte sr25519 public key as 0x-hex (== on-chain AccountId, #650). PUBLIC data, no
  // secret: it is what the mediated reads key on and what a peer would see. Null while locked.
  pubkeyHex() {
    if (!wallet) return null;
    const p = wallet.publicKey();
    let h = "";
    for (let i = 0; i < p.length; i++) h += p[i].toString(16).padStart(2, "0");
    return "0x" + h;
  },
  verifyTx(pubkey, tx, sig) { return verifyTx(pubkey, tx, sig); },
  // The wallet's on-chain address in SS58 (prefix 1728). PUBLIC, like pubkeyHex; what the node's
  // nonce query keys on. Null while locked. (2026-09-23, market publishing.)
  ss58() { return wallet ? ss58Of(wallet.publicKey()) : null; },

  // Forge a NEW mask+wallet on-device, seal at rest, and return the one-time mnemonic to show. The phrase
  // is returned, never retained here; entropy/pass are zeroed on EVERY path (try/finally).
  async create(passphrase) {
    const entropy = crypto.getRandomValues(new Uint8Array(32));
    const pass = enc(passphrase);
    try {
      wallet = Wallet.fromEntropy(entropy);
      const handle = wallet.address();
      const mnemonic = entropyToPhrase(entropy);   // shown ONCE by the caller; not stored
      const salt = crypto.getRandomValues(new Uint8Array(16));
      const nonce = crypto.getRandomValues(new Uint8Array(24));
      const blob = sealKeystore(entropy, pass, salt, nonce);
      await idbPut(blob);
      return { handle, mnemonic };
    } finally { zero(entropy); zero(pass); }
  },

  // Decrypt the at-rest seed into memory. Wrong passphrase / tampered blob → throws (fail closed).
  async unlock(passphrase) {
    const blob = await idbGet();
    if (!blob) throw new Error("no keystore");
    const pass = enc(passphrase);
    let entropy = null;
    try {
      entropy = openKeystore(blob, pass); // throws on bad pass / tamper
      wallet = Wallet.fromEntropy(entropy);
      // Payload-free state-change signal for the logged-in shell (shell.js). NEVER carry a detail:
      // no seed/key/balance — listeners read state via the normal API. (#680, audited in the operator's sweep.)
      // try/catch (the operator's nit): a misbehaving listener can never propagate into the wallet flow.
      try { window.dispatchEvent(new Event("hlqwallet-changed")); } catch (e) {}
      return { handle: wallet.address() };
    } finally { zero(pass); if (entropy) zero(entropy); }
  },

  // Recover from the 24-word phrase, then re-seal at rest under a new passphrase.
  async restore(mnemonic, passphrase) {
    const entropy = phraseToEntropy(mnemonic.trim()); // throws on bad checksum
    const pass = enc(passphrase);
    try {
      wallet = Wallet.fromEntropy(entropy);
      const handle = wallet.address();
      const salt = crypto.getRandomValues(new Uint8Array(16));
      const nonce = crypto.getRandomValues(new Uint8Array(24));
      const blob = sealKeystore(entropy, pass, salt, nonce);
      await idbPut(blob);
      return { handle };
    } finally { zero(entropy); zero(pass); }
  },

  exportMnemonic() { return null; }, // by design: phrase shown ONCE at create, never retained

  // READ-ONLY balance, read through the panel's OWN-ORIGIN mediating backend — NEVER a raw node RPC from
  // the browser (PANEL-ARCHITECTURE §80/§83/§99: the node RPC is never exposed; `connect-src 'self'`; the
  // backend does the `TokensApi::balance_of` state_call server-side against a trusted/loopback node and
  // returns just the figures). No custody: a display can never move funds. `endpoint` is the same-origin
  // path of that backend; empty ⇒ honest zeros (never invent a number). A backend error rejects → the UI
  // shows "—" rather than a stale figure. Returns decimal strings (real u128, which the render groups).
  //
  // Backend contract (proposed; pending the operator confirm): GET <endpoint>?account=<hex32> →
  //   200 {"hlq":"<u128 dec>","sov":"<u128 dec>"}. Account = 32-byte sr25519 public key (== on-chain
  //   AccountId == the mask, #650). The backend validates the hex and never proxies raw RPC methods.
  async balances(endpoint) {
    if (!endpoint) return { hlq: "0", sov: "0" };
    if (!wallet) throw new Error("locked");
    const pub = wallet.publicKey();                 // 32-byte sr25519 public key == on-chain AccountId
    let hex = "";
    for (let i = 0; i < pub.length; i++) hex += pub[i].toString(16).padStart(2, "0");
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), 4000);
    let res;
    try {
      res = await fetch(endpoint + "?account=0x" + hex, {
        method: "GET",
        headers: { "Accept": "application/json" },
        credentials: "same-origin",
        signal: ctrl.signal,
      });
    } finally { clearTimeout(timer); }
    if (!res.ok) throw new Error("balance http " + res.status);
    const b = await res.json();
    // The backend returns decimal u128 strings; validate shape before trusting it for display.
    const dec = (v) => { const s = String(v); if (!/^\d+$/.test(s)) throw new Error("bad balance"); return s; };
    return { hlq: dec(b.hlq), sov: dec(b.sov) };
  },

  // Sign + submit a transfer. Signing is local (domain hlq-wallet-tx-v1); extrinsic assembly + submit is
  // the remaining node integration. Disabled until wired.
  async send(/* tx, rpc */) { throw new Error("send pending: extrinsic assembly + submit (the operator follow-up)"); },

  signTx(txBytes) {
    if (!wallet) throw new Error("locked");
    return wallet.signTx(txBytes);
  },

  // Sign a LOGIN challenge (scheme B): signLogin prepends b"harlequin-login:v1:" and signs under the standard
  // context b"substrate", so a login signature can never verify as a transaction (message-domain separation).
  // `nonce` is the RAW challenge bytes (Uint8Array); the wasm prepends the prefix. Requires the wallet_wasm
  // build with the signLogin export — throws clearly if the loaded wasm predates it. Locked ⇒ throws.
  signLogin(nonce) {
    if (!wallet) throw new Error("locked");
    if (typeof wallet.signLogin !== "function") throw new Error("signLogin unavailable: wallet_wasm predates login support");
    return wallet.signLogin(nonce);
  },

  // Sign a Substrate EXTRINSIC signing-payload — the Key governance flow (#937: cold-signing the runtime
  // upgrade). The wasm signs the RAW SignedPayload under context b"substrate" with NO message-domain
  // prefix — UNLIKE signTx (domain hlq-wallet-tx-v1) and signLogin (harlequin-login:v1:), which prepend
  // their own domains and MUST NOT be used for extrinsics or the runtime would reject the signature.
  // Per the sr25519 substrate rule, the wasm blake2_256-hashes the payload first IFF it exceeds 256 bytes
  // (propose/approve ~194B → signed as-is; apply_upgrade carries the >1MB runtime WASM → hashed). That
  // hash-if-long branch lives INSIDE the wasm, so this wrapper just forwards the raw payload. `payload` is
  // the RAW SignedPayload bytes (Uint8Array) built by the driver; returns the 64-byte sr25519 signature.
  // The seed never leaves the wasm — only the 64-byte signature comes back. Requires the wallet_wasm
  // build with signExtrinsicPayload — throws clearly otherwise. Locked ⇒ throws.
  signExtrinsicPayload(payload) {
    if (!wallet) throw new Error("locked");
    if (typeof wallet.signExtrinsicPayload !== "function") {
      throw new Error("signExtrinsicPayload unavailable: wallet_wasm predates governance-signing support (#937)");
    }
    return wallet.signExtrinsicPayload(payload);
  },

  lock() {
    if (wallet) { wallet.free(); wallet = null; }
    // Payload-free state-change signal for the logged-in shell (shell.js). Bare Event, no detail. (#680)
    try { window.dispatchEvent(new Event("hlqwallet-changed")); } catch (e) {}
  }
};

init()
  .then(() => {
    window.HlqWallet = Hlq;
    window.dispatchEvent(new Event("hlqwallet-ready"));
  })
  .catch((e) => { if (window.console) console.error("wallet-core wasm init failed", e); });
