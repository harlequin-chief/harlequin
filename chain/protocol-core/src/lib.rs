//! Harlequin epoch state machine — the living chain that ties the validated pieces together.
//!
//! It composes two engines, each ported and cross-validated on its own:
//! - `reputation-core` (deterministic fixed-point EigenTrust-with-anti-collusion) → WHO is trusted,
//!   per suit, recomputed every epoch from the evidence + the vouch graph.
//! - `consensus-core` (reputation-weighted VRF sortition) → WHO runs consensus this epoch, drawn in
//!   proportion to reputation so a Sybil at reputation ~0 wins ~0 seats (Art. VI).
//!
//! And it emits, as a first-class output of every epoch, the **telemetry the nodes publish**
//! (`EpochReport`): node count, committee, reputation distribution per suit, and — deliberately — a
//! Gini coefficient of reputation. Harlequin fights concentrated power; the network must be able to
//! watch whether it is *itself* concentrating. The panel is served by the nodes (SPEC §5c), so there
//! is no central dashboard a State can switch off.
//!
//! Status: this is the host/reference state machine. Reputation already runs on the deterministic
//! fixed-point path; the sortition (consensus-core) is still f64 and its fixed-point port — like the
//! vouch-scoring port — is the step that makes the whole machine `no_std` for the pallet. Until then
//! `advance_epoch` is the integration oracle and the source of the telemetry schema.

use std::collections::BTreeMap;

use consensus_core::elect_committee_fp;
use consensus_core::sha256::sha256;
use consensus_core::sim::{run_once, Adversary, Population, SplitMix64};
use consensus_core::snowball::SnowballParams;
use reputation_core::{
    reputation_dimension_fully_fixed_fp, Agent, Params, TrustGraph, DIMENSIONS, FP_SCALE,
};

/// The chain state across epochs.
pub struct Protocol {
    agents: Vec<Agent>,
    graph: TrustGraph,
    /// node -> VRF secret key. SIMULATION ONLY: derived deterministically as `sk-<id>` so a run is
    /// reproducible. On a real node each member holds its own key and never shares it.
    secret_keys: BTreeMap<String, String>,
    params: Params,
    /// expected total committee seats per epoch (sortition intensity, SPEC §2.2).
    tau: f64,
    epoch: u64,
}

/// The telemetry a node publishes after each epoch (SPEC §5c). Plain data: any node can recompute and
/// serve it, so the public panel has no single point a State can cut.
#[derive(Clone, Debug, PartialEq)]
pub struct EpochReport {
    pub epoch: u64,
    pub seed: String,
    /// members with strictly positive consensus reputation (the conservative min across suits).
    pub active_nodes: usize,
    /// total committee seats this epoch.
    pub committee_size: u32,
    /// elected committee: node -> seats.
    pub committee: BTreeMap<String, u32>,
    /// sum of consensus reputation over all members.
    pub total_reputation: f64,
    /// reputation summed per suit (♦ commerce, ♣ technical, ♠ judicial, ♥ governance).
    pub reputation_by_suit: BTreeMap<String, f64>,
    /// the single highest consensus reputation (entrenchment watch).
    pub top_reputation: f64,
    /// Gini of consensus reputation among ACTIVE members, in [0,1]: 0 = perfectly even, →1 = one node
    /// holds it all. The network's own "is an elite forming?" alarm (Sybils excluded so they can't
    /// skew it).
    pub gini: f64,
    /// members present but with ~0 reputation and 0 seats (Sybils / freeloaders kept out).
    pub excluded: usize,
    /// Liveness check of the REAL elected committee: did the honest committee reach finality on the
    /// legitimate value this epoch? (Snowball voting over the committee, weighted by seats, no
    /// adversary — the chain's own "can this committee decide?" signal, served in the telemetry.)
    pub finalized: bool,
}

impl Protocol {
    /// Start the chain from a genesis cohort (SPEC §1.4): the founding members that seed pre-trust.
    /// Each should be marked `.genesis` and carry the evidence that anchors the bootstrap.
    pub fn genesis(cohort: Vec<Agent>, params: Params, tau: f64) -> Self {
        let mut p = Protocol {
            agents: Vec::new(),
            graph: TrustGraph::new(),
            secret_keys: BTreeMap::new(),
            params,
            tau,
            epoch: 0,
        };
        for a in cohort {
            p.register(a);
        }
        p
    }

    fn register(&mut self, agent: Agent) {
        let id = agent.id.clone();
        self.secret_keys.entry(id.clone()).or_insert_with(|| format!("sk-{id}"));
        self.agents.push(agent);
    }

    /// Admit a new member (joins from the next epoch). Reputation is earned, not granted: a fresh
    /// member with no evidence and no vouches stays at ~0 until the network backs it.
    pub fn admit(&mut self, agent: Agent) {
        self.register(agent);
    }

    /// `source` vouches for `target` in `dim` with `weight` (§1.3b). The edge feeds the next recompute.
    pub fn attest(&mut self, source: &str, target: &str, dim: &str, weight: f64) {
        self.graph.attest(source, target, dim, weight);
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn member_count(&self) -> usize {
        self.agents.len()
    }

    /// Reputation vector per member for the current state: `node -> {suit -> reputation}`, each suit on
    /// the deterministic fixed-point path (reproducible across machines).
    pub fn reputation_vector(&self) -> BTreeMap<String, BTreeMap<String, f64>> {
        let scale = self.params.scale / FP_SCALE as f64;
        let per_dim = self.reputation_vector_fp();
        let mut out: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        for a in &self.agents {
            let v: BTreeMap<String, f64> =
                DIMENSIONS.iter().map(|d| (d.to_string(), per_dim[*d][&a.id] as f64 * scale)).collect();
            out.insert(a.id.clone(), v);
        }
        out
    }

    /// Per-suit reputation in RAW fixed-point (i128) — the exact on-chain values. `suit -> {node -> r}`.
    fn reputation_vector_fp(&self) -> BTreeMap<&'static str, BTreeMap<String, i128>> {
        let mut per_dim: BTreeMap<&'static str, BTreeMap<String, i128>> = BTreeMap::new();
        for d in DIMENSIONS {
            per_dim.insert(
                d,
                reputation_dimension_fully_fixed_fp(&self.agents, &self.graph, d, &self.params),
            );
        }
        per_dim
    }

    /// Advance one epoch: recompute reputation, elect the committee by reputation-weighted sortition,
    /// and return the telemetry. `beacon` is the epoch randomness (a public unbiasable beacon on a real
    /// chain); folding the epoch number in rotates the committee (Art. VI, anti-entrenchment).
    pub fn advance_epoch(&mut self, beacon: &str) -> EpochReport {
        self.epoch += 1;
        let seed = format!("{beacon}|epoch{}", self.epoch);

        // Raw fixed-point reputation per suit — the exact values the chain computes.
        let per_dim = self.reputation_vector_fp();
        let scale = self.params.scale / FP_SCALE as f64;

        // Consensus reputation = conservative MIN across suits (§1.2b): authority that needs global
        // reliability cannot be bought in one suit. A high ♦ does not buy a missing ♠. Kept in raw
        // fixed-point so the committee is elected by the deterministic on-chain sortition.
        let mut scalar_fp: BTreeMap<String, i128> = BTreeMap::new();
        let mut reputation_by_suit: BTreeMap<String, f64> =
            DIMENSIONS.iter().map(|d| (d.to_string(), 0.0)).collect();
        for a in &self.agents {
            let mut min_fp = i128::MAX;
            for d in DIMENSIONS {
                let r = per_dim[d][&a.id];
                if r < min_fp {
                    min_fp = r;
                }
                *reputation_by_suit.get_mut(d).unwrap() += r as f64 * scale;
            }
            scalar_fp.insert(a.id.clone(), min_fp.max(0));
        }

        // Committee by the deterministic fixed-point sortition — the exact path the runtime runs.
        let committee = elect_committee_fp(&scalar_fp, &self.secret_keys, &seed, self.tau.round() as u32);
        let committee_size: u32 = committee.values().sum();

        // Liveness sanity: run the Snowball voting over the REAL elected committee (members weighted by
        // their seats, all honest) and check it reaches finality. The adversarial analysis lives in
        // `consensus-core::sim`; here we confirm the committee the chain actually elected can decide.
        let finalized = if committee.is_empty() {
            false
        } else {
            let reputation: Vec<f64> = committee.values().map(|&s| s as f64).collect();
            let adversary = vec![false; committee.len()];
            let pop = Population { reputation, adversary };
            let digest = sha256(seed.as_bytes());
            let mut rng_seed = 0u64;
            for &b in &digest[..8] {
                rng_seed = (rng_seed << 8) | b as u64;
            }
            let mut rng = SplitMix64::new(rng_seed);
            let outcome = run_once(
                &pop,
                &SnowballParams::default(),
                &mut rng,
                true,
                Adversary::Fixed,
                0.0,
            );
            outcome.safe
        };

        let values_fp: Vec<i128> = scalar_fp.values().cloned().collect();
        let total_reputation: f64 = values_fp.iter().map(|&r| r as f64 * scale).sum();
        let top_reputation = values_fp.iter().cloned().max().unwrap_or(0) as f64 * scale;
        // Gini is measured over the ACTIVE members only (reputation > 0). Otherwise a swarm of
        // powerless Sybils would inflate it toward 1 and hide whether power is actually concentrating
        // among those who hold it. This is the "is an elite forming?" alarm, not a head-count of zeros.
        let mut active: Vec<f64> =
            values_fp.iter().filter(|&&r| r > 0).map(|&r| r as f64 * scale).collect();
        let active_nodes = active.len();
        let excluded = values_fp.len() - active_nodes;
        let gini = gini_coefficient(&mut active);

        EpochReport {
            epoch: self.epoch,
            seed,
            active_nodes,
            committee_size,
            committee,
            total_reputation,
            reputation_by_suit,
            top_reputation,
            gini,
            excluded,
            finalized,
        }
    }
}

/// Gini coefficient of a set of non-negative values, in [0,1]. 0 = perfectly even; →1 = all mass on
/// one holder. `gini = Σ_i Σ_j |x_i - x_j| / (2 n Σ x)`. Empty or all-zero → 0.
fn gini_coefficient(values: &mut [f64]) -> f64 {
    let n = values.len();
    if n == 0 {
        return 0.0;
    }
    let sum: f64 = values.iter().sum();
    if sum <= 0.0 {
        return 0.0;
    }
    let mut abs_diffs = 0.0;
    for i in 0..n {
        for j in 0..n {
            abs_diffs += (values[i] - values[j]).abs();
        }
    }
    abs_diffs / (2.0 * n as f64 * sum)
}

impl EpochReport {
    /// Hand-rolled JSON (no serde dependency — nothing extra pulled onto the isolated station). This is
    /// the wire shape a node serves to the public panel.
    pub fn to_json(&self) -> String {
        let mut committee = String::from("{");
        for (i, (node, seats)) in self.committee.iter().enumerate() {
            if i > 0 {
                committee.push(',');
            }
            committee.push_str(&format!("\"{}\":{}", json_escape(node), seats));
        }
        committee.push('}');

        let mut suits = String::from("{");
        for (i, (suit, r)) in self.reputation_by_suit.iter().enumerate() {
            if i > 0 {
                suits.push(',');
            }
            suits.push_str(&format!("\"{}\":{:.4}", json_escape(suit), r));
        }
        suits.push('}');

        format!(
            "{{\"epoch\":{},\"seed\":\"{}\",\"active_nodes\":{},\"committee_size\":{},\
\"committee\":{},\"total_reputation\":{:.4},\"reputation_by_suit\":{},\
\"top_reputation\":{:.4},\"gini\":{:.6},\"excluded\":{},\"finalized\":{}}}",
            self.epoch,
            json_escape(&self.seed),
            self.active_nodes,
            self.committee_size,
            committee,
            self.total_reputation,
            suits,
            self.top_reputation,
            self.gini,
            self.excluded,
            self.finalized,
        )
    }
}

/// Escape a string for embedding in a JSON string literal. Node ids are member-chosen pseudonyms
/// (Art. VII) and `seed` carries an external beacon, so this must handle the FULL set the JSON spec
/// requires — not just `"` and `\`, but every control char U+0000..=U+001F — or a hostile id with a
/// raw newline/control char would emit invalid JSON to the public telemetry panel (broken dashboard,
/// or an injection vector). Dependency-free (no serde on the isolated station).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A founder trusted across all four suits (multi-suit evidence), so the conservative MIN is > 0
    /// and they are consensus-eligible.
    fn founder(id: &str) -> Agent {
        let mut a = Agent::new(id).genesis();
        for d in DIMENSIONS {
            a = a.with_evidence(d, 5.0);
        }
        a
    }

    fn base_params() -> Params {
        Params { community: true, in_concentration: true, ..Default::default() }
    }

    #[test]
    fn genesis_then_epoch_elects_a_committee() {
        let cohort: Vec<Agent> = (0..40).map(|i| founder(&format!("g{i}"))).collect();
        let mut p = Protocol::genesis(cohort, base_params(), 30.0);
        assert_eq!(p.epoch(), 0);
        let r = p.advance_epoch("beacon");
        assert_eq!(r.epoch, 1);
        assert!(r.active_nodes == 40, "all founders are active, got {}", r.active_nodes);
        assert!(r.committee_size > 0, "a committee must be elected, got {}", r.committee_size);
        // sortition is reputation-weighted around tau=30
        assert!(r.committee_size <= 40, "committee can't exceed members, got {}", r.committee_size);
        // the honest elected committee reaches finality (liveness of the real committee)
        assert!(r.finalized, "the honest committee should finalise");
    }

    #[test]
    fn epoch_report_serialises_finalized() {
        let cohort: Vec<Agent> = (0..40).map(|i| founder(&format!("g{i}"))).collect();
        let mut p = Protocol::genesis(cohort, base_params(), 30.0);
        let r = p.advance_epoch("beacon");
        assert!(r.finalized);
        assert!(r.to_json().contains("\"finalized\":true"), "telemetry must carry the finality flag");
    }

    #[test]
    fn sybils_without_evidence_or_vouches_are_excluded() {
        let cohort: Vec<Agent> = (0..30).map(|i| founder(&format!("g{i}"))).collect();
        let mut p = Protocol::genesis(cohort, base_params(), 20.0);
        // 500 sybils with no evidence and no vouches
        for i in 0..500 {
            p.admit(Agent::new(&format!("s{i}")));
        }
        let r = p.advance_epoch("beacon");
        let sybils_in_committee = r.committee.keys().filter(|n| n.starts_with('s')).count();
        assert_eq!(sybils_in_committee, 0, "sybils must not enter the committee");
        assert!(r.excluded >= 500, "the 500 sybils sit at ~0 reputation, excluded={}", r.excluded);
        assert_eq!(r.active_nodes, 30, "only the founders are active");
    }

    #[test]
    fn committee_rotates_across_epochs() {
        let cohort: Vec<Agent> = (0..60).map(|i| founder(&format!("g{i}"))).collect();
        let mut p = Protocol::genesis(cohort, base_params(), 40.0);
        let r0 = p.advance_epoch("beacon");
        let r1 = p.advance_epoch("beacon");
        let c0: std::collections::HashSet<&String> = r0.committee.keys().collect();
        let c1: std::collections::HashSet<&String> = r1.committee.keys().collect();
        let overlap = c0.intersection(&c1).count() as f64 / c0.union(&c1).count().max(1) as f64;
        assert!(overlap < 0.85, "committees should rotate epoch to epoch, overlap {overlap}");
    }

    #[test]
    fn gini_rises_with_concentration() {
        // even cohort: low Gini. then make one founder dominate the trust graph: Gini climbs.
        let cohort: Vec<Agent> = (0..30).map(|i| founder(&format!("g{i}"))).collect();
        let mut p = Protocol::genesis(cohort, base_params(), 20.0);
        let even = p.advance_epoch("beacon");
        for i in 1..30 {
            for d in DIMENSIONS {
                p.attest(&format!("g{i}"), "g0", d, 1.0);
            }
        }
        let skewed = p.advance_epoch("beacon");
        assert!(
            skewed.gini > even.gini,
            "funnelling trust onto one node must raise Gini: {} -> {}",
            even.gini,
            skewed.gini
        );
    }

    #[test]
    fn telemetry_json_has_the_schema() {
        let cohort: Vec<Agent> = (0..10).map(|i| founder(&format!("g{i}"))).collect();
        let mut p = Protocol::genesis(cohort, base_params(), 8.0);
        let r = p.advance_epoch("beacon");
        let j = r.to_json();
        for key in [
            "\"epoch\":1",
            "\"active_nodes\":10",
            "\"committee_size\":",
            "\"reputation_by_suit\":",
            "\"gini\":",
            "\"commerce\":",
        ] {
            assert!(j.contains(key), "telemetry json missing {key}: {j}");
        }
    }

    #[test]
    fn json_escape_neutralises_hostile_member_ids() {
        // A node id is a member-chosen pseudonym (Art. VII) that ends up as a JSON string key in the
        // public telemetry. The escaper must neutralise EVERY dangerous character class — quote,
        // backslash and the full control range U+0000..=U+001F — or a hostile id breaks/injects the panel.
        let hostile = "ev\"il\\back\nline\ttab\u{01}\u{1f}end";
        let e = json_escape(hostile);
        // no raw dangerous byte survives: the escaped form is pure printable ASCII+ (>= 0x20).
        for b in e.bytes() {
            assert!(b >= 0x20, "json_escape left a raw control byte {b:#x} in {e:?}");
        }
        // each class is represented by its proper escape sequence.
        for needle in ["\\\"", "\\\\", "\\n", "\\t", "\\u0001", "\\u001f"] {
            assert!(e.contains(needle), "missing escape {needle:?} in {e:?}");
        }
        // a benign id is left untouched.
        assert_eq!(json_escape("alice_42"), "alice_42");
    }

    #[test]
    fn funnelled_concentration_does_not_reach_the_sortition_collapse_band() {
        // GOOD NEWS half of the tick-2 follow-up (campaña estrés tick 4): trying to MANUFACTURE a
        // dominant node by funnelling trust onto g0 in every suit does NOT push its lam into the
        // collapse band — the in-concentration + community damping caps what one node accrues from
        // vouches, so Gini stays moderate and g0 still rotates (its seat count varies across beacons).
        let cohort: Vec<Agent> = (0..12).map(|i| founder(&format!("g{i}"))).collect();
        let mut p = Protocol::genesis(cohort, base_params(), 40.0);
        for i in 1..12 {
            for d in DIMENSIONS {
                p.attest(&format!("g{i}"), "g0", d, 5.0);
            }
        }
        let mut g0_seats = std::collections::HashSet::new();
        let mut max_gini = 0.0f64;
        for b in ["beacon-A", "beacon-B", "beacon-C", "beacon-D", "beacon-E"] {
            let r = p.advance_epoch(b);
            g0_seats.insert(*r.committee.get("g0").unwrap_or(&0));
            max_gini = max_gini.max(r.gini);
        }
        assert!(max_gini < 0.6, "funnelling should not let one node dominate; Gini={max_gini}");
        assert!(g0_seats.len() > 1, "g0 still rotates (varying seats across beacons): {g0_seats:?}");
    }

    #[test]
    fn dominant_node_rotates_instead_of_pinning_at_the_cap() {
        // FIXED at the epoch level (campaña estrés tick 11, mode-anchored Poisson). A node that genuinely
        // dominates evidence across all suits — with its expected seats lam HIGH but below the seat cap —
        // used to win the cap (64) on EVERY beacon (the collapse, tick 4): zero rotation, defeating Art.
        // VI. Now its seat count is Poisson-distributed and VARIES beacon to beacon (rotation restored),
        // while the Gini alarm still flags the concentration on the telemetry. tau kept modest so lam
        // stays under the 64 cap (above it, capping every beacon would be legitimate, not a defect).
        let mut cohort: Vec<Agent> = vec![];
        let mut g0 = Agent::new("g0").genesis();
        for d in DIMENSIONS {
            g0 = g0.with_evidence(d, 200.0);
        }
        cohort.push(g0);
        for i in 1..12 {
            let mut a = Agent::new(&format!("g{i}")).genesis();
            for d in DIMENSIONS {
                a = a.with_evidence(d, 1.0);
            }
            cohort.push(a);
        }
        let mut p = Protocol::genesis(cohort, base_params(), 30.0);
        let mut g0_seats = std::collections::HashSet::new();
        let mut max_gini = 0.0f64;
        for b in ["beacon-A", "beacon-B", "beacon-C", "beacon-D", "beacon-E"] {
            let r = p.advance_epoch(b);
            g0_seats.insert(*r.committee.get("g0").unwrap_or(&0));
            max_gini = max_gini.max(r.gini);
        }
        assert!(g0_seats.len() > 1, "fixed: a dominant node now ROTATES (varying seats), not pinned: {g0_seats:?}");
        assert!(!g0_seats.contains(&64), "a below-cap dominant node must not be pinned at the seat cap: {g0_seats:?}");
        assert!(max_gini > 0.6, "the Gini alarm still flags the concentration, was {max_gini}");
    }

    #[test]
    fn committee_is_deterministic_across_runs() {
        // the committee is now elected by the fixed-point sortition — bit-identical across runs,
        // the property a chain needs (every node must agree on the same committee).
        let build = || {
            let cohort: Vec<Agent> = (0..40).map(|i| founder(&format!("g{i}"))).collect();
            let mut p = Protocol::genesis(cohort, base_params(), 25.0);
            p.advance_epoch("beacon").committee
        };
        assert_eq!(build(), build());
    }

    #[test]
    fn reputation_is_deterministic_across_runs() {
        // same genesis + same edges -> identical reputation vector (the fixed-point path).
        let build = || {
            let cohort: Vec<Agent> = (0..20).map(|i| founder(&format!("g{i}"))).collect();
            let mut p = Protocol::genesis(cohort, base_params(), 12.0);
            p.attest("g1", "g0", "commerce", 1.0);
            p.reputation_vector()
        };
        assert_eq!(build(), build());
    }
}
