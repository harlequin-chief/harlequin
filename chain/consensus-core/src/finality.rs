//! Hash-based Snowball finality round (SPEC §2.2): the bridge between the validated binary voting core
//! (`snowball::SnowballNode`) and real block finality, where the "colour" is a 32-byte block hash.
//!
//! A committee member runs one [`FinalityRound`] per target height (the frontier `last_finalized + 1`).
//! Each round it learns, for the peers it sampled, *which block hash they prefer at that height*; from
//! that multiset it advances exactly like the binary core — reinforce the leading hash, build a streak,
//! finalise after `beta` confirmations while a network quorum is reachable (anti-partition guard).
//!
//! **Why a generalisation and not a literal reuse of `SnowballNode`:** the binary core decides between
//! two colours `{0,1}`; finality must decide among *N* competing hashes (normally one — a non-forking
//! chain — occasionally two under an attempted fork). The rule below is the binary rule with the colour
//! replaced by a hash and the tie-break "ties go to colour 1" replaced by "ties go to the larger hash"
//! (any total order works; this one is deterministic and architecture-independent). On the two-hash
//! case it is **bit-identical** to `SnowballNode` — proven in the tests — so the 13/13 cross-validation
//! of the core carries over. No floats, `no_std`: reproducible on every node.

use crate::snowball::SnowballParams;
use alloc::collections::BTreeMap;

/// A 32-byte block hash — the value the committee is finalising.
pub type Hash = [u8; 32];

/// One committee member's finalisation state for a single target height. Mirrors `SnowballNode`
/// (`pref`/`streak`/`decision`) with the colour generalised to a block hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FinalityRound {
    pref: Hash,
    streak: u32,
    decision: Option<Hash>,
}

impl FinalityRound {
    /// New round preferring `initial` (a node's own best block hash at this height). The voting
    /// parameters are passed to each [`observe_round`](Self::observe_round) call (like
    /// `SnowballNode`), so the same long-lived round can absorb a committee that grows between rounds
    /// **without losing its streak** — the round is created once per height and fed every round.
    pub fn new(initial: Hash) -> Self {
        FinalityRound { pref: initial, streak: 0, decision: None }
    }

    /// The current (non-final) preferred hash.
    pub fn pref(&self) -> Hash {
        self.pref
    }

    /// Consecutive-round streak on the current preference.
    pub fn streak(&self) -> u32 {
        self.streak
    }

    /// The finalised hash, if decided. Once `Some`, it never changes (irreversibility).
    pub fn decision(&self) -> Option<Hash> {
        self.decision
    }

    /// Whether this height has finalised.
    pub fn is_decided(&self) -> bool {
        self.decision.is_some()
    }

    /// Process one voting round from the hashes this node received from its sampled peers (`votes` is
    /// post-sampling and post-loss: only the responses that actually arrived). `reaches_quorum` is the
    /// anti-partition guard (see `SnowballNode::observe_round`): pass the share of network reputation the
    /// node can currently see clearing the quorum bar; under a partition the isolated side never does, so
    /// it stalls instead of forking. Returns the finalised hash iff this node finalises on *this* round.
    pub fn observe_round(
        &mut self,
        votes: &[Hash],
        p: &SnowballParams,
        reaches_quorum: bool,
    ) -> Option<Hash> {
        // A decided node is frozen: it keeps reporting its decision but no longer evolves.
        if self.decision.is_some() {
            return None;
        }

        match leading(votes) {
            // The leading hash carries a quorum of responses: reinforce it (Snowball).
            Some((color, count)) if count >= p.alpha => {
                if color == self.pref {
                    self.streak += 1;
                } else {
                    self.pref = color;
                    self.streak = 1;
                }
                // Finalise only with enough confirmations AND while a network quorum is reachable.
                if self.streak >= p.beta && reaches_quorum {
                    self.decision = Some(color);
                    return self.decision;
                }
            }
            // No hash reached quorum this round: the preference holds, but the streak restarts.
            _ => {
                self.streak = 0;
            }
        }
        None
    }
}

/// Tally a round's received hashes and return the leading hash and its count. The leading hash is the
/// most-voted one; ties break to the **larger** hash (a fixed total order → deterministic on every
/// node), mirroring the binary core's "ties go to colour 1". `None` for an empty round.
fn leading(votes: &[Hash]) -> Option<(Hash, u32)> {
    if votes.is_empty() {
        return None;
    }
    let mut counts: BTreeMap<Hash, u32> = BTreeMap::new();
    for &h in votes {
        *counts.entry(h).or_insert(0) += 1;
    }
    // Pick max by (count, hash): higher count wins; on equal counts the larger hash wins.
    counts.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snowball::SnowballNode;
    use alloc::vec;
    use alloc::vec::Vec;

    const A: Hash = [0xAA; 32]; // the legitimate / canonical block
    const B: Hash = [0xBB; 32]; // a competing fork block (larger than A)

    fn many(h: Hash, n: usize) -> Vec<Hash> {
        vec![h; n]
    }

    #[test]
    fn unanimous_finalises_after_beta() {
        let p = SnowballParams::default();
        let mut r = FinalityRound::new(A);
        let votes = many(A, p.k as usize);
        for _ in 0..(p.beta - 1) {
            assert_eq!(r.observe_round(&votes, &p, true), None);
            assert!(!r.is_decided());
        }
        assert_eq!(r.observe_round(&votes, &p, true), Some(A));
        assert_eq!(r.decision(), Some(A));
    }

    #[test]
    fn flips_to_overwhelming_competitor() {
        let p = SnowballParams::default();
        let mut r = FinalityRound::new(A);
        let votes = many(B, p.k as usize);
        assert_eq!(r.observe_round(&votes, &p, true), None);
        assert_eq!(r.pref(), B);
        assert_eq!(r.streak(), 1);
        for _ in 0..(p.beta - 2) {
            r.observe_round(&votes, &p, true);
        }
        assert_eq!(r.observe_round(&votes, &p, true), Some(B));
    }

    #[test]
    fn split_below_quorum_resets_streak() {
        let p = SnowballParams::default();
        let mut r = FinalityRound::new(A);
        for _ in 0..5 {
            r.observe_round(&many(A, p.k as usize), &p, true);
        }
        assert_eq!(r.streak(), 5);
        // 10 A vs 10 B: leading count 10 < alpha(14) -> no quorum -> streak resets.
        let mut split: Vec<Hash> = many(A, 10);
        split.extend(many(B, 10));
        assert_eq!(r.observe_round(&split, &p, true), None);
        assert_eq!(r.streak(), 0);
    }

    #[test]
    fn quorum_guard_blocks_finalisation_under_partition() {
        let p = SnowballParams::default();
        let mut r = FinalityRound::new(A);
        let votes = many(A, p.k as usize);
        for _ in 0..(p.beta + 5) {
            assert_eq!(r.observe_round(&votes, &p, false), None);
        }
        assert!(!r.is_decided());
        assert!(r.streak() >= p.beta);
        assert_eq!(r.observe_round(&votes, &p, true), Some(A));
    }

    #[test]
    fn decided_round_is_frozen() {
        let p = SnowballParams::default();
        let mut r = FinalityRound::new(A);
        for _ in 0..p.beta {
            r.observe_round(&many(A, p.k as usize), &p, true);
        }
        assert_eq!(r.decision(), Some(A));
        // Even an overwhelming counter-vote cannot move a finalised round.
        assert_eq!(r.observe_round(&many(B, p.k as usize), &p, true), None);
        assert_eq!(r.decision(), Some(A));
        assert_eq!(r.pref(), A);
    }

    /// The crux: on the binary (two-hash) case `FinalityRound` is bit-identical to the validated
    /// `SnowballNode`, so the core's 13/13 cross-validation carries over to finality. We map colour
    /// `0 -> A`, `1 -> B` (B > A, matching "ties go to colour 1" <-> "ties go to the larger hash") and
    /// drive both with the same round inputs, asserting equal decisions at every step.
    #[test]
    fn reduces_to_snowball_on_binary_case() {
        let p = SnowballParams::default();
        // A spread of rounds: building, a flip, a tie, recovery.
        let rounds: &[(u32, u32)] = &[
            (20, 0),
            (18, 2),
            (3, 17), // overwhelming B -> flip
            (10, 10), // tie -> below alpha, reset
            (16, 4),
            (15, 5),
            (20, 0), // back to A
        ];
        let mut sn = SnowballNode::new(0);
        let mut fr = FinalityRound::new(A);
        for &(zeros, ones) in rounds {
            // colour votes for the binary core (0 = A, 1 = B).
            let mut cv: Vec<u8> = vec![0u8; zeros as usize];
            cv.extend(vec![1u8; ones as usize]);
            // the same round as hashes for finality.
            let mut hv: Vec<Hash> = many(A, zeros as usize);
            hv.extend(many(B, ones as usize));

            let sd = sn.observe_round(&cv, &p, true);
            let fd = fr.observe_round(&hv, &p, true);
            // Decisions agree (mapped through the colour<->hash bijection) at every step.
            let fd_as_colour = fd.map(|h| if h == A { 0u8 } else { 1u8 });
            assert_eq!(sd, fd_as_colour, "decision mismatch on round ({zeros},{ones})");
            // And so do the live preference and streak.
            assert_eq!(sn.streak(), fr.streak());
            let pref_colour = if fr.pref() == A { 0 } else { 1 };
            assert_eq!(sn.pref(), pref_colour);
        }
    }
}
