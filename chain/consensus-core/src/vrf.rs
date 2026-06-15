//! Verifiable sortition by reputation (SPEC §2.2) — Rust port of the test-rig `vrf.py`. The VRF is
//! SIMULATED by SHA-256 (real ECVRF in production); the committee is elected weighted by REPUTATION,
//! not stake, so a Sybil with reputation ~0 wins ~0 seats (Art. VI). Dependency-free.

use crate::sha256::{hex, sha256};
use std::collections::HashMap;

/// Simulated VRF: returns (value in [0,1), proof). Deterministic in (sk, seed); a node cannot grind
/// for a better value without changing its key. `proof` lets a verifier recompute it.
pub fn vrf(sk: &str, seed: &str) -> (f64, String) {
    let proof = hex(&sha256(format!("{sk}|{seed}").as_bytes()));
    // value from the top 128 bits of the digest / 2^128 (ample precision for sortition thresholds).
    let d = sha256(format!("{sk}|{seed}").as_bytes());
    let mut top: u128 = 0;
    for &b in &d[..16] {
        top = (top << 8) | b as u128;
    }
    let value = top as f64 / (u128::MAX as f64 + 1.0);
    (value, proof)
}

/// Recompute the VRF from (sk, seed) and check it matches the claimed proof (no grinding).
pub fn vrf_verify(sk: &str, seed: &str, proof: &str) -> bool {
    hex(&sha256(format!("{sk}|{seed}").as_bytes())) == proof
}

fn poisson_cdf(j: u32, lam: f64) -> f64 {
    if lam <= 0.0 {
        return 1.0;
    }
    let mut term = (-lam).exp();
    let mut cdf = term;
    for i in 1..=j {
        term *= lam / i as f64;
        cdf += term;
    }
    cdf.min(1.0)
}

/// Inverse Poisson CDF: number of committee seats for a node with expected `lam`, from its VRF value.
/// lam=0 -> 0 seats. Capped at `max_seats`.
pub fn sortition_seats(value: f64, lam: f64, max_seats: u32) -> u32 {
    if lam <= 0.0 {
        return 0;
    }
    let mut j = 0;
    while j < max_seats {
        if value < poisson_cdf(j, lam) {
            return j;
        }
        j += 1;
    }
    max_seats
}

/// Elect an epoch committee by reputation-weighted sortition. Returns {node: seats} for winners.
/// Changing `seed` each epoch rotates the committee (Art. VI, anti-entrenchment).
pub fn elect_committee(
    reputation: &HashMap<String, f64>,
    secret_keys: &HashMap<String, String>,
    seed: &str,
    tau: f64,
) -> HashMap<String, u32> {
    let total: f64 = reputation.values().map(|r| r.max(0.0)).sum();
    let mut committee = HashMap::new();
    if total <= 0.0 {
        return committee;
    }
    for (node, &r) in reputation {
        let r = r.max(0.0);
        if r <= 0.0 {
            continue;
        }
        let lam = tau * r / total;
        let (value, _) = vrf(&secret_keys[node], seed);
        let seats = sortition_seats(value, lam, 64);
        if seats > 0 {
            committee.insert(node.clone(), seats);
        }
    }
    committee
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(reps: &HashMap<String, f64>) -> HashMap<String, String> {
        reps.keys().map(|n| (n.clone(), format!("sk-{n}"))).collect()
    }

    #[test]
    fn vrf_verifiable_and_grind_resistant() {
        let (_, proof) = vrf("sk-h0", "epoch0");
        assert!(vrf_verify("sk-h0", "epoch0", &proof));
        assert!(!vrf_verify("sk-h0", "epoch1", &proof));
        assert!(!vrf_verify("sk-h9", "epoch0", &proof));
    }

    #[test]
    fn committee_is_reputation_weighted_and_excludes_sybils() {
        let mut reps: HashMap<String, f64> = (0..120).map(|i| (format!("h{i}"), 1.0)).collect();
        let k = keys(&reps);
        let c = elect_committee(&reps, &k, "seed-x", 60.0);
        let seats: u32 = c.values().sum();
        assert!(seats >= 30 && seats <= 90, "committee seats ~tau=60, was {seats}");

        // add 2000 sybils at reputation ~0 -> they win ~0 seats
        for i in 0..2000 {
            reps.insert(format!("s{i}"), 1e-6);
        }
        let k2 = keys(&reps);
        let c2 = elect_committee(&reps, &k2, "seed-y", 60.0);
        let sybils = c2.keys().filter(|n| n.starts_with('s')).count();
        assert_eq!(sybils, 0, "sybils must not enter the committee, got {sybils}");
    }

    #[test]
    fn committee_rotates_across_epochs() {
        let reps: HashMap<String, f64> = (0..120).map(|i| (format!("h{i}"), 1.0)).collect();
        let k = keys(&reps);
        let c0: std::collections::HashSet<String> =
            elect_committee(&reps, &k, "beacon|epoch0", 60.0).into_keys().collect();
        let c1: std::collections::HashSet<String> =
            elect_committee(&reps, &k, "beacon|epoch1", 60.0).into_keys().collect();
        let overlap = c0.intersection(&c1).count() as f64 / c0.union(&c1).count() as f64;
        assert!(overlap < 0.8, "committees should rotate, overlap {overlap}");
    }
}
