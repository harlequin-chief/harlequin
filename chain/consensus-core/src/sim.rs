//! Whole-network simulation harness — the Rust cross-validation of the voting port (`std` only).
//! Mirrors the test-rig `prototipos/consenso/wtc_sim/{consensus,population}.py` (`run_once` + the
//! populations) so the Rust `SnowballNode` can be checked against the **validated** security behaviour
//! (11/11). It is NOT consensus-critical code and never ships in the runtime — it is the reference
//! oracle, the analogue of `vrf.rs` for the voting layer.
//!
//! The RNG is a hand-rolled SplitMix64 (dependency-free; OPSEC: nothing pulled onto the isolated
//! station). It is *not* Python's Mersenne Twister, so per-run outcomes are not bit-identical — but the
//! security properties the rig asserts are **statistical** (safe% / capture% over many trials), and
//! those are RNG-independent. The tests therefore assert the same thresholds, not the same seeds.

use crate::snowball::{SnowballNode, SnowballParams};
use crate::sortition_fp::{elect_committee_fp, FP};
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Minimal deterministic PRNG (SplitMix64). Enough for weighted sampling in the harness.
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        SplitMix64(seed)
    }
    #[inline]
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform f64 in `[0, 1)` (53-bit mantissa), matching the role of Python's `random()`.
    #[inline]
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A node population: per-node reputation weight and which ids are adversarial.
pub struct Population {
    /// Reputation weight per node (index = node id).
    pub reputation: Vec<f64>,
    /// `true` at index i iff node i is adversarial.
    pub adversary: Vec<bool>,
}

/// The adversary's reporting strategy (test-rig `adversary`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Adversary {
    /// Always report colour 1 (push a conflicting value).
    Fixed,
    /// Report the minority colour among the honest, to keep them split (anti-finality).
    Adaptive,
}

/// Outcome tally among the HONEST nodes after one run (mirrors the Python dict).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub decided_0: u32,
    pub decided_1: u32,
    pub undecided: u32,
    /// All honest nodes decided the legitimate value 0.
    pub safe: bool,
    /// Some honest node decided the adversary's value.
    pub capture: bool,
    /// Both values present among honest decisions.
    pub fork: bool,
}

fn cum_weights(w: &[f64]) -> Vec<f64> {
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(w.len());
    for &x in w {
        acc += x.max(0.0);
        out.push(acc);
    }
    out
}

/// Pick one index ∝ weight, by drawing in `[0, total)` and bisecting the cumulative weights —
/// the same construction as Python's `random.choices(cum_weights=...)`.
fn weighted_pick(cum: &[f64], rng: &mut SplitMix64) -> usize {
    let total = *cum.last().unwrap_or(&0.0);
    if total <= 0.0 {
        return (rng.next_u64() as usize) % cum.len().max(1);
    }
    let target = rng.next_f64() * total;
    // bisect_right
    let (mut lo, mut hi) = (0usize, cum.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if target < cum[mid] {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    lo.min(cum.len() - 1)
}

/// Full sampling/fault configuration for a run — test-rig parity (`run_once` keyword args).
pub struct RunConfig<'a> {
    /// Sample weighted by reputation (`false` = uniform, the contrast that shows reputation protects).
    pub weighted: bool,
    pub adversary: Adversary,
    /// Probability each queried response is lost (latency / unreliable network). Liveness cost only.
    pub loss: f64,
    /// Cluster id per node, for independence-weighted sampling (PAPER §5.4). `None` = no clustering.
    pub clusters: Option<&'a [usize]>,
    /// Max nodes of a single cluster allowed in one sample. `None` = no cap.
    pub cap_cluster: Option<u32>,
    /// Group id per node, for a network partition. `None` = no partition.
    pub groups: Option<&'a [usize]>,
    /// Rounds the partition holds before the network heals.
    pub partition_rounds: u32,
    /// Anti-partition mitigation: finalise only while visible reputation / total ≥ this. `0` = off.
    pub network_quorum: f64,
}

impl Default for RunConfig<'_> {
    fn default() -> Self {
        RunConfig {
            weighted: true,
            adversary: Adversary::Fixed,
            loss: 0.0,
            clusters: None,
            cap_cluster: None,
            groups: None,
            partition_rounds: 0,
            network_quorum: 0.0,
        }
    }
}

/// One run of the network with the simple defaults (no clustering / no partition). `weighted=false`
/// samples uniformly; `loss` drops each response with that probability. Thin wrapper over
/// [`run_once_cfg`].
pub fn run_once(
    pop: &Population,
    params: &SnowballParams,
    rng: &mut SplitMix64,
    weighted: bool,
    adversary: Adversary,
    loss: f64,
) -> Outcome {
    let cfg = RunConfig { weighted, adversary, loss, ..Default::default() };
    run_once_cfg(pop, params, rng, &cfg)
}

/// One run of the network with the full config (independence cap + partition). Returns the honest-node
/// outcome tally. Mirrors the test-rig `run_once`.
pub fn run_once_cfg(
    pop: &Population,
    params: &SnowballParams,
    rng: &mut SplitMix64,
    cfg: &RunConfig,
) -> Outcome {
    let n = pop.reputation.len();
    let w = |i: usize| -> f64 {
        if cfg.weighted {
            pop.reputation[i].max(0.0)
        } else {
            1.0
        }
    };

    // Whole-network sampling pool (indices + cumulative weights).
    let all_ids: Vec<usize> = (0..n).collect();
    let all_cum = cum_weights(&all_ids.iter().map(|&i| w(i)).collect::<Vec<_>>());
    let total_rep: f64 = (0..n).map(w).sum();

    // Per-group pools (for the partition phase) + per-group reputation.
    let mut group_pool: BTreeMap<usize, (Vec<usize>, Vec<f64>)> = BTreeMap::new();
    let mut group_rep: BTreeMap<usize, f64> = BTreeMap::new();
    if let Some(groups) = cfg.groups {
        for g in groups.iter().copied().collect::<alloc::collections::BTreeSet<_>>() {
            let gids: Vec<usize> = (0..n).filter(|&i| groups[i] == g).collect();
            let gcum = cum_weights(&gids.iter().map(|&i| w(i)).collect::<Vec<_>>());
            group_rep.insert(g, gids.iter().map(|&i| w(i)).sum());
            group_pool.insert(g, (gids, gcum));
        }
    }

    let partition_active = |rnd: u32| cfg.groups.is_some() && rnd < cfg.partition_rounds;

    let honest_idx: Vec<usize> = (0..n).filter(|&i| !pop.adversary[i]).collect();
    let mut nodes: Vec<SnowballNode> = (0..n).map(|_| SnowballNode::new(0)).collect();
    let mut adv_color: u8 = 1;

    let report = |nodes: &Vec<SnowballNode>, adv_color: u8, i: usize| -> u8 {
        if pop.adversary[i] {
            adv_color
        } else {
            nodes[i].decision().unwrap_or_else(|| nodes[i].pref())
        }
    };

    for round in 0..params.max_rounds {
        if honest_idx.iter().all(|&i| nodes[i].is_decided()) {
            break;
        }
        if cfg.adversary == Adversary::Adaptive {
            let mut ones = 0u32;
            let mut zeros = 0u32;
            for &i in &honest_idx {
                match report(&nodes, adv_color, i) {
                    1 => ones += 1,
                    _ => zeros += 1,
                }
            }
            adv_color = if ones <= zeros { 1 } else { 0 };
        }
        for &node in &honest_idx {
            if nodes[node].is_decided() {
                continue;
            }
            // Choose the sampling base: the node's group while partitioned, else the whole network.
            let (base_ids, base_cum): (&[usize], &[f64]) = if partition_active(round) {
                let g = cfg.groups.unwrap()[node];
                let (ids, cum) = &group_pool[&g];
                (ids, cum)
            } else {
                (&all_ids, &all_cum)
            };

            // Draw k peers, applying the per-cluster independence cap if configured.
            let mut votes: Vec<u8> = Vec::with_capacity(params.k as usize);
            let mut per_cluster: BTreeMap<usize, u32> = BTreeMap::new();
            let mut drawn = 0u32;
            let mut attempts = 0u32;
            let limit = params.k * 40; // anti-loop bound when diversity is short
            while drawn < params.k {
                let pick = if attempts < limit {
                    attempts += 1;
                    let pos = base_pick(base_ids, base_cum, rng);
                    if let (Some(clusters), Some(cap)) = (cfg.clusters, cfg.cap_cluster) {
                        let cl = clusters[pos];
                        if *per_cluster.get(&cl).unwrap_or(&0) >= cap {
                            continue; // cluster full; redraw
                        }
                        *per_cluster.entry(cl).or_insert(0) += 1;
                    }
                    pos
                } else {
                    // Diversity exhausted: fill the rest without the cap (do not penalise liveness).
                    base_pick(base_ids, base_cum, rng)
                };
                drawn += 1;
                if cfg.loss > 0.0 && rng.next_f64() < cfg.loss {
                    continue; // response lost
                }
                votes.push(report(&nodes, adv_color, pick));
            }

            // Anti-partition guard: may finalise only while a network quorum of reputation is visible.
            let reaches_quorum = if cfg.network_quorum <= 0.0 {
                true
            } else {
                let visible = if partition_active(round) {
                    group_rep[&cfg.groups.unwrap()[node]]
                } else {
                    total_rep
                };
                total_rep > 0.0 && visible / total_rep >= cfg.network_quorum
            };
            nodes[node].observe_round(&votes, params, reaches_quorum);
        }
    }

    let mut out = Outcome::default();
    for &i in &honest_idx {
        match nodes[i].decision() {
            Some(0) => out.decided_0 += 1,
            Some(_) => out.decided_1 += 1,
            None => out.undecided += 1,
        }
    }
    out.safe = out.decided_1 == 0 && out.undecided == 0;
    out.capture = out.decided_1 > 0;
    out.fork = out.decided_0 > 0 && out.decided_1 > 0;
    out
}

/// Pick a position within `base_ids` ∝ its weight, returning the actual node index `base_ids[pos]`.
fn base_pick(base_ids: &[usize], base_cum: &[f64], rng: &mut SplitMix64) -> usize {
    let pos = weighted_pick(base_cum, rng);
    base_ids[pos]
}

/// Adversary controls a fraction `f` of the TOTAL reputation, spread over `n_adv` nodes; honest nodes
/// have reputation 1 each. (Test-rig `population_reputation_fraction`.)
pub fn population_reputation_fraction(f: f64, n_honest: usize, n_adv: usize) -> Population {
    let mut reputation = vec![1.0f64; n_honest];
    let mut adversary = vec![false; n_honest];
    if f > 0.0 {
        let honest_total = n_honest as f64;
        let adv_total = f * honest_total / (1.0 - f);
        let per = adv_total / n_adv as f64;
        for _ in 0..n_adv {
            reputation.push(per);
            adversary.push(true);
        }
    }
    Population { reputation, adversary }
}

/// Adversary has MANY nodes but reputation ~0 (the fake crowd). (Test-rig `population_sybil`.)
pub fn population_sybil(n_honest: usize, n_sybil: usize, sybil_rep: f64) -> Population {
    let mut reputation = vec![1.0f64; n_honest];
    let mut adversary = vec![false; n_honest];
    for _ in 0..n_sybil {
        reputation.push(sybil_rep);
        adversary.push(true);
    }
    Population { reputation, adversary }
}

/// Cluster-id base for adversarial blocs, kept disjoint from the honest cluster ids.
const ADV_CLUSTER_BASE: usize = 1_000_000;

/// CORRELATED adversary: fraction `f` of the reputation spread over `n_adv_clusters` trust blocs (1 =
/// one correlated bloc). Honest nodes sit in many small independent clusters. Returns the population
/// plus the per-node cluster ids. (Test-rig `population_clustered_adversary`.)
pub fn population_clustered_adversary(
    f: f64,
    n_honest: usize,
    n_adv: usize,
    honest_per_cluster: usize,
    n_adv_clusters: usize,
) -> (Population, Vec<usize>) {
    let mut reputation = vec![1.0f64; n_honest];
    let mut adversary = vec![false; n_honest];
    let mut clusters: Vec<usize> = (0..n_honest)
        .map(|i| i / honest_per_cluster.max(1))
        .collect();
    if f > 0.0 {
        let adv_total = f * n_honest as f64 / (1.0 - f);
        let per = adv_total / n_adv as f64;
        let ncl = n_adv_clusters.max(1);
        for i in 0..n_adv {
            reputation.push(per);
            adversary.push(true);
            clusters.push(ADV_CLUSTER_BASE + (i % ncl));
        }
    }
    (Population { reputation, adversary }, clusters)
}

/// Honest nodes split into a large group A (id 0) and a small group B (id 1); the adversary (fraction
/// `f_adv` of total reputation) lives entirely in B. Returns the population + per-node group ids.
/// (Test-rig `population_partitioned`.)
pub fn population_partitioned(
    f_adv: f64,
    n_a: usize,
    n_b: usize,
    n_adv: usize,
) -> (Population, Vec<usize>) {
    let mut reputation = vec![1.0f64; n_a + n_b];
    let mut adversary = vec![false; n_a + n_b];
    let mut groups: Vec<usize> = (0..n_a).map(|_| 0).chain((0..n_b).map(|_| 1)).collect();
    if f_adv > 0.0 {
        let adv_total = f_adv * (n_a + n_b) as f64 / (1.0 - f_adv);
        let per = adv_total / n_adv as f64;
        for _ in 0..n_adv {
            reputation.push(per);
            adversary.push(true);
            groups.push(1); // the adversary lives in the small group B
        }
    }
    (Population { reputation, adversary }, groups)
}

/// One full epoch of Woven-Trust: **sortition elects the committee, then only the committee votes.**
/// Ties the two defences together (SPEC §2.1–2.2) — sortition keeps reputation-less sybils out of the
/// committee, and the Snowball quorum keeps a minority of reputation from imposing a value even when it
/// does win seats. `reputation`/`adversary` are indexed by node; `seed`/`tau` parameterise the election
/// (expected committee size `tau`). Returns the honest-committee outcome; if the elected committee has
/// no honest members the run is vacuously "no honest decision" (undecided).
///
/// `std` host harness only — the reference oracle, never shipped in the runtime.
pub fn run_epoch(
    pop: &Population,
    seed: &str,
    tau: u32,
    params: &SnowballParams,
    rng: &mut SplitMix64,
    adversary: Adversary,
) -> Outcome {
    let n = pop.reputation.len();
    // Reputation -> fixed-point map + deterministic per-node VRF secret keys (host-side stand-ins).
    let mut rep_fp: BTreeMap<String, i128> = BTreeMap::new();
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    for i in 0..n {
        let id = format!("n{i}");
        rep_fp.insert(id.clone(), (pop.reputation[i].max(0.0) * FP as f64) as i128);
        keys.insert(id.clone(), format!("sk{i}"));
    }
    let committee = elect_committee_fp(&rep_fp, &keys, seed, tau);

    // The committee becomes the voting population; a member's weight = its seats (sortition multiplicity).
    let mut reputation = Vec::new();
    let mut adversary_flags = Vec::new();
    for (id, &seats) in &committee {
        let i: usize = id[1..].parse().unwrap();
        reputation.push(seats as f64);
        adversary_flags.push(pop.adversary[i]);
    }
    let committee_pop = Population { reputation, adversary: adversary_flags };
    run_once(&committee_pop, params, rng, true, adversary, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aggregate safe% / capture% over `trials` runs (the rig's statistical assertion style).
    fn aggregate(pop: &Population, weighted: bool, trials: u32, seed: u64) -> (f64, f64) {
        let params = SnowballParams::default();
        let mut rng = SplitMix64::new(seed);
        let (mut safe, mut capture) = (0u32, 0u32);
        for _ in 0..trials {
            let r = run_once(pop, &params, &mut rng, weighted, Adversary::Fixed, 0.0);
            safe += r.safe as u32;
            capture += r.capture as u32;
        }
        (100.0 * safe as f64 / trials as f64, 100.0 * capture as f64 / trials as f64)
    }

    #[test]
    fn no_adversary_is_safe() {
        let pop = population_reputation_fraction(0.0, 80, 5);
        let (safe, capture) = aggregate(&pop, true, 60, 0xC0FFEE);
        assert_eq!((safe, capture), (100.0, 0.0));
    }

    #[test]
    fn weighted_sybil_is_safe() {
        // 1000 fake nodes at reputation ~0 vs 80 honest -> reputation weighting keeps it safe.
        let pop = population_sybil(80, 1000, 1e-9);
        let (safe, capture) = aggregate(&pop, true, 60, 0x5EED);
        assert!(safe >= 95.0, "weighted sybil should be safe, was {safe}%");
        assert_eq!(capture, 0.0);
    }

    #[test]
    fn uniform_sybil_fails() {
        // Same crowd, but sampling ignores reputation -> the fake majority breaks it (contrast).
        let pop = population_sybil(80, 1000, 1e-9);
        let (safe, _capture) = aggregate(&pop, false, 40, 0x5EED);
        assert!(safe < 50.0, "uniform sybil should NOT be safe, was {safe}%");
    }

    #[test]
    fn reputation_majority_captures() {
        let pop = population_reputation_fraction(0.5, 80, 5);
        let (safe, capture) = aggregate(&pop, true, 60, 0xBADF00D);
        assert!(capture > 0.0 && safe == 0.0, "50% rep should capture: safe={safe} cap={capture}");
    }

    #[test]
    fn threshold_is_reputation_not_nodes() {
        // 10% adversarial reputation, well under the 0.70 wall -> safe.
        let pop = population_reputation_fraction(0.1, 80, 5);
        let (safe, _) = aggregate(&pop, true, 60, 0x1234);
        assert_eq!(safe, 100.0);
    }

    /// Aggregate safe% / capture% over `trials` full epochs (committee re-elected each epoch via a
    /// fresh seed) — the combined sortition+voting path.
    fn aggregate_epoch(pop: &Population, tau: u32, trials: u32, seed: u64) -> (f64, f64) {
        let params = SnowballParams::default();
        let mut rng = SplitMix64::new(seed);
        let (mut safe, mut capture, mut counted) = (0u32, 0u32, 0u32);
        for t in 0..trials {
            let r = run_epoch(pop, &format!("epoch{t}"), tau, &params, &mut rng, Adversary::Fixed);
            // Skip vacuous runs where the elected committee had no honest members at all.
            if r.decided_0 + r.decided_1 + r.undecided == 0 {
                continue;
            }
            counted += 1;
            safe += r.safe as u32;
            capture += r.capture as u32;
        }
        let c = counted.max(1) as f64;
        (100.0 * safe as f64 / c, 100.0 * capture as f64 / c)
    }

    #[test]
    fn adaptive_adversary_does_not_capture() {
        // The adaptive (anti-finality) adversary attacks LIVENESS (stalls), but below the reputation
        // threshold it never forces a false decision (safety intact). Rig: test_adaptive_adversary_*.
        let pop = population_reputation_fraction(0.3, 80, 5);
        let params = SnowballParams::default();
        let mut rng = SplitMix64::new(0xADA);
        let mut capture = 0u32;
        for _ in 0..40 {
            let r = run_once(&pop, &params, &mut rng, true, Adversary::Adaptive, 0.0);
            capture += r.capture as u32;
        }
        assert_eq!(capture, 0, "adaptive adversary should not capture (only stall)");
    }

    #[test]
    fn message_loss_preserves_safety() {
        // An adversary below the threshold (25%) never captures even under heavy loss; loss only adds
        // stalls (liveness), never breaks safety. Rig: test_message_loss_degrades_liveness_not_safety.
        let pop = population_reputation_fraction(0.25, 80, 5);
        let params = SnowballParams::default();
        for &loss in &[0.0, 0.4, 0.6] {
            let mut rng = SplitMix64::new(0x1984);
            let mut capture = 0u32;
            for _ in 0..40 {
                let r = run_once(&pop, &params, &mut rng, true, Adversary::Fixed, loss);
                capture += r.capture as u32;
            }
            assert_eq!(capture, 0, "loss {loss}: must not capture (safety)");
        }
    }

    #[test]
    fn safety_property_below_threshold_sweep() {
        // PROPERTY (ANALYSIS §3): in the safety regime the network never finalises a false value nor
        // forks, across many schedules and BOTH adversary behaviours. Rig: test_safety_property_*.
        let params = SnowballParams::default();
        for &f in &[0.1, 0.2, 0.3] {
            let pop = population_reputation_fraction(f, 80, 5);
            for adv in [Adversary::Fixed, Adversary::Adaptive] {
                for seed in 0..30u64 {
                    let mut rng = SplitMix64::new(0x5A_0000 + seed);
                    let r = run_once(&pop, &params, &mut rng, true, adv, 0.0);
                    assert!(!r.capture, "safety broken at f={f} seed={seed}");
                    assert!(!r.fork, "fork at f={f} seed={seed}");
                }
            }
        }
    }

    #[test]
    fn loss_above_bound_stalls_without_capture() {
        // PROPERTY (ANALYSIS §4): loss that violates alpha <= k(1-p) only ADDS stalls, never capture.
        // Measured on the no-adversary network to isolate liveness. Rig: test_liveness_boundary_*.
        let pop = population_reputation_fraction(0.0, 80, 5);
        let params = SnowballParams::default();
        let mut rng = SplitMix64::new(7);
        let (mut stalls, mut capture) = (0u32, 0u32);
        for _ in 0..40 {
            let r = run_once(&pop, &params, &mut rng, true, Adversary::Fixed, 0.5);
            stalls += (r.undecided > 0) as u32;
            capture += r.capture as u32;
        }
        assert_eq!(capture, 0, "loss must never cause capture (safety)");
        assert!(stalls > 0, "loss above the alpha<=k(1-p) bound should produce stalls");
    }

    /// Aggregate safe% / capture% with a cluster-cap config over `trials` runs.
    fn aggregate_cap(
        pop: &Population,
        clusters: &[usize],
        cap: Option<u32>,
        adversary: Adversary,
        trials: u32,
        seed: u64,
    ) -> (f64, f64) {
        let params = SnowballParams::default();
        let mut rng = SplitMix64::new(seed);
        let (mut safe, mut capture) = (0u32, 0u32);
        for _ in 0..trials {
            let cfg = RunConfig {
                adversary,
                clusters: Some(clusters),
                cap_cluster: cap,
                ..Default::default()
            };
            let r = run_once_cfg(pop, &params, &mut rng, &cfg);
            safe += r.safe as u32;
            capture += r.capture as u32;
        }
        (100.0 * safe as f64 / trials as f64, 100.0 * capture as f64 / trials as f64)
    }

    #[test]
    fn independence_protects_from_correlated_bloc() {
        // 45% of the reputation all in ONE correlated cluster captures with rep-only sampling; the
        // per-cluster independence cap neutralises it. Rig: test_independence_protects_*.
        let (pop, cl) = population_clustered_adversary(0.45, 80, 12, 2, 1);
        let (_safe_no, cap_no) = aggregate_cap(&pop, &cl, None, Adversary::Fixed, 40, 0xC011);
        let (safe_cap, cap_cap) = aggregate_cap(&pop, &cl, Some(3), Adversary::Fixed, 40, 0xC011);
        assert!(cap_no > 0.0, "rep-only should be captured by the correlated bloc");
        assert!(safe_cap >= 95.0 && cap_cap == 0.0, "the cap should protect: safe={safe_cap} cap={cap_cap}");
    }

    #[test]
    fn independence_yields_if_adversary_fragments_enough() {
        // cap=3 over alpha=14 forces >= ceil(14/3)=5 distinct blocs to capture. 2 blocs hold; 6 break.
        // Rig: test_independence_yields_if_adversary_fragments_enough.
        let (pop2, cl2) = population_clustered_adversary(0.45, 80, 12, 2, 2);
        let (_s2, cap2) = aggregate_cap(&pop2, &cl2, Some(3), Adversary::Fixed, 40, 0xF2A9);
        let (pop6, cl6) = population_clustered_adversary(0.45, 80, 12, 2, 6);
        let (_s6, cap6) = aggregate_cap(&pop6, &cl6, Some(3), Adversary::Fixed, 40, 0xF2A9);
        assert_eq!(cap2, 0.0, "with 2 blocs (<5) the cap should hold");
        assert!(cap6 > 0.0, "with 6 blocs (>=5) the adversary should capture again");
    }

    /// Partition measure: fork% / safe% over `trials` runs with a partition of `d` rounds.
    fn measure_partition(
        f_adv: f64,
        d: u32,
        network_quorum: f64,
        trials: u32,
        seed: u64,
    ) -> (f64, f64) {
        // The rig uses max_rounds=120 for the partition study (longer, to let groups heal + decide).
        let params = SnowballParams { max_rounds: 120, ..SnowballParams::default() };
        let (pop, groups) = population_partitioned(f_adv, 60, 20, 6);
        let mut rng = SplitMix64::new(seed);
        let (mut fork, mut safe) = (0u32, 0u32);
        for _ in 0..trials {
            let cfg = RunConfig {
                groups: Some(&groups),
                partition_rounds: d,
                network_quorum,
                ..Default::default()
            };
            let r = run_once_cfg(&pop, &params, &mut rng, &cfg);
            fork += r.fork as u32;
            safe += r.safe as u32;
        }
        (100.0 * fork as f64 / trials as f64, 100.0 * safe as f64 / trials as f64)
    }

    #[test]
    fn partition_long_forks_without_mitigation() {
        // A globally harmless 15% adversary concentrated in the small group, with a long partition,
        // captures B and forks on heal (the real safety risk the simulator found). Rig: test_partition_long_*.
        let (fork, _safe) = measure_partition(0.15, 90, 0.0, 40, 0x7C04);
        assert!(fork > 50.0, "the long partition should fork, was {fork}%");
    }

    #[test]
    fn partition_quorum_preserves_safety() {
        // Conditioning finality on seeing >=60% of the reputation almost eliminates the fork.
        // Rig: test_partition_quorum_preserves_safety.
        let (fork_no, _) = measure_partition(0.15, 90, 0.0, 40, 0x7C04);
        let (fork_yes, _) = measure_partition(0.15, 90, 0.6, 40, 0x7C04);
        assert!(fork_yes < fork_no * 0.2, "quorum should cut the fork sharply ({fork_no}->{fork_yes})");
        assert!(fork_yes < 10.0, "with quorum the fork should be low, was {fork_yes}%");
    }

    #[test]
    fn epoch_honest_only_is_safe() {
        let pop = population_reputation_fraction(0.0, 80, 5);
        let (safe, capture) = aggregate_epoch(&pop, 40, 40, 0xE0);
        assert_eq!((safe, capture), (100.0, 0.0));
    }

    #[test]
    fn epoch_sortition_excludes_sybils() {
        // 1000 fake nodes at reputation ~0: sortition gives them ~0 seats -> they never enter the
        // committee -> the committee is all-honest -> safe. The sortition layer alone defeats the crowd.
        let pop = population_sybil(80, 1000, 1e-9);
        let (safe, capture) = aggregate_epoch(&pop, 40, 40, 0x5B17);
        assert!(safe >= 95.0, "sortition should keep sybils out of committee, safe={safe}%");
        assert_eq!(capture, 0.0);
    }

    #[test]
    fn epoch_low_reputation_adversary_is_mostly_safe() {
        // 10% adversarial reputation -> ~10% of committee seats, well under the 0.70 wall. Through a
        // SMALL committee (tau=40) sortition variance lets the adversary occasionally over-draw seats,
        // so safety is high but not perfect (~97–98%). That residual is the cost of a small committee
        // and shrinks as tau grows (the committee-size lever, PARAMETERS.md / task #18); the analytical
        // bound lives in ANALYSIS-safety-liveness.md. We assert the qualitative property: low reputation
        // stays overwhelmingly safe (same bar as the sybil case).
        let pop = population_reputation_fraction(0.1, 80, 5);
        let (safe, _) = aggregate_epoch(&pop, 40, 40, 0x10A);
        assert!(safe >= 95.0, "10% rep through sortition should stay safe, safe={safe}%");
    }

    #[test]
    fn epoch_reputation_majority_captures() {
        // 50% adversarial reputation -> wins enough committee seats to breach the quorum -> capture.
        let pop = population_reputation_fraction(0.5, 80, 5);
        let (_safe, capture) = aggregate_epoch(&pop, 40, 40, 0x50C);
        assert!(capture > 0.0, "50% rep should still capture through the committee, capture={capture}%");
    }
}
