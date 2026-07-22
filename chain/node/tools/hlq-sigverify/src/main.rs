//! hlq-sigverify — first-party sr25519 signature verifier for the Harlequin mediator (#637 login).
//!
//! The mediator is stdlib-only Python (no schnorrkel), so it shells out to THIS tiny binary to verify
//! a login signature server-side without pulling a third-party crypto dependency into the mediator.
//!
//! Contract (hardening agreed with the reviewer, subprocess = new attack surface):
//!   - Input is read ONLY from stdin, never argv (zero command injection). Three whitespace-separated
//!     hex tokens, in order: PUBLIC_KEY (32 bytes), MESSAGE (any length), SIGNATURE (64 bytes).
//!   - Output is the EXIT CODE only — the mediator trusts nothing else:
//!       0 = signature VALID
//!       1 = signature INVALID
//!       2 = malformed input (bad hex / wrong lengths / missing tokens)
//!   - The mediator calls this with an ABSOLUTE path, feeds hex it has already regex-validated, and
//!     enforces a 2s timeout treating anything but a clean exit 0 as FAIL-CLOSED.
//!   - Verification uses the standard substrate sr25519 signing context, so it matches wallet-core's
//!     signature exactly (login message = nonce ++ genesis_hash ++ issued_at, domain-separated by the
//!     `hlq-villa-login-v1` label the client folds into the signed bytes).

use sp_core::crypto::ByteArray;
use sp_core::sr25519::{Public, Signature};
use sp_core::Pair as _;
use std::io::Read;

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.is_empty() || s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn main() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        std::process::exit(2);
    }
    let mut toks = input.split_whitespace();
    let (pk_hex, msg_hex, sig_hex) = match (toks.next(), toks.next(), toks.next()) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        _ => std::process::exit(2),
    };
    // Exactly three tokens — a fourth means the caller sent something unexpected; refuse.
    if toks.next().is_some() {
        std::process::exit(2);
    }
    let (pk, msg, sig) = match (hex_decode(pk_hex), hex_decode(msg_hex), hex_decode(sig_hex)) {
        (Some(p), Some(m), Some(s)) => (p, m, s),
        _ => std::process::exit(2),
    };
    if pk.len() != 32 || sig.len() != 64 {
        std::process::exit(2);
    }
    let public = match Public::from_slice(&pk) {
        Ok(p) => p,
        Err(_) => std::process::exit(2),
    };
    let signature = match Signature::try_from(&sig[..]) {
        Ok(s) => s,
        Err(_) => std::process::exit(2),
    };
    // Standard substrate context — identical to wallet-core's signer and the node's own verify path.
    if sp_core::sr25519::Pair::verify(&signature, &msg, &public) {
        std::process::exit(0);
    }
    std::process::exit(1);
}
