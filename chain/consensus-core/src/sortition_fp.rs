//! Deterministic fixed-point sortition (i128) — the integer counterpart of `vrf.rs`, so committee
//! election is bit-identical across architectures (a consensus requirement: f64 `exp`/summation is not
//! reproducible). Same algorithm as the f64 path; cross-validated against it within a tiny tolerance.
//!
//! The hard part is the Poisson CDF, which needs `e^{-lam}`. We compute it in fixed-point by range
//! reduction: `e^{-lam} = (e^{-1})^floor(lam) * e^{-frac}`, the integer power by repeated multiply and
//! the fractional part by a short Taylor series (it converges fast because `frac < 1`). No floats, no
//! libm — `no_std`-ready, dependency-free (OPSEC: nothing pulled onto the isolated station).

use crate::sha256::sha256;
use alloc::collections::BTreeMap;
use alloc::string::String;

/// Fixed-point scale, 1e9 — matches `reputation-core` so reputation values feed straight in.
pub const FP: i128 = 1_000_000_000;

/// e^{-1} * FP, rounded. The base of the integer-power range reduction.
const E_INV_FP: i128 = 367_879_441;

#[inline]
fn fp_mul(a: i128, b: i128) -> i128 {
    a * b / FP
}

#[inline]
fn fp_div(a: i128, b: i128) -> i128 {
    a * FP / b
}

/// `e^{-lam}` in fixed-point, `lam_fp = lam * FP >= 0`, result in `[0, FP]`. Range-reduced so it stays
/// accurate for large `lam` (where the naive Taylor series of `e^{-lam}` loses all precision).
pub fn exp_neg_fp(lam_fp: i128) -> i128 {
    if lam_fp <= 0 {
        return FP;
    }
    let n = lam_fp / FP; // integer part of lam
    let f = lam_fp - n * FP; // fractional part in [0, FP)

    // e^{-f} via Taylor: sum_k (-f)^k / k!  (f < 1 -> fast convergence)
    let mut term = FP; // k = 0 term = 1
    let mut sum = FP;
    let mut k: i128 = 1;
    while k <= 20 {
        // term_k = term_{k-1} * (-f) / k   (all fixed-point)
        term = term * (-f) / FP / k;
        sum += term;
        if term == 0 {
            break;
        }
        k += 1;
    }

    // multiply by (e^{-1})^n
    let mut result = sum;
    for _ in 0..n {
        result = fp_mul(result, E_INV_FP);
        if result == 0 {
            break;
        }
    }
    result.clamp(0, FP)
}

/// Poisson CDF `P(X <= j)` for mean `lam` (fixed-point), in `[0, FP]`. Iterative term:
/// `term_0 = e^{-lam}`, `term_i = term_{i-1} * lam / i`.
pub fn poisson_cdf_fp(j: u32, lam_fp: i128) -> i128 {
    if lam_fp <= 0 {
        return FP;
    }
    let mut term = exp_neg_fp(lam_fp);
    let mut cdf = term;
    for i in 1..=j {
        term = term * lam_fp / FP / (i as i128);
        cdf += term;
        if term == 0 {
            break;
        }
    }
    cdf.min(FP)
}

/// Inverse Poisson CDF: committee seats for a node with expected `lam` and VRF value `value_fp` in
/// `[0, FP)`. `lam_fp <= 0 -> 0` seats. Capped at `max_seats`.
pub fn sortition_seats_fp(value_fp: i128, lam_fp: i128, max_seats: u32) -> u32 {
    if lam_fp <= 0 {
        return 0;
    }
    let mut j = 0;
    while j < max_seats {
        if value_fp < poisson_cdf_fp(j, lam_fp) {
            return j;
        }
        j += 1;
    }
    max_seats
}

/// Deterministic VRF value in `[0, FP)` from (sk, seed): the top 64 bits of SHA-256(`sk|seed`) scaled
/// to fixed-point. Deterministic in (sk, seed) and grind-resistant (a node cannot improve its value
/// without changing its key). Uses fewer bits than the f64 path but is exact integer arithmetic.
pub fn vrf_value_fp(sk: &str, seed: &str) -> i128 {
    let mut input = sk.as_bytes().to_vec();
    input.push(b'|');
    input.extend_from_slice(seed.as_bytes());
    let d = sha256(&input);
    let mut top: u128 = 0;
    for &b in &d[..8] {
        top = (top << 8) | b as u128;
    }
    // value = top / 2^64 * FP, computed without overflow (top < 2^64, FP < 2^30 -> < 2^94).
    ((top * FP as u128) >> 64) as i128
}

/// Elect an epoch committee by reputation-weighted sortition, fully fixed-point. `reputation` is any
/// consistent scale (only the ratio r/total matters); `tau` is the expected total seats. Returns
/// {node: seats} for winners. Deterministic and architecture-independent.
pub fn elect_committee_fp(
    reputation: &BTreeMap<String, i128>,
    secret_keys: &BTreeMap<String, String>,
    seed: &str,
    tau: u32,
) -> BTreeMap<String, u32> {
    let mut committee = BTreeMap::new();
    let total: i128 = reputation.values().map(|&r| if r > 0 { r } else { 0 }).sum();
    if total <= 0 {
        return committee;
    }
    let tau_fp = tau as i128 * FP;
    for (node, &r) in reputation {
        if r <= 0 {
            continue;
        }
        let share = fp_div(r, total); // r/total in FP
        let lam_fp = fp_mul(tau_fp, share); // tau * r/total
        let value_fp = vrf_value_fp(&secret_keys[node], seed);
        let seats = sortition_seats_fp(value_fp, lam_fp, 64);
        if seats > 0 {
            committee.insert(node.clone(), seats);
        }
    }
    committee
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vrf::{poisson_cdf_f64_for_test, sortition_seats};

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn exp_neg_matches_f64() {
        for &lam in &[0.0, 0.05, 0.3, 0.7, 1.0, 2.0, 3.5, 5.0, 10.0, 20.0, 40.0] {
            let fp = exp_neg_fp((lam * FP as f64).round() as i128) as f64 / FP as f64;
            let f = (-lam).exp();
            assert!(approx(fp, f, 1e-6 + f * 1e-6), "exp(-{lam}): fp {fp} vs f64 {f}");
        }
    }

    #[test]
    fn poisson_cdf_matches_f64() {
        for &lam in &[0.1, 0.5, 1.0, 3.0, 8.0] {
            let lam_fp = (lam * FP as f64).round() as i128;
            for j in 0..12u32 {
                let fp = poisson_cdf_fp(j, lam_fp) as f64 / FP as f64;
                let f = poisson_cdf_f64_for_test(j, lam);
                assert!(approx(fp, f, 1e-5), "cdf(j={j}, lam={lam}): fp {fp} vs f64 {f}");
            }
        }
    }

    #[test]
    fn sortition_seats_matches_f64() {
        // for matched (value, lam), the fixed-point seat count equals the f64 one.
        for &lam in &[0.5, 1.0, 2.0, 5.0] {
            let lam_fp = (lam * FP as f64).round() as i128;
            for vi in 0..20 {
                let value = vi as f64 / 20.0;
                let value_fp = (value * FP as f64).round() as i128;
                let f = sortition_seats(value, lam, 64);
                let fp = sortition_seats_fp(value_fp, lam_fp, 64);
                assert_eq!(f, fp, "seats(value={value}, lam={lam}): f64 {f} vs fp {fp}");
            }
        }
    }

    #[test]
    fn committee_excludes_sybils_and_sizes_to_tau() {
        let mut reps: BTreeMap<String, i128> = (0..120).map(|i| (format!("h{i}"), FP)).collect();
        let keys: BTreeMap<String, String> =
            reps.keys().map(|n| (n.clone(), format!("sk-{n}"))).collect();
        let c = elect_committee_fp(&reps, &keys, "seed-x", 60);
        let seats: u32 = c.values().sum();
        assert!(seats >= 30 && seats <= 90, "committee ~tau=60, was {seats}");

        for i in 0..2000 {
            reps.insert(format!("s{i}"), FP / 1_000_000); // reputation ~0
        }
        let keys2: BTreeMap<String, String> =
            reps.keys().map(|n| (n.clone(), format!("sk-{n}"))).collect();
        let c2 = elect_committee_fp(&reps, &keys2, "seed-y", 60);
        let sybils = c2.keys().filter(|n| n.starts_with('s')).count();
        assert_eq!(sybils, 0, "sybils must not enter the committee, got {sybils}");
    }

    #[test]
    fn deterministic_across_runs() {
        let reps: BTreeMap<String, i128> = (0..50).map(|i| (format!("h{i}"), FP)).collect();
        let keys: BTreeMap<String, String> =
            reps.keys().map(|n| (n.clone(), format!("sk-{n}"))).collect();
        let a = elect_committee_fp(&reps, &keys, "beacon|epoch3", 25);
        let b = elect_committee_fp(&reps, &keys, "beacon|epoch3", 25);
        assert_eq!(a, b);
    }
}
