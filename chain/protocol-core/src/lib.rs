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

use std::collections::{BTreeMap, HashMap};

use consensus_core::elect_committee;
use reputation_core::{reputation_dimension_fully_fixed, Agent, Params, TrustGraph, DIMENSIONS};

/// The chain state across epochs.
pub struct Protocol {
    agents: Vec<Agent>,
    graph: TrustGraph,
    /// node -> VRF secret key. SIMULATION ONLY: derived deterministically as `sk-<id>` so a run is
    /// reproducible. On a real node each member holds its own key and never shares it.
    secret_keys: HashMap<String, String>,
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
}

impl Protocol {
    /// Start the chain from a genesis cohort (SPEC §1.4): the founding members that seed pre-trust.
    /// Each should be marked `.genesis()` and carry the evidence that anchors the bootstrap.
    pub fn genesis(cohort: Vec<Agent>, params: Params, tau: f64) -> Self {
        let mut p = Protocol {
            agents: Vec::new(),
            graph: TrustGraph::new(),
            secret_keys: HashMap::new(),
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
        let mut per_dim: BTreeMap<&str, BTreeMap<String, f64>> = BTreeMap::new();
        for d in DIMENSIONS {
            per_dim.insert(d, reputation_dimension_fully_fixed(&self.agents, &self.graph, d, &self.params));
        }
        let mut out: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        for a in &self.agents {
            let v: BTreeMap<String, f64> =
                DIMENSIONS.iter().map(|d| (d.to_string(), per_dim[d][&a.id])).collect();
            out.insert(a.id.clone(), v);
        }
        out
    }

    /// Advance one epoch: recompute reputation, elect the committee by reputation-weighted sortition,
    /// and return the telemetry. `beacon` is the epoch randomness (a public unbiasable beacon on a real
    /// chain); folding the epoch number in rotates the committee (Art. VI, anti-entrenchment).
    pub fn advance_epoch(&mut self, beacon: &str) -> EpochReport {
        self.epoch += 1;
        let seed = format!("{beacon}|epoch{}", self.epoch);

        let vector = self.reputation_vector();

        // Consensus reputation = conservative MIN across suits (§1.2b): authority that needs global
        // reliability cannot be bought in one suit. A high ♦ does not buy a missing ♠.
        let mut scalar: HashMap<String, f64> = HashMap::new();
        let mut reputation_by_suit: BTreeMap<String, f64> =
            DIMENSIONS.iter().map(|d| (d.to_string(), 0.0)).collect();
        for (id, v) in &vector {
            let agg = v.values().cloned().fold(f64::INFINITY, f64::min);
            scalar.insert(id.clone(), if agg.is_finite() { agg } else { 0.0 });
            for (suit, r) in v {
                *reputation_by_suit.get_mut(suit).unwrap() += r;
            }
        }

        let committee = elect_committee(&scalar, &self.secret_keys, &seed, self.tau);
        let committee: BTreeMap<String, u32> = committee.into_iter().collect();
        let committee_size: u32 = committee.values().sum();

        let values: Vec<f64> = scalar.values().cloned().collect();
        let total_reputation: f64 = values.iter().sum();
        let top_reputation = values.iter().cloned().fold(0.0_f64, f64::max);
        // Gini is measured over the ACTIVE members only (reputation > 0). Otherwise a swarm of
        // powerless Sybils would inflate it toward 1 and hide whether power is actually concentrating
        // among those who hold it. This is the "is an elite forming?" alarm, not a head-count of zeros.
        let mut active: Vec<f64> = values.into_iter().filter(|&r| r > EPS).collect();
        let active_nodes = active.len();
        let excluded = scalar.len() - active_nodes;
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
        }
    }
}

/// Reputation below this counts as zero (sortition/exclusion threshold), after the §1 `scale`.
const EPS: f64 = 1e-6;

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
\"top_reputation\":{:.4},\"gini\":{:.6},\"excluded\":{}}}",
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
        )
    }
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
