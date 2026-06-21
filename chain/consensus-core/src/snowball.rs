//! Per-node Snowball decision core (sub-sampled Avalanche voting, SPEC §2.2; PAPER §5.4) — the
//! integer heart of Woven-Trust finality, ported from the validated test-rig
//! (the consensus reference, behaviour reference). **Pure integer, `no_std`.**
//!
//! Separation of concerns: *who* a node samples each round is reputation-weighted and is the job of
//! the sortition (`sortition_fp.rs`) + the node's p2p layer (and is where network loss happens). What
//! lives here is what is reproducible and consensus-critical: given the votes a node *received* this
//! round, how its preference, streak and final decision evolve. No floats enter the decision rule —
//! only counts — so it is bit-identical on every architecture (a hard requirement for consensus).
//!
//! Model (binary decision, as in the test-rig: enough to capture safety and forking): an undecided
//! honest node queries `k` peers; if at least `alpha` of the responses agree on a colour it reinforces
//! that colour (Snowball); after `beta` consecutive rounds on the same colour — *and* only while it can
//! reach a network quorum of reputation (anti-partition guard) — it finalises. Below `alpha` the streak
//! resets. The quorum bar `alpha/k = 14/20 = 0.70` is "the wall ♠": truth in Harlequin is near-
//! consensus, never a 51 % vote (see the consensus parameters reference).

/// Voting parameters. Defaults echo the test-rig and `PARAMETERS.md`: `k=20` (runs on a phone ♣),
/// `alpha=14` (the 0.70 wall ♠), `beta=12` (confirmations before irreversibility).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnowballParams {
    /// Sample size per round (peers queried).
    pub k: u32,
    /// Quorum: minimum agreeing responses for a round to count. Must be `> k/2`.
    pub alpha: u32,
    /// Consecutive rounds on the same colour required to finalise.
    pub beta: u32,
    /// Safety bound on rounds before a node gives up converging (liveness ceiling).
    pub max_rounds: u32,
}

impl Default for SnowballParams {
    fn default() -> Self {
        SnowballParams { k: 20, alpha: 14, beta: 12, max_rounds: 80 }
    }
}

/// One honest node's voting state machine. Adversarial behaviour is *not* modelled here — a Byzantine
/// node simply reports whatever colour it likes, which surfaces to honest nodes as their received
/// votes; this core only governs an honest node's own evolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnowballNode {
    pref: u8,
    streak: u32,
    decision: Option<u8>,
}

impl SnowballNode {
    /// New node preferring `initial_pref` (honest nodes start at the legitimate value `0`).
    pub fn new(initial_pref: u8) -> Self {
        SnowballNode { pref: initial_pref, streak: 0, decision: None }
    }

    /// The node's current (non-final) preference.
    pub fn pref(&self) -> u8 {
        self.pref
    }

    /// Consecutive-round streak on the current preference.
    pub fn streak(&self) -> u32 {
        self.streak
    }

    /// The finalised decision, if the node has decided. Once `Some`, it never changes.
    pub fn decision(&self) -> Option<u8> {
        self.decision
    }

    /// Whether the node has finalised.
    pub fn is_decided(&self) -> bool {
        self.decision.is_some()
    }

    /// Process one voting round from the votes this node received from its sampled peers
    /// (`votes` is post-sampling and post-loss: only the responses that actually arrived).
    ///
    /// `reaches_quorum` is the anti-partition guard (test-rig `reaches_quorum`): the node may finalise
    /// only while it can see at least the required share of the network's total reputation. Under a
    /// partition the isolated side never reaches it, so it *stalls instead of forking* and heals on
    /// reconnection. Pass `true` when there is no partition mitigation.
    ///
    /// Returns the decision iff the node finalises on *this* round.
    pub fn observe_round(
        &mut self,
        votes: &[u8],
        p: &SnowballParams,
        reaches_quorum: bool,
    ) -> Option<u8> {
        // A decided node is frozen: it keeps reporting its decision but no longer evolves.
        if self.decision.is_some() {
            return None;
        }

        // Binary tally of the received responses. Ties go to colour 1, matching the test-rig exactly
        // (`(1, ones) if ones >= zeros else (0, zeros)`) so the validated behaviour is preserved.
        let ones = votes.iter().filter(|&&c| c == 1).count() as u32;
        let zeros = votes.len() as u32 - ones;
        let (color, count) = if ones >= zeros { (1u8, ones) } else { (0u8, zeros) };

        if count >= p.alpha {
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
        } else {
            // No quorum this round: the preference is not abandoned, but the streak must restart.
            self.streak = 0;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn votes_of(color: u8, n: usize) -> Vec<u8> {
        vec![color; n]
    }

    #[test]
    fn defaults_match_test_rig() {
        let p = SnowballParams::default();
        assert_eq!((p.k, p.alpha, p.beta, p.max_rounds), (20, 14, 12, 80));
        // The wall: a strict supermajority, never a bare 51 %.
        assert!(p.alpha * 2 > p.k);
    }

    #[test]
    fn unanimous_legitimate_decides_zero_after_beta() {
        let p = SnowballParams::default();
        let mut n = SnowballNode::new(0);
        let votes = votes_of(0, p.k as usize);
        // beta-1 rounds: streak builds but no decision yet.
        for _ in 0..(p.beta - 1) {
            assert_eq!(n.observe_round(&votes, &p, true), None);
            assert!(!n.is_decided());
        }
        // The beta-th confirming round finalises.
        assert_eq!(n.observe_round(&votes, &p, true), Some(0));
        assert_eq!(n.decision(), Some(0));
    }

    #[test]
    fn flips_to_adversary_colour_when_overwhelmed() {
        let p = SnowballParams::default();
        let mut n = SnowballNode::new(0);
        let ones = votes_of(1, p.k as usize);
        // First overwhelming round flips preference to 1 and resets streak to 1.
        assert_eq!(n.observe_round(&ones, &p, true), None);
        assert_eq!(n.pref(), 1);
        assert_eq!(n.streak(), 1);
        for _ in 0..(p.beta - 2) {
            n.observe_round(&ones, &p, true);
        }
        assert_eq!(n.observe_round(&ones, &p, true), Some(1));
    }

    #[test]
    fn below_quorum_resets_streak() {
        let p = SnowballParams::default();
        let mut n = SnowballNode::new(0);
        let votes = votes_of(0, p.k as usize);
        for _ in 0..5 {
            n.observe_round(&votes, &p, true);
        }
        assert_eq!(n.streak(), 5);
        // A split round: 10 vs 10 -> majority colour 1 with count 10 < alpha(14) -> no quorum.
        let split: Vec<u8> = (0..p.k).map(|i| (i % 2) as u8).collect();
        assert_eq!(n.observe_round(&split, &p, true), None);
        assert_eq!(n.streak(), 0);
    }

    #[test]
    fn quorum_guard_blocks_finalisation_under_partition() {
        let p = SnowballParams::default();
        let mut n = SnowballNode::new(0);
        let votes = votes_of(0, p.k as usize);
        // Build a decisive streak but never reaching a network quorum (isolated partition side).
        for _ in 0..(p.beta + 5) {
            assert_eq!(n.observe_round(&votes, &p, false), None);
        }
        assert!(!n.is_decided());
        assert!(n.streak() >= p.beta);
        // Network heals: quorum reachable again -> the very next confirming round finalises.
        assert_eq!(n.observe_round(&votes, &p, true), Some(0));
    }

    #[test]
    fn decided_node_is_frozen() {
        let p = SnowballParams::default();
        let mut n = SnowballNode::new(0);
        let zeros = votes_of(0, p.k as usize);
        for _ in 0..p.beta {
            n.observe_round(&zeros, &p, true);
        }
        assert_eq!(n.decision(), Some(0));
        // Even an overwhelming counter-vote cannot move a finalised node.
        let ones = votes_of(1, p.k as usize);
        assert_eq!(n.observe_round(&ones, &p, true), None);
        assert_eq!(n.decision(), Some(0));
        assert_eq!(n.pref(), 0);
    }
}
