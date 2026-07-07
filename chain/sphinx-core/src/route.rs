//! Path selection — pick `k` relays weighted by reputation, not by pure chance.
//!
//! the maintainer + the reviewer: hops are chosen by reputation so a flood of fresh Sybil relays cannot expect to land
//! on a circuit. This mirrors the consensus sortition (reputation = the scarce, earned weight) rather
//! than committing to a node-count majority. The selection here is deterministic given a public `seed`
//! (so it is verifiable / reproducible); in production the seed comes from the same kind of public
//! randomness the consensus uses, and `candidates` is the live, reputation-bearing relay set.
//!
//! Dependency-free; the reputation values are plugged in by the caller (on-chain reputation, SPEC §1).

use crate::crypto::Key;
use crate::packet::MAX_HOPS;
use alloc::vec::Vec;
use consensus_core::sha256::sha256;

/// A relay eligible to carry a circuit, with its earned reputation weight.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub public: [u8; 32],
    pub reputation: u64,
}

/// Select `k` distinct relays weighted by reputation, deterministically from `seed`.
///
/// Weighted sampling without replacement: each round draws a point in `[0, total_reputation)` from the
/// seed and picks the candidate whose cumulative weight covers it. Zero-reputation candidates are never
/// chosen (no Sybil free-ride). Returns the chosen relays' public keys in selection order.
///
/// Returns `None` if `k` is out of range or there are not `k` positive-reputation candidates.
pub fn select_path(candidates: &[Candidate], k: usize, seed: &Key) -> Option<Vec<[u8; 32]>> {
    if k == 0 || k > MAX_HOPS {
        return None;
    }
    let positive = candidates.iter().filter(|c| c.reputation > 0).count();
    if positive < k {
        return None;
    }

    // Mutable pool of remaining candidates (indices into `candidates`).
    let mut pool: Vec<usize> = (0..candidates.len()).filter(|&i| candidates[i].reputation > 0).collect();
    let mut chosen: Vec<[u8; 32]> = Vec::with_capacity(k);

    for round in 0..k {
        let total: u128 = pool.iter().map(|&i| candidates[i].reputation as u128).sum();
        // Draw in [0, total) from the seed and the round index.
        let draw = draw_u128(seed, round as u32) % total;
        let mut acc: u128 = 0;
        let mut hit = 0usize;
        for (pos, &i) in pool.iter().enumerate() {
            acc += candidates[i].reputation as u128;
            if draw < acc {
                hit = pos;
                break;
            }
        }
        chosen.push(candidates[pool[hit]].public);
        pool.swap_remove(hit);
    }
    Some(chosen)
}

/// Derive a 128-bit draw from `seed` and a round counter.
fn draw_u128(seed: &Key, round: u32) -> u128 {
    let h = sha256(&[&seed[..], b"hlq-route", &round.to_le_bytes()].concat());
    let mut b = [0u8; 16];
    b.copy_from_slice(&h[..16]);
    u128::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(seed: u8, rep: u64) -> Candidate {
        Candidate { public: [seed; 32], reputation: rep }
    }

    #[test]
    fn picks_k_distinct() {
        let cs = [cand(1, 10), cand(2, 10), cand(3, 10), cand(4, 10), cand(5, 10)];
        let path = select_path(&cs, 3, &[7u8; 32]).unwrap();
        assert_eq!(path.len(), 3);
        // distinct
        for i in 0..path.len() {
            for j in i + 1..path.len() {
                assert_ne!(path[i], path[j]);
            }
        }
    }

    #[test]
    fn zero_reputation_never_selected() {
        let cs = [cand(1, 0), cand(2, 0), cand(3, 5), cand(4, 5), cand(5, 5)];
        let path = select_path(&cs, 3, &[1u8; 32]).unwrap();
        for p in &path {
            assert_ne!(p, &[1u8; 32]);
            assert_ne!(p, &[2u8; 32]);
        }
    }

    #[test]
    fn deterministic_for_same_seed() {
        let cs = [cand(1, 3), cand(2, 7), cand(3, 11), cand(4, 2), cand(5, 9)];
        assert_eq!(select_path(&cs, 4, &[3u8; 32]), select_path(&cs, 4, &[3u8; 32]));
    }

    #[test]
    fn not_enough_positive_candidates() {
        let cs = [cand(1, 0), cand(2, 1)];
        assert!(select_path(&cs, 3, &[1u8; 32]).is_none());
    }

    #[test]
    fn high_reputation_dominates_distribution() {
        // One whale (rep 1000) vs four minnows (rep 1): the whale appears in nearly every 1-hop draw.
        let cs = [cand(1, 1000), cand(2, 1), cand(3, 1), cand(4, 1), cand(5, 1)];
        let mut whale = 0;
        for s in 0u8..100 {
            let p = select_path(&cs, 1, &[s; 32]).unwrap();
            if p[0] == [1u8; 32] {
                whale += 1;
            }
        }
        assert!(whale > 90, "whale picked {whale}/100");
    }
}
