//! Vouching mechanics: sponsorship quota, persistent liability, mentor dividend and cascade slashing.
//! Rust port of the validated prototype (`reputation-engine/vouch.py`). SPEC §1.5c, §1.7.
//!
//! The incentive to sponsor is ONLY reputational and the sponsor->protege link is PERSISTENT:
//! - negative: if the protege defrauds, the cascade slashing climbs the vouch chain (½ per hop) —
//!   *you answer for whom you bring in*.
//! - positive: the mentor dividend is a small echo of the protege's INDEPENDENT reputation, so
//!   sponsoring puppets (reputation dependent on one's own cluster) does not pay off.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::{fp_div, fp_mul, FP_SCALE};

/// A persistent sponsor->protege link. `live` is released on graduation, but the liability does not
/// expire (the link stays for cascade slashing).
#[derive(Clone, Debug)]
pub struct Sponsorship {
    pub sponsor: String,
    pub protege: String,
    pub live: bool,
}

/// Registry of who vouched for whom (persistent liability, §1.5c).
#[derive(Default)]
pub struct VouchRegistry {
    pub links: Vec<Sponsorship>,
}

impl VouchRegistry {
    pub fn new() -> Self {
        VouchRegistry { links: Vec::new() }
    }

    pub fn sponsor_link(&mut self, sponsor: &str, protege: &str) {
        self.links.push(Sponsorship { sponsor: sponsor.into(), protege: protege.into(), live: true });
    }

    pub fn sponsors_of(&self, protege: &str) -> Vec<String> {
        self.links.iter().filter(|v| v.protege == protege).map(|v| v.sponsor.clone()).collect()
    }

    pub fn live_vouches(&self, sponsor: &str) -> usize {
        self.links.iter().filter(|v| v.sponsor == sponsor && v.live).count()
    }

    /// Graduate the protege: release the LIVE link (frees the sponsor's quota) once the protege stands
    /// on its own. The liability stays in the registry. Returns true if any were graduated.
    pub fn graduate(&mut self, sponsor: &str, protege: &str) -> bool {
        let mut graduated = false;
        for v in self.links.iter_mut() {
            if v.sponsor == sponsor && v.protege == protege && v.live {
                v.live = false;
                graduated = true;
            }
        }
        graduated
    }
}

/// Live-vouch quota = sublinear in reputation (§1.5c): `floor(k * log2(1 + rep))`. Decreasing returns,
/// so nobody monopolises sponsorship.
#[cfg(feature = "std")]
pub fn vouch_quota(aggregate_reputation: f64, k: f64) -> u64 {
    let r = aggregate_reputation.max(0.0);
    (k * (1.0 + r).log2()).floor() as u64
}

/// Mentor dividend (§1.5c, positive side): a small echo of the protege's INDEPENDENT reputation.
/// Sponsoring puppets (independent reputation ~0) pays ~0.
#[cfg(feature = "std")]
pub fn mentor_dividend(protege_independent_rep: f64, echo: f64) -> f64 {
    echo * protege_independent_rep.max(0.0)
}

// ---------------------------------------------------------------------------------------------------
// Deterministic fixed-point vouch scoring (i128, FP_SCALE-scaled). `no_std`-ready: no libm — the only
// hard piece, `log2`, is computed in integers via `ln(m) = 2·atanh((m-1)/(m+1))` then `/ ln2`. This is
// the path the pallet runs; cross-validated against the f64 versions above within tolerance.
// ---------------------------------------------------------------------------------------------------

/// ln(2) · FP_SCALE.
const LN2_FP: i128 = 693_147_181;

/// `log2(x)` in fixed-point, `x_fp = x · FP_SCALE`, `x > 0`. Range-reduce to a mantissa in [1,2), then
/// `ln(mantissa) = 2·atanh(s)`, `s = (m-1)/(m+1)` (|s| < 1/3 → fast series), and divide by ln2.
pub fn log2_fp(x_fp: i128) -> i128 {
    if x_fp <= 0 {
        return 0;
    }
    let mut e: i128 = 0;
    let mut m = x_fp;
    while m >= 2 * FP_SCALE {
        m /= 2;
        e += 1;
    }
    while m < FP_SCALE {
        m *= 2;
        e -= 1;
    }
    // m in [FP_SCALE, 2·FP_SCALE): mantissa in [1,2). s = (m-1)/(m+1).
    let s = fp_div(m - FP_SCALE, m + FP_SCALE);
    let s2 = fp_mul(s, s);
    let mut term = s; // s^1
    let mut sum = s;
    let mut k: i128 = 3;
    loop {
        term = fp_mul(term, s2); // s^k, k odd
        let add = term / k;
        sum += add;
        if add == 0 || k > 21 {
            break;
        }
        k += 2;
    }
    let ln_m = 2 * sum; // ln(mantissa)
    e * FP_SCALE + fp_div(ln_m, LN2_FP)
}

/// Live-vouch quota in fixed-point: `floor(k · log2(1 + rep))`. `aggregate_reputation_fp` and `k_fp`
/// are FP_SCALE-scaled; negative reputation clamps to 0.
pub fn vouch_quota_fp(aggregate_reputation_fp: i128, k_fp: i128) -> u64 {
    let r = aggregate_reputation_fp.max(0);
    let l = log2_fp(FP_SCALE + r); // log2(1 + rep)
    let q = fp_mul(k_fp, l); // k · log2(1+rep), FP_SCALE-scaled
    (q / FP_SCALE).max(0) as u64
}

/// Mentor dividend in fixed-point: `echo · max(rep, 0)`. All FP_SCALE-scaled.
pub fn mentor_dividend_fp(protege_independent_rep_fp: i128, echo_fp: i128) -> i128 {
    fp_mul(echo_fp, protege_independent_rep_fp.max(0))
}

/// Cascade slashing in fixed-point (i128, FP_SCALE-scaled). Same persistent-liability recursion as the
/// f64 version: the culprit loses `loss_fp`; each sponsor up the chain loses `sponsor_fraction` of what
/// their protege lost, up to `depth` hops. Returns a NEW map.
pub fn cascade_slashing_fp(
    reputation: &BTreeMap<String, i128>,
    registry: &VouchRegistry,
    culprit: &str,
    loss_fp: i128,
    sponsor_fraction_fp: i128,
    depth: i32,
) -> BTreeMap<String, i128> {
    let mut updated = reputation.clone();
    fn apply(
        updated: &mut BTreeMap<String, i128>,
        registry: &VouchRegistry,
        agent: &str,
        amount: i128,
        level: i32,
        sponsor_fraction_fp: i128,
    ) {
        if amount <= 0 || level < 0 || !updated.contains_key(agent) {
            return;
        }
        let new = (updated[agent] - amount).max(0);
        updated.insert(agent.to_string(), new);
        let passes_on = fp_mul(amount, sponsor_fraction_fp);
        for sponsor in registry.sponsors_of(agent) {
            apply(updated, registry, &sponsor, passes_on, level - 1, sponsor_fraction_fp);
        }
    }
    apply(&mut updated, registry, culprit, loss_fp, depth, sponsor_fraction_fp);
    updated
}

/// Slashing for proven fraud (§1.7) with PERSISTENT LIABILITY in cascade (§1.5c). The culprit loses
/// `loss`; each sponsor up the chain loses `sponsor_fraction` of what their protege lost, up to
/// `depth` hops. Returns a NEW reputation map (does not mutate the input).
#[cfg(feature = "std")]
pub fn cascade_slashing(
    reputation: &BTreeMap<String, f64>,
    registry: &VouchRegistry,
    culprit: &str,
    loss: f64,
    sponsor_fraction: f64,
    depth: i32,
) -> BTreeMap<String, f64> {
    let mut updated = reputation.clone();
    fn apply(
        updated: &mut BTreeMap<String, f64>,
        registry: &VouchRegistry,
        agent: &str,
        amount: f64,
        level: i32,
        sponsor_fraction: f64,
    ) {
        if amount <= 0.0 || level < 0 || !updated.contains_key(agent) {
            return;
        }
        let new = (updated[agent] - amount).max(0.0);
        updated.insert(agent.to_string(), new);
        let passes_on = amount * sponsor_fraction;
        for sponsor in registry.sponsors_of(agent) {
            apply(updated, registry, &sponsor, passes_on, level - 1, sponsor_fraction);
        }
    }
    apply(&mut updated, registry, culprit, loss, depth, sponsor_fraction);
    updated
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn cascade_slashing_matches_python() {
        // protege defrauds and loses 100; the hit climbs ½ per hop. Python prototype:
        // protege 100->0, middle 120->70, mentor 200->175, outsider 150->150.
        let mut rep = BTreeMap::new();
        rep.insert("protege".to_string(), 100.0);
        rep.insert("middle".to_string(), 120.0);
        rep.insert("mentor".to_string(), 200.0);
        rep.insert("outsider".to_string(), 150.0);
        let mut reg = VouchRegistry::new();
        reg.sponsor_link("middle", "protege");
        reg.sponsor_link("mentor", "middle");
        let out = cascade_slashing(&rep, &reg, "protege", 100.0, 0.5, 3);
        assert_eq!(out["protege"], 0.0);
        assert_eq!(out["middle"], 70.0);
        assert_eq!(out["mentor"], 175.0);
        assert_eq!(out["outsider"], 150.0);
    }

    #[test]
    fn quota_is_sublinear() {
        assert_eq!(vouch_quota(0.0, 3.0), 0); // no reputation -> no sponsorship
        assert_eq!(vouch_quota(7.0, 3.0), 9); // floor(3*log2(8)) = 9
        // doubling reputation does not double quota (decreasing returns)
        assert!(vouch_quota(1000.0, 3.0) < 2 * vouch_quota(31.0, 3.0));
    }

    #[test]
    fn graduation_frees_quota_keeps_liability() {
        let mut reg = VouchRegistry::new();
        reg.sponsor_link("mentor", "protege");
        assert_eq!(reg.live_vouches("mentor"), 1);
        assert!(reg.graduate("mentor", "protege"));
        assert_eq!(reg.live_vouches("mentor"), 0); // quota freed
        assert_eq!(reg.sponsors_of("protege"), vec!["mentor".to_string()]); // liability persists
    }

    // ---- fixed-point parity (the no_std path the pallet runs) ----

    fn to_fp(x: f64) -> i128 {
        (x * FP_SCALE as f64).round() as i128
    }

    #[test]
    fn log2_fp_matches_f64() {
        for &x in &[1.0, 1.5, 2.0, 3.0, 8.0, 17.0, 100.0, 1001.0] {
            let fp = log2_fp(to_fp(x)) as f64 / FP_SCALE as f64;
            let f = x.log2();
            assert!((fp - f).abs() < 1e-6, "log2({x}): fp {fp} vs f64 {f}");
        }
    }

    #[test]
    fn vouch_quota_fp_matches_f64() {
        for &(r, k) in &[(0.0, 3.0), (7.0, 3.0), (31.0, 3.0), (1000.0, 3.0), (50.0, 2.0)] {
            let f = vouch_quota(r, k);
            let fp = vouch_quota_fp(to_fp(r), to_fp(k));
            assert_eq!(f, fp, "quota(r={r}, k={k}): f64 {f} vs fp {fp}");
        }
    }

    #[test]
    fn mentor_dividend_fp_matches_f64() {
        for &(r, e) in &[(0.0, 0.1), (10.0, 0.05), (-5.0, 0.2), (700.0, 0.01)] {
            let f = mentor_dividend(r, e);
            let fp = mentor_dividend_fp(to_fp(r), to_fp(e)) as f64 / FP_SCALE as f64;
            assert!((f - fp).abs() < 1e-6, "dividend(r={r}, e={e}): f64 {f} vs fp {fp}");
        }
    }

    #[test]
    fn cascade_slashing_fp_matches_f64() {
        let f64_rep: BTreeMap<String, f64> = [
            ("protege", 100.0),
            ("middle", 120.0),
            ("mentor", 200.0),
            ("outsider", 150.0),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
        let fp_rep: BTreeMap<String, i128> =
            f64_rep.iter().map(|(k, v)| (k.clone(), to_fp(*v))).collect();
        let mut reg = VouchRegistry::new();
        reg.sponsor_link("middle", "protege");
        reg.sponsor_link("mentor", "middle");

        let f = cascade_slashing(&f64_rep, &reg, "protege", 100.0, 0.5, 3);
        let fp = cascade_slashing_fp(&fp_rep, &reg, "protege", to_fp(100.0), to_fp(0.5), 3);
        for id in ["protege", "middle", "mentor", "outsider"] {
            let fv = f[id];
            let xv = fp[id] as f64 / FP_SCALE as f64;
            assert!((fv - xv).abs() < 1e-6, "{id}: f64 {fv} vs fp {xv}");
        }
    }
}
