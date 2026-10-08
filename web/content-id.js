/* Harlequin — content addressing for the browser (#592/#613/#651).  [ES MODULE]
 *
 * Mirrors the dependency-free `content-id` Rust core EXACTLY, so the client can verify a body fetched from
 * IPFS against the 32-byte digest the chain stores — verify-by-recompute, the same trust model as the forum
 * and the market board. The client NEVER trusts the IPFS gateway (its own or anyone's): it recomputes the
 * CID/digest from the bytes and compares to the on-chain hash.
 *
 *   digest(body)        = sha256(body)                         (the 32 bytes stored on-chain)
 *   cidV1Raw(body)      = "b" + base32(0x01 0x55 0x12 0x20 || digest)   → "bafkrei…"
 *                          version 0x01 | raw codec 0x55 | sha2-256 0x12 | length 0x20
 *
 * base32 = RFC4648 lowercase, NO padding; multibase prefix 'b'. Same alphabet as the Rust core.
 */

const CID_PREFIX = new Uint8Array([0x01, 0x55, 0x12, 0x20]);
const ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

export function toHex(u8) {
  let s = "";
  for (let i = 0; i < u8.length; i++) s += u8[i].toString(16).padStart(2, "0");
  return s;
}

// RFC4648 base32 lower, no padding (matches base32_lower_nopad_into in the Rust core).
function base32LowerNopad(data) {
  let out = "";
  let buffer = 0, bits = 0;
  for (let i = 0; i < data.length; i++) {
    buffer = (buffer << 8) | data[i];
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += ALPHABET[(buffer >>> bits) & 0x1f];
    }
  }
  if (bits > 0) out += ALPHABET[(buffer << (5 - bits)) & 0x1f];
  return out;
}

// sha256(body) → Uint8Array(32). Uses the host WebCrypto (no dependency, no key material).
export async function digest(bodyBytes) {
  const buf = await crypto.subtle.digest("SHA-256", bodyBytes);
  return new Uint8Array(buf);
}

// The CIDv1 string for a known 32-byte digest (when the digest is already on-chain).
export function cidV1FromDigest(digest32) {
  const bytes = new Uint8Array(CID_PREFIX.length + 32);
  bytes.set(CID_PREFIX, 0);
  bytes.set(digest32, CID_PREFIX.length);
  return "b" + base32LowerNopad(bytes);
}

// The CIDv1 string for a body (recompute from raw bytes).
export async function cidV1Raw(bodyBytes) {
  return cidV1FromDigest(await digest(bodyBytes));
}

// The canonical mask handle for a 32-byte public key: "hlq-" + base32(sha256(pubkey)[..16]) (#650/#652).
// The client DERIVES this from the pubkey and never trusts a feed-supplied handle — a compromised feed
// must not be able to paint a foreign handle over someone's pubkey.
export async function deriveHandle(pubkeyBytes) {
  const h = await digest(pubkeyBytes);            // sha256(pubkey)
  return "hlq-" + base32LowerNopad(h.subarray(0, 16));
}

// hex string ("0x…" or bare) → Uint8Array.
export function fromHex(hex) {
  const s = hex.replace(/^0x/, "");
  const out = new Uint8Array(s.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(s.substr(i * 2, 2), 16);
  return out;
}

// Verify a body fetched from IPFS against the on-chain digest (hex or Uint8Array). The gateway is never
// trusted: this re-derives the digest from the bytes and compares. Returns true iff they match exactly.
export async function verifyBody(bodyBytes, onchainDigest) {
  const want = typeof onchainDigest === "string"
    ? onchainDigest.replace(/^0x/, "").toLowerCase()
    : toHex(onchainDigest);
  const got = toHex(await digest(bodyBytes));
  return got === want;
}
