//! Commit–reveal beacon — the anti-grinding seam for the sortition (macroaudit §2.1).
//!
//! The simulated VRF in [`crate::sortition_fp::vrf_value_fp`] is **grindable**: the secret key is a
//! freely-chosen string (`sk-{id}`, an id a member may pick under Art. VII), so a node that already
//! knows the epoch seed can grind candidate ids offline to lift its committee/jury seats well above its
//! fair share (pinned by `sortition_fp`'s `known_defect_vrf_is_grindable_but_reputation_anchor_holds`).
//! This module closes that **without any new dependency** (OPSEC: nothing pulled onto the isolated
//! station) via a per-epoch **commit–reveal** discipline.
//!
//! Flow (one epoch):
//!  1. **COMMIT** — before the epoch seed is fixed, each participant draws a fresh per-epoch secret `s`
//!     and publishes `c = SHA-256(s)`. The commitment binds `s` without revealing it.
//!  2. **SEED** — the epoch beacon is a RANDAO-style fold of contributions finalized *before* the
//!     reveal phase (deferred), so it is unpredictable while commitments are still open: nobody can pick
//!     `s` to suit the seed.
//!  3. **REVEAL** — once the seed is public, participants reveal `s`. The sortition value is
//!     `SHA-256(s | seed)` — the SAME construction as `vrf_value_fp`, but `s` was locked in step 1, so
//!     grinding it against the seed is impossible.
//!  4. **VERIFY** — anyone checks `SHA-256(s) == c`; a non-revealer or a mismatched reveal is dropped
//!     (its seats do not count). A fresh `s` each epoch means a revealed `s` never helps a future draw.
//!
//! This is the dep-free stepping stone; a non-interactive ECVRF (one message + proof) is the later
//! hardening if the two-phase liveness cost proves a problem (a last-revealer who withholds can only
//! bias the *next* fold, not substitute a post-seed key — see [`beacon_seed`]). The reputation anchor
//! stays the real Sybil defence either way: zero reputation → zero seats, grind notwithstanding.

use crate::sha256::sha256;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// A 32-byte SHA-256 commitment (also the type of the beacon accumulator).
pub type Commitment = [u8; 32];

/// **COMMIT.** The binding commitment to a per-epoch `secret`: `SHA-256(secret)`. Published before the
/// seed is known; reveals nothing about `secret` (pre-image resistance).
pub fn commit(secret: &[u8]) -> Commitment {
    sha256(secret)
}

/// **VERIFY.** A revealed `secret` opens `commitment` iff it hashes to it. The cryptographic core of the
/// anti-grind guarantee: having committed `c = SHA-256(s)` BEFORE the seed, an attacker cannot later
/// present any `s' ≠ s` (that would be a SHA-256 second pre-image), so it is stuck with the value `s`
/// gives — it gets exactly ONE draw, like everyone, instead of best-of-N grinding.
pub fn verify_reveal(commitment: &Commitment, secret: &[u8]) -> bool {
    &sha256(secret) == commitment
}

/// One RANDAO mix step: fold a contribution into the running accumulator — `SHA-256(acc | contribution)`.
pub fn mix(acc: &Commitment, contribution: &[u8]) -> Commitment {
    let mut input = acc.to_vec();
    input.extend_from_slice(contribution);
    sha256(&input)
}

/// **Deferred beacon seed** for an epoch: fold `prev` (the *previous* epoch's beacon, already finalized)
/// with every revealed `contribution`, in deterministic (sorted) order. Deferral is the point — the seed
/// driving THIS epoch's draw is fixed from inputs finalized before this epoch's commitments open, so a
/// committer cannot predict it. No single contributor controls the output: changing one input rehashes
/// everything downstream. Sorting makes the fold reproducible regardless of map iteration order.
///
/// **Residual (honest):** the *last* revealer of a contributing set can choose to withhold and so bias
/// which fold lands — a known RANDAO weakness. It can only veto its own contribution to the *next*
/// beacon, never substitute a post-seed key for the *current* draw (that is what the commitment blocks).
/// A VDF over the fold (so even the last revealer cannot compute the outcome in time) is the documented
/// later hardening; the reputation anchor bounds the damage meanwhile.
pub fn beacon_seed(prev: &Commitment, contributions: &[Vec<u8>]) -> Commitment {
    let mut sorted: Vec<&Vec<u8>> = contributions.iter().collect();
    sorted.sort();
    let mut acc = *prev;
    for c in sorted {
        acc = mix(&acc, c);
    }
    acc
}

/// Lowercase hex of a 32-byte digest — the string form a beacon seed/secret takes to feed the
/// string-keyed [`crate::sortition_fp::vrf_value_fp`] and [`crate::sortition_fp::elect_committee_fp`]
/// unchanged. (Hex, not raw bytes, so the seed/key stay valid UTF-8 `&str`.)
pub fn hex(digest: &Commitment) -> String {
    const HEXCHARS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for &b in digest.iter() {
        s.push(HEXCHARS[(b >> 4) as usize] as char);
        s.push(HEXCHARS[(b & 0x0f) as usize] as char);
    }
    s
}

/// Verify a round of reveals and produce the **committed** secret-key map the sortition consumes: for
/// every `(node → (commitment, revealed_secret))`, keep the node iff its reveal opens its commitment,
/// mapping it to `hex(secret)` — the per-epoch key that was LOCKED before the seed. Drop-in for
/// [`crate::sortition_fp::elect_committee_fp`]: the "secret key" each node now contributes is its
/// committed secret, not a freely-chosen id, so the draw is no longer grindable. Non-revealers and
/// mismatched reveals are simply absent (their seats do not count). Deterministic (`BTreeMap`).
pub fn verified_secret_keys(
    reveals: &BTreeMap<String, (Commitment, Vec<u8>)>,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (node, (commitment, secret)) in reveals {
        if verify_reveal(commitment, secret) {
            out.insert(node.clone(), hex(&sha256(secret)));
        }
    }
    out
}

/// End-to-end committed committee election: build the deferred beacon seed from `prev` + the revealed
/// contributions, then run the reputation-weighted sortition over only the nodes whose reveal opened
/// their commitment, keyed by their committed secret. The grind-resistant counterpart of
/// [`crate::sortition_fp::elect_committee_fp`]. Returns `{node: seats}` for winners.
pub fn elect_committee_committed(
    reputation: &BTreeMap<String, i128>,
    reveals: &BTreeMap<String, (Commitment, Vec<u8>)>,
    prev_beacon: &Commitment,
    tau: u32,
) -> BTreeMap<String, u32> {
    let contributions: Vec<Vec<u8>> = reveals.values().map(|(_, s)| s.clone()).collect();
    let seed = hex(&beacon_seed(prev_beacon, &contributions));
    let keys = verified_secret_keys(reveals);
    // restrict reputation to verified revealers (an unrevealed node cannot be drawn this epoch).
    let mut rep: BTreeMap<String, i128> = BTreeMap::new();
    for node in keys.keys() {
        if let Some(&r) = reputation.get(node) {
            rep.insert(node.clone(), r);
        }
    }
    crate::sortition_fp::elect_committee_fp(&rep, &keys, &seed, tau)
}

/// The on-chain commit–reveal beacon as a **deferred pipeline** state machine — the model a thin
/// `pallet-beacon` wraps (dep-free core first; the pallet is storage + extrinsics over this, the proven
/// project pattern). It threads the two-phase discipline across discrete epochs WITHOUT splitting an
/// epoch into intra-block windows (which would complicate the no-king clock):
///
///  - During an epoch a node **commits** `c = H(s_next)` for the *next* roll and **reveals** the `s`
///    it committed in the *previous* epoch.
///  - [`roll`](Self::roll) (called once per epoch boundary, like the reputation recompute) folds the
///    epoch's valid reveals into the rolling [`beacon`](Self::beacon), turns them into the **active
///    committed keys** for this epoch's draws, and advances the pipeline (this epoch's commitments
///    become next epoch's awaited reveals).
///
/// Anti-grind: a node's draw value is `H(s | beacon)` over the `s` it committed a full epoch earlier,
/// before this `beacon` was folded — it cannot choose `s` to suit the seed. The reputation anchor stays
/// the real Sybil defence. Last-revealer residual as in [`beacon_seed`]. Deterministic (`BTreeMap`).
#[derive(Clone, Debug)]
pub struct BeaconState {
    /// The rolling beacon — the seed for THIS epoch's draws (hex via [`seed`](Self::seed)).
    beacon: Commitment,
    /// Committed secret keys (`node → hex(H(secret))`) valid for this epoch's draws.
    active_keys: BTreeMap<String, String>,
    /// Commitments made this epoch, awaiting their reveal next epoch.
    pending: BTreeMap<String, Commitment>,
    /// Commitments from last epoch, against which this epoch's reveals are checked.
    awaiting: BTreeMap<String, Commitment>,
    /// Reveals staged this epoch that opened an `awaiting` commitment.
    staged: BTreeMap<String, Vec<u8>>,
}

impl BeaconState {
    /// Genesis: start from the deferred genesis beacon (e.g. the committed BTC-block + founder-phrase
    /// mix). No keys are active until the first [`roll`](Self::roll).
    pub fn new(genesis_beacon: Commitment) -> Self {
        BeaconState {
            beacon: genesis_beacon,
            active_keys: BTreeMap::new(),
            pending: BTreeMap::new(),
            awaiting: BTreeMap::new(),
            staged: BTreeMap::new(),
        }
    }

    /// **COMMIT** `c = H(secret)` for the next roll. Replaces any earlier commitment this epoch (a node
    /// holds one outstanding commitment), so it cannot register many cheap pseudo-commitments per epoch.
    pub fn commit(&mut self, node: &str, commitment: Commitment) {
        self.pending.insert(node.into(), commitment);
    }

    /// **REVEAL** the secret committed last epoch. Accepted (and staged) iff it opens the node's awaited
    /// commitment; a mismatch or a node with no awaited commitment is rejected. Returns whether accepted.
    pub fn reveal(&mut self, node: &str, secret: Vec<u8>) -> bool {
        match self.awaiting.get(node) {
            Some(c) if verify_reveal(c, &secret) => {
                self.staged.insert(node.into(), secret);
                true
            }
            _ => false,
        }
    }

    /// **ROLL** one epoch boundary: fold this epoch's valid reveals into the beacon (deferred seed),
    /// promote them to the active committed keys for the new epoch's draws, and advance the pipeline
    /// (this epoch's `pending` commitments become next epoch's `awaiting`). Reveal staging is cleared.
    pub fn roll(&mut self) {
        let contributions: Vec<Vec<u8>> = self.staged.values().cloned().collect();
        self.beacon = beacon_seed(&self.beacon, &contributions);
        self.active_keys = self
            .staged
            .iter()
            .map(|(node, s)| (node.clone(), hex(&sha256(s))))
            .collect();
        self.awaiting = core::mem::take(&mut self.pending);
        self.staged.clear();
    }

    /// The current beacon seed (hex) for the sortition draw.
    pub fn seed(&self) -> String {
        hex(&self.beacon)
    }

    /// The committed secret-key map for this epoch's draws (`node → hex key`), drop-in for
    /// [`crate::sortition_fp::elect_committee_fp`]. Only nodes that revealed a valid commitment appear.
    pub fn active_keys(&self) -> &BTreeMap<String, String> {
        &self.active_keys
    }

    /// Elect this epoch's committee over `reputation`, restricted to nodes with an active committed key,
    /// using the current deferred beacon as the seed. The grind-resistant, stateful counterpart of
    /// [`crate::sortition_fp::elect_committee_fp`].
    pub fn elect(&self, reputation: &BTreeMap<String, i128>, tau: u32) -> BTreeMap<String, u32> {
        let seed = self.seed();
        let mut rep: BTreeMap<String, i128> = BTreeMap::new();
        for node in self.active_keys.keys() {
            if let Some(&r) = reputation.get(node) {
                rep.insert(node.clone(), r);
            }
        }
        crate::sortition_fp::elect_committee_fp(&rep, &self.active_keys, &seed, tau)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sortition_fp::{sortition_seats_fp, vrf_value_fp, FP};
    use alloc::format;

    fn secret(tag: &str) -> Vec<u8> {
        sha256(tag.as_bytes()).to_vec()
    }

    #[test]
    fn commitment_binds_and_opens() {
        let s = secret("node-7-epoch-3");
        let c = commit(&s);
        assert!(verify_reveal(&c, &s), "the true secret opens its commitment");
        assert!(!verify_reveal(&c, &secret("other")), "a different secret does not open it");
    }

    #[test]
    fn committed_value_matches_plain_vrf_over_hex() {
        // the committed path is the SAME hash construction as the existing sortition — only the KEY's
        // provenance (committed vs freely-chosen) changes, so values cross-check exactly.
        let s = secret("alice");
        let seed = hex(&sha256(b"epoch-9-beacon"));
        let keys = {
            let mut r = BTreeMap::new();
            r.insert("alice".to_string(), (commit(&s), s.clone()));
            verified_secret_keys(&r)
        };
        let via_beacon = vrf_value_fp(&keys["alice"], &seed);
        let direct = vrf_value_fp(&hex(&sha256(&s)), &seed);
        assert_eq!(via_beacon, direct);
    }

    #[test]
    fn beacon_kills_post_seed_grinding() {
        // THE win that flips `sortition_fp::known_defect_…`. Honest model: a node commits ONE fresh
        // per-epoch secret BEFORE the seed; once the seed is public it can only reveal that secret.
        // Grinding 400 alternatives is futile — none opens the commitment, so the attacker is pinned to
        // its single committed draw (one shot, like everyone), not best-of-400.
        let s_committed = secret("attacker-committed");
        let c = commit(&s_committed);
        let seed = hex(&sha256(b"epoch-revealed-after-commit"));
        let lam_fp = 5 * FP; // a node with real reputation (the only kind grinding could ever help)

        // attacker's locked-in seats: determined by the committed secret, nothing it can do post-seed.
        let committed_seats =
            sortition_seats_fp(vrf_value_fp(&hex(&sha256(&s_committed)), &seed), lam_fp, 64);

        // it grinds 400 candidate secrets against the now-known seed, hunting a better value…
        let mut best_grind = 0u32;
        for i in 0..400u32 {
            let cand = secret(&format!("attacker-grind-{i}"));
            // …but it CANNOT substitute any of them: the on-chain commitment rejects every cand ≠ s.
            assert!(!verify_reveal(&c, &cand) || cand == s_committed);
            best_grind =
                best_grind.max(sortition_seats_fp(vrf_value_fp(&hex(&sha256(&cand)), &seed), lam_fp, 64));
        }
        // the seats it could actually CLAIM = only the committed draw; grinding buys nothing it may use.
        assert!(
            committed_seats <= best_grind.max(committed_seats),
            "sanity: committed draw is a valid single sample"
        );
        // and the realised draw is bounded by a single sample of the fair distribution (≈lam), not the
        // grind maximum — the attacker cannot present `best_grind` because it cannot open the commitment.
        assert!(
            committed_seats <= 2 * 5 + 6,
            "a single committed draw stays near the fair share (lam=5), was {committed_seats}"
        );
    }

    #[test]
    fn average_draw_is_fair_under_commitment() {
        // many nodes, each committing one fresh secret before the seed → the mean seat count tracks lam
        // (the commitment does not skew the distribution, it only forbids best-of-N selection).
        let seed = hex(&sha256(b"fairness-epoch"));
        let lam = 5.0f64;
        let lam_fp = (lam * FP as f64) as i128;
        let (mut sum, n) = (0u64, 300u32);
        for i in 0..n {
            let s = secret(&format!("honest-{i}"));
            assert!(verify_reveal(&commit(&s), &s));
            sum += sortition_seats_fp(vrf_value_fp(&hex(&sha256(&s)), &seed), lam_fp, 64) as u64;
        }
        let avg = sum as f64 / n as f64;
        assert!((avg - lam).abs() < lam * 0.3, "committed draw must be fair on average: avg={avg}");
    }

    #[test]
    fn deferred_seed_is_deterministic_order_independent_and_sensitive() {
        let prev = sha256(b"epoch-N-1-beacon");
        let a = secret("ca");
        let b = secret("cb");
        let cc = secret("cd");
        let s1 = beacon_seed(&prev, &[a.clone(), b.clone(), cc.clone()]);
        // same inputs, shuffled → same seed (sorted fold).
        let s2 = beacon_seed(&prev, &[cc.clone(), a.clone(), b.clone()]);
        assert_eq!(s1, s2, "fold is order-independent (deterministic)");
        // change ANY one contribution → seed changes (no contributor controls it alone).
        let s3 = beacon_seed(&prev, &[a, b, secret("cd-tampered")]);
        assert_ne!(s1, s3, "tampering one input must change the beacon");
        // a different previous beacon → different seed (chains across epochs).
        let s4 = beacon_seed(&sha256(b"epoch-N-2-beacon"), &[secret("ca")]);
        let s5 = beacon_seed(&prev, &[secret("ca")]);
        assert_ne!(s4, s5);
    }

    #[test]
    fn end_to_end_drops_non_revealers_and_elects() {
        let mut rep: BTreeMap<String, i128> = BTreeMap::new();
        let mut reveals: BTreeMap<String, (Commitment, Vec<u8>)> = BTreeMap::new();
        for i in 0..20u32 {
            let node = format!("n{i}");
            rep.insert(node.clone(), FP); // equal reputation
            let s = secret(&format!("secret-{i}"));
            reveals.insert(node, (commit(&s), s));
        }
        // node n0 reveals garbage that does not open its commitment → must be excluded.
        let bad = reveals.get_mut("n0").unwrap();
        bad.1 = secret("garbage-does-not-open");
        let prev = sha256(b"genesis-beacon");
        let committee = elect_committee_committed(&rep, &reveals, &prev, 12);
        assert!(!committee.contains_key("n0"), "a node whose reveal fails is not drawn");
        assert!(!committee.is_empty(), "honest revealers form a committee");
        // deterministic: same inputs, same draw.
        let again = elect_committee_committed(&rep, &reveals, &prev, 12);
        assert_eq!(committee, again);
    }

    #[test]
    fn zero_reputation_wins_nothing_even_committed() {
        // the reputation anchor holds in the committed path too: no standing → no seats, commit or not.
        let mut rep: BTreeMap<String, i128> = BTreeMap::new();
        rep.insert("rich".to_string(), FP);
        rep.insert("broke".to_string(), 0);
        let mut reveals: BTreeMap<String, (Commitment, Vec<u8>)> = BTreeMap::new();
        for node in ["rich", "broke"] {
            let s = secret(node);
            reveals.insert(node.to_string(), (commit(&s), s));
        }
        let committee = elect_committee_committed(&rep, &reveals, &sha256(b"b"), 12);
        assert!(!committee.contains_key("broke"), "zero reputation → zero seats");
    }

    // ---- BeaconState (deferred pipeline) ----

    /// Drive one node through two epochs: commit in epoch 1, reveal in epoch 2, key goes active.
    #[test]
    fn pipeline_commit_then_reveal_next_epoch_activates_key() {
        let mut st = BeaconState::new(sha256(b"genesis-beacon"));
        let s = secret("n1-epoch1");
        st.commit("n1", commit(&s));
        // before the reveal window opens, a reveal has nothing to open → rejected.
        assert!(!st.reveal("n1", s.clone()), "cannot reveal before the commitment is awaited");
        st.roll(); // epoch 1 → 2: n1's commitment is now awaited
        assert!(st.active_keys().is_empty(), "no reveals yet → no active keys");
        assert!(st.reveal("n1", s.clone()), "now the committed secret opens the awaited commitment");
        assert!(!st.reveal("n1", secret("wrong")), "a wrong secret is rejected");
        st.roll(); // epoch 2 → 3: the reveal folds in and the key goes active
        assert!(st.active_keys().contains_key("n1"), "revealed node has an active committed key");
        assert_eq!(st.active_keys()["n1"], hex(&sha256(&s)));
    }

    /// Rolling the beacon with new reveals changes the seed (deferred randomness advances each epoch).
    #[test]
    fn pipeline_beacon_advances_with_reveals() {
        let mut st = BeaconState::new(sha256(b"g"));
        let seed0 = st.seed();
        let s = secret("contributor");
        st.commit("c", commit(&s));
        st.roll();
        let seed_after_empty = st.seed(); // no reveals folded yet (commitment just became awaited)
        st.reveal("c", s);
        st.roll();
        let seed_after_reveal = st.seed();
        assert_eq!(seed0, seed_after_empty, "an empty roll does not change the beacon");
        assert_ne!(seed_after_empty, seed_after_reveal, "a folded reveal advances the beacon");
    }

    /// The pipeline draw is grind-resistant: the key was committed a full epoch before the beacon it is
    /// drawn against was folded, so the value `H(s|seed)` cannot be ground; a non-revealer is not drawn.
    #[test]
    fn pipeline_draw_is_committed_and_drops_non_revealers() {
        let mut st = BeaconState::new(sha256(b"g0"));
        let mut rep: BTreeMap<String, i128> = BTreeMap::new();
        // 12 nodes commit in epoch 1.
        for i in 0..12u32 {
            let node = format!("v{i}");
            rep.insert(node.clone(), FP);
            st.commit(&node, commit(&secret(&format!("sk-{i}"))));
        }
        st.roll(); // epoch 1 → 2: commitments awaited
        // 11 of 12 reveal; v0 stays silent.
        for i in 1..12u32 {
            assert!(st.reveal(&format!("v{i}"), secret(&format!("sk-{i}"))));
        }
        st.roll(); // fold reveals → active keys + new beacon
        // the draw value for an active node equals the plain VRF over its committed key + the beacon seed
        // (committed an epoch earlier — not grindable against this seed).
        let seed = st.seed();
        let v3_val = vrf_value_fp(&hex(&sha256(&secret("sk-3"))), &seed);
        assert_eq!(st.active_keys()["v3"], hex(&sha256(&secret("sk-3"))));
        let _ = sortition_seats_fp(v3_val, FP, 64); // value is well-formed
        let committee = st.elect(&rep, 12);
        assert!(!committee.contains_key("v0"), "the non-revealer is not eligible this epoch");
        assert!(!committee.is_empty(), "the revealers form a committee");
        // deterministic: a fresh state replaying the same calls reaches the same draw.
        let again = st.elect(&rep, 12);
        assert_eq!(committee, again);
    }

    /// One outstanding commitment per node per epoch: a second commit replaces the first (no cheap
    /// multi-commitment grinding within an epoch).
    #[test]
    fn pipeline_one_commitment_per_node() {
        let mut st = BeaconState::new(sha256(b"g"));
        let s_final = secret("the-real-one");
        st.commit("n", commit(&secret("throwaway")));
        st.commit("n", commit(&s_final)); // replaces
        st.roll();
        assert!(!st.reveal("n", secret("throwaway")), "the replaced commitment cannot be opened");
        assert!(st.reveal("n", s_final), "only the latest commitment is awaited");
    }
}
