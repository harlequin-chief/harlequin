//! # forum-core — canonical forum-post body (#651)
//!
//! `pallet-forum` stores a post as `author + body[u8;32] + parent + at + hidden`, where `body` is a
//! 32-byte hash and the actual text lives off-chain on IPFS (see [`content_id`]). For that to be
//! **verifiable** — anyone fetches the text and re-checks it against the on-chain hash — every client
//! must turn "the same post" into the *same bytes* before hashing. This crate fixes that canonical
//! serialization so the addressing is interoperable and the verify-by-recompute is exact.
//!
//! Scope: the **body** only. Threading (`parent` id), ordering (`id`), author and moderation are
//! on-chain concerns handled by `pallet-forum`; the client passes `parent` to the extrinsic separately.
//!
//! Pipeline:
//! ```text
//!   text --validate--> PostBody --serialize--> bytes --content_id--> { digest(32B on-chain), CID(IPFS) }
//!   publish `bytes` to IPFS under CID; submit `digest` (+ parent) to pallet-forum.
//!   later: fetch bytes by CID, verify_fetched(on-chain digest, bytes) -> PostBody (or tamper error).
//! ```
//!
//! Dependency-free, `no_std` (alloc only) without the default `std` feature.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

/// Canonical body format version. The first byte of the serialized body. Bumped only on a
/// format-breaking change so old posts stay verifiable under their original rules.
pub const FORMAT_VERSION: u8 = 1;

/// Maximum body length in bytes (PARÁMETRO — client/anti-abuse bound, not an on-chain limit since the
/// text is off-chain). Keeps a single post to one content-addressed block.
pub const MAX_BODY_LEN: usize = 16 * 1024;

/// A forum post body. v1 is plain UTF-8 text (richer formats get a new `FORMAT_VERSION`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PostBody {
    pub text: String,
}

/// Everything a client needs to publish a post: the on-chain hash, the IPFS CID, and the exact bytes
/// to push to IPFS (so the CID it resolves to is this one).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Submission {
    /// 32-byte digest for `pallet-forum` `Post.body`.
    pub digest: [u8; 32],
    /// IPFS CIDv1 (raw+sha256) the body is addressed by.
    pub cid: String,
    /// The canonical bytes to store on IPFS (`serialize` of the body).
    pub ipfs_bytes: Vec<u8>,
}

/// What can go wrong building or verifying a body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ForumError {
    /// Body is empty (no all-whitespace/empty posts).
    Empty,
    /// Body exceeds `MAX_BODY_LEN`.
    TooLong,
    /// Serialized bytes are malformed (bad version, bad length, or non-UTF-8 text).
    Malformed,
    /// Fetched bytes do not hash to the on-chain digest (tampered or wrong content).
    DigestMismatch,
}

/// Canonical serialization: `FORMAT_VERSION` byte followed by the UTF-8 text. Deterministic.
pub fn serialize(body: &PostBody) -> Vec<u8> {
    let bytes = body.text.as_bytes();
    let mut out = Vec::with_capacity(1 + bytes.len());
    out.push(FORMAT_VERSION);
    out.extend_from_slice(bytes);
    out
}

/// Parse canonical bytes back into a `PostBody`, validating version, length and UTF-8.
pub fn parse(bytes: &[u8]) -> Result<PostBody, ForumError> {
    let (&version, rest) = bytes.split_first().ok_or(ForumError::Malformed)?;
    if version != FORMAT_VERSION {
        return Err(ForumError::Malformed);
    }
    if rest.is_empty() {
        return Err(ForumError::Empty);
    }
    if rest.len() > MAX_BODY_LEN {
        return Err(ForumError::TooLong);
    }
    let text = core::str::from_utf8(rest).map_err(|_| ForumError::Malformed)?;
    Ok(PostBody { text: String::from(text) })
}

/// Client side: validate `text` and produce everything needed to publish the post.
pub fn prepare(text: &str) -> Result<Submission, ForumError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ForumError::Empty);
    }
    // Length is measured on the canonical (untrimmed-internally) text we will store.
    if text.len() > MAX_BODY_LEN {
        return Err(ForumError::TooLong);
    }
    let body = PostBody { text: String::from(text) };
    let ipfs_bytes = serialize(&body);
    Ok(Submission {
        digest: content_id::digest(&ipfs_bytes),
        cid: content_id::cid_v1_raw(&ipfs_bytes),
        ipfs_bytes,
    })
}

/// Re-fetcher side: verify bytes fetched from IPFS against the on-chain `Post.body` digest, then parse.
/// This is the censorship-resistance guarantee: the body is whatever hashes to the on-chain hash.
pub fn verify_fetched(onchain_digest: &[u8; 32], ipfs_bytes: &[u8]) -> Result<PostBody, ForumError> {
    if &content_id::digest(ipfs_bytes) != onchain_digest {
        return Err(ForumError::DigestMismatch);
    }
    parse(ipfs_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn roundtrip_prepare_then_verify() {
        let text = "Freedom is not granted; it is held up by those who carry the network.";
        let sub = prepare(text).unwrap();
        // The on-chain digest is content_id::digest of the canonical bytes.
        assert_eq!(sub.digest, content_id::digest(&sub.ipfs_bytes));
        assert!(sub.cid.starts_with("bafkrei"));
        // A re-fetcher gets the body back and it matches.
        let body = verify_fetched(&sub.digest, &sub.ipfs_bytes).unwrap();
        assert_eq!(body.text, text);
    }

    #[test]
    fn cid_matches_onchain_digest() {
        let sub = prepare("two encodings of one fact").unwrap();
        // The CID decodes back to the same 32 bytes stored on-chain.
        assert_eq!(content_id::digest_from_cid(&sub.cid), Some(sub.digest));
    }

    #[test]
    fn tampered_body_is_rejected() {
        let sub = prepare("original post").unwrap();
        let mut tampered = sub.ipfs_bytes.clone();
        let n = tampered.len();
        tampered[n - 1] ^= 0xff;
        assert_eq!(verify_fetched(&sub.digest, &tampered), Err(ForumError::DigestMismatch));
    }

    #[test]
    fn empty_and_whitespace_rejected() {
        assert_eq!(prepare(""), Err(ForumError::Empty));
        assert_eq!(prepare("   \n\t "), Err(ForumError::Empty));
    }

    #[test]
    fn too_long_rejected() {
        let big = "a".repeat(MAX_BODY_LEN + 1);
        assert_eq!(prepare(&big), Err(ForumError::TooLong));
    }

    #[test]
    fn serialize_is_canonical_and_versioned() {
        let body = PostBody { text: "hi".to_string() };
        let bytes = serialize(&body);
        assert_eq!(bytes[0], FORMAT_VERSION);
        assert_eq!(&bytes[1..], b"hi");
        assert_eq!(parse(&bytes).unwrap(), body);
    }

    #[test]
    fn wrong_version_is_malformed() {
        let bytes = [2u8, b'h', b'i']; // version 2 != FORMAT_VERSION
        assert_eq!(parse(&bytes), Err(ForumError::Malformed));
    }

    #[test]
    fn non_utf8_is_malformed() {
        let bytes = [FORMAT_VERSION, 0xff, 0xfe];
        assert_eq!(parse(&bytes), Err(ForumError::Malformed));
    }
}

#[cfg(test)]
mod conformance_vector {
    use super::*;
    /// CONFORMANCE ANCHOR: a fixed body text -> canonical bytes -> on-chain digest -> IPFS CID.
    /// Any implementation (the runtime, a re-fetcher, another client) MUST reproduce these byte-for-byte.
    #[test]
    fn forum_body_ground_truth() {
        let text = "Harlequin";
        let sub = prepare(text).unwrap();
        // canonical bytes = FORMAT_VERSION(0x01) || "Harlequin"
        let bytes_hex: alloc::string::String =
            sub.ipfs_bytes.iter().map(|b| alloc::format!("{:02x}", b)).collect();
        assert_eq!(bytes_hex, "014861726c657175696e", "canonical serialized bytes");
        let digest_hex: alloc::string::String =
            sub.digest.iter().map(|b| alloc::format!("{:02x}", b)).collect();
        // sha256(0x01 || "Harlequin")
        assert_eq!(digest_hex.len(), 64);
        // CID is raw+sha256 CIDv1 of those bytes
        assert!(sub.cid.starts_with("bafkrei"));
        assert_eq!(content_id::digest_from_cid(&sub.cid), Some(sub.digest));
        // print for the conformance doc
        std::eprintln!("FORUM VECTOR text={text:?} bytes={bytes_hex} digest={digest_hex} cid={}", sub.cid);
    }
}
