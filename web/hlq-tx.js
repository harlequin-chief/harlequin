/* Harlequin — shared on-device transaction path (2026-09-23). Used by the market and the forum.  [ES MODULE]
 *
 * One way to put something on chain from a browser, so every page does it the same (and a fix lands once):
 *   storeText(text)   → the mediator's content store keeps the text under its sha256; the page RE-HASHES it
 *                       itself and refuses a mismatch (the server cannot swap the words).
 *   submitCall(call)  → signs `call` with the mask ON THIS DEVICE using bind.js's builder (mortal era, the
 *                       runtime's signed extensions, v4 extrinsic — verified byte-for-byte against the
 *                       runtime) and hands the signed bytes to the mediator, which only relays them.
 *   openWallet()      → the unlocked wallet, or null (tries the empty passphrase: the rite's unlocked mask).
 * Requires /wallet-core.js (window.HlqWallet) and /bind.js (window.HlqBind) on the page.
 */
import { toHex, digest } from "./content-id.js";

const enc = new TextEncoder();
export const MAX_TEXT_BYTES = 4096;   // mediator store bound

export function capable() {
  const W = window.HlqWallet;
  return !!(W && typeof W.ss58 === "function" && typeof W.signExtrinsicPayload === "function" && window.HlqBind);
}

export async function openWallet() {
  const W = window.HlqWallet;
  if (!W) return null;
  if (!W.isUnlocked()) {
    try { if (await W.hasKeystore()) await W.unlock(""); } catch (e) { /* locked: the unlock bar handles it */ }
  }
  return W.isUnlocked() ? W : null;
}

async function getJSON(url, opts) {
  const r = await fetch(url, Object.assign({ cache: "no-store", credentials: "same-origin" }, opts || {}));
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || ("http " + r.status));
  return j;
}

/** Store `text` by its fingerprint; returns the 32-byte sha256 the chain will anchor. */
export async function storeText(text) {
  const bytes = enc.encode(text);
  if (!bytes.length) throw new Error("empty");
  if (bytes.length > MAX_TEXT_BYTES) throw new Error("too long");
  const stored = await getJSON("/api/detail", {
    method: "POST", headers: { "Content-Type": "text/plain; charset=utf-8" }, body: bytes
  });
  const mine = await digest(bytes);                   // never trust the server's hash
  if (toHex(mine) !== String(stored.detail_hash).replace(/^0x/, "")) throw new Error("the store returned a different fingerprint");
  return mine;
}

/** Sign `call` (Uint8Array: pallet index, call index, args) with the open mask and relay it. → tx hash. */
export async function submitCall(W, call) {
  const B = window.HlqBind._internals;
  const ss58 = W.ss58();
  const m0 = await getJSON("/api/tx/meta?ss58=" + encodeURIComponent(ss58));
  const era = B.eraFor(m0.best_number);
  const m = await getJSON("/api/tx/meta?ss58=" + encodeURIComponent(ss58) + "&birth=" + era.birth);
  const ctx = {
    specVersion: m.spec_version, txVersion: m.tx_version, genesis: B.hexToBytes(m.genesis_hash),
    nonce: Number(m.nonce), eraBytes: era.bytes, eraBlockHash: B.hexToBytes(m.birth_hash)
  };
  const payload = B.signingPayload(call, ctx);
  const sig = W.signExtrinsicPayload(payload);
  if (!sig || sig.length !== 64) throw new Error("the wallet did not return a signature");
  const xt = B.assembleExtrinsic(call, B.hexToBytes(W.pubkeyHex()), sig, ctx.nonce, ctx.eraBytes);
  const res = await getJSON("/api/tx", {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ xt: B.bytesToHex(xt) })
  });
  return String(res.hash);
}

export function compact(n) { return window.HlqBind._internals.compact(n); }
