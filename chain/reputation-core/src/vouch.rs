//! Vouching mechanics: sponsorship quota, persistent liability, mentor dividend and cascade slashing.
//! Rust port of the validated prototype (`reputation-engine/vouch.py`). SPEC §1.5c, §1.7.
//!
//! The incentive to sponsor is ONLY reputational and the sponsor->protege link is PERSISTENT:
//! - negative: if the protege defrauds, the cascade slashing climbs the vouch chain (½ per hop) —
//!   *you answer for whom you bring in*.
//! - positive: the mentor dividend is a small echo of the protege's INDEPENDENT reputation, so
//!   sponsoring puppets (reputation dependent on one's own cluster) does not pay off.

use alloc::collections::{BTreeMap, BTreeSet};
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

    /// Undirected neighbours of `node`: every member one vouch edge away, in EITHER direction
    /// ("related" is symmetric for the interest test, Art. IX).
    fn neighbors(&self, node: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for l in &self.links {
            if l.sponsor == node {
                out.push(l.protege.clone());
            } else if l.protege == node {
                out.push(l.sponsor.clone());
            }
        }
        out
    }

    /// Undirected vouch-graph distance between `a` and `b`, capped at `max_hops` (BFS over sponsor links
    /// in either direction). `Some(0)` if `a == b`; `Some(d)` if reachable in `d ≤ max_hops` hops;
    /// `None` if farther, unreachable, OR the search exceeds `max_visit` nodes. Lets justice distinguish
    /// **depth-1** (excluded outright) from **depth-2 rings** (weight-penalised + audited, SPEC §4i-(8)):
    /// a ring woven only at distance ≥2 evades depth-1 exclusion (the reputed-ring residual quantified in
    /// (8)). **`max_visit` is a HARD anti-DoS bound:** a party with a huge vouch set projects a
    /// large depth-2 neighbourhood → an unbounded BFS would make opening a case a vector to inflate; the
    /// cap makes each query cost-bounded (conservative: a capped-out search returns `None` = no penalty,
    /// never a false relation).
    pub fn vouch_distance(&self, a: &str, b: &str, max_hops: u32, max_visit: u32) -> Option<u32> {
        if a == b {
            return Some(0);
        }
        let mut visited: BTreeSet<String> = BTreeSet::new();
        visited.insert(a.to_string());
        let mut frontier: Vec<String> = Vec::new();
        frontier.push(a.to_string());
        let mut depth = 0u32;
        while !frontier.is_empty() && depth < max_hops {
            depth += 1;
            let mut next: Vec<String> = Vec::new();
            for node in &frontier {
                for nb in self.neighbors(node) {
                    if nb == b {
                        return Some(depth);
                    }
                    if visited.len() as u32 >= max_visit {
                        return None; // hard compute bound: give up rather than let the graph inflate cost
                    }
                    if visited.insert(nb.clone()) {
                        next.push(nb);
                    }
                }
            }
            frontier = next;
        }
        None
    }

    /// Smallest vouch-distance from `who` to ANY of `parties`, capped at `max_hops` / `max_visit`: the
    /// relation depth a juror has to the closest party. `Some(1)` = directly related (excluded at the
    /// draw); `Some(2)` = a 2-hop ring (the residual in (8): weight-penalise + audit, not a hard cut — a
    /// hard depth-2 cut shrinks the pool and opens pool-poisoning); `None` = independent beyond the cap.
    pub fn nearest_party_distance(
        &self,
        who: &str,
        parties: &[String],
        max_hops: u32,
        max_visit: u32,
    ) -> Option<u32> {
        parties
            .iter()
            .filter_map(|p| self.vouch_distance(who, p, max_hops, max_visit))
            .min()
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

/// Cascade slashing in fixed-point (i128, FP_SCALE-scaled). The culprit loses `loss_fp`; each sponsor up
/// the vouch chain loses `sponsor_fraction` of what their protege lost, up to `depth` hops. Returns a NEW
/// map. **Breadth-first with a visited set** — bit-identical to the on-chain `pallet::cascade_slash`, so a
/// CYCLIC vouch graph slashes each sponsor AT MOST ONCE (its own vouch is one act, §1.5d), instead of the
/// old recursion that re-slashed nodes that sat on a cycle. Acyclic graphs are unchanged.
pub fn cascade_slashing_fp(
    reputation: &BTreeMap<String, i128>,
    registry: &VouchRegistry,
    culprit: &str,
    loss_fp: i128,
    sponsor_fraction_fp: i128,
    depth: i32,
) -> BTreeMap<String, i128> {
    let mut updated = reputation.clone();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut level: Vec<(String, i128)> = Vec::new();
    level.push((culprit.to_string(), loss_fp));
    let mut remaining = depth;
    loop {
        let mut next: Vec<(String, i128)> = Vec::new();
        for (who, amt) in core::mem::take(&mut level) {
            if amt <= 0 || visited.contains(&who) {
                continue;
            }
            visited.insert(who.clone());
            if let Some(r) = updated.get_mut(&who) {
                *r = (*r - amt).max(0);
            }
            if remaining <= 0 {
                continue;
            }
            let passes_on = fp_mul(amt, sponsor_fraction_fp);
            if passes_on <= 0 {
                continue;
            }
            for sponsor in registry.sponsors_of(&who) {
                if !visited.contains(&sponsor) {
                    next.push((sponsor, passes_on));
                }
            }
        }
        if next.is_empty() || remaining <= 0 {
            break;
        }
        level = next;
        remaining -= 1;
    }
    updated
}

/// Slashing for proven fraud (§1.7) with PERSISTENT LIABILITY in cascade (§1.5c). The culprit loses
/// `loss`; each sponsor up the chain loses `sponsor_fraction` of what their protege lost, up to `depth`
/// hops. Returns a NEW reputation map. **Breadth-first with a visited set** (mirrors `cascade_slashing_fp`
/// and the on-chain pallet): a cyclic vouch graph slashes each sponsor at most once.
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
    let mut visited: BTreeSet<String> = BTreeSet::new();
    let mut level: Vec<(String, f64)> = Vec::new();
    level.push((culprit.to_string(), loss));
    let mut remaining = depth;
    loop {
        let mut next: Vec<(String, f64)> = Vec::new();
        for (who, amt) in core::mem::take(&mut level) {
            if amt <= 0.0 || visited.contains(&who) {
                continue;
            }
            visited.insert(who.clone());
            if let Some(r) = updated.get_mut(&who) {
                *r = (*r - amt).max(0.0);
            }
            if remaining <= 0 {
                continue;
            }
            let passes_on = amt * sponsor_fraction;
            if passes_on <= 0.0 {
                continue;
            }
            for sponsor in registry.sponsors_of(&who) {
                if !visited.contains(&sponsor) {
                    next.push((sponsor, passes_on));
                }
            }
        }
        if next.is_empty() || remaining <= 0 {
            break;
        }
        level = next;
        remaining -= 1;
    }
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
    fn cascade_is_proportional_decaying_and_consent_scoped() {
        // Constitutional invariants of cascade slashing (Art. V bridge, SPEC §1.5d, campaña estrés tick 6).
        // A sponsor loses standing NOT because another's act is charged to them, but because vouching is
        // THEIR own act and it proved a bad judgement — and only ever an attenuated reflection of it:
        //   (1) consent-scoped: only those who vouched are touched; an outsider is never slashed;
        //   (2) proportional & strictly decaying up the chain (fraction < 1 per hop) — far sponsors lose
        //       negligibly, so nobody is "stripped" for a distant protege's act;
        //   (3) depth-bounded: beyond `depth` hops, no loss;
        //   (4) floored at zero: reputation is a reflection, not a debt — it never goes negative.
        let mut rep = BTreeMap::new();
        for (id, r) in [("culprit", 100.0), ("s1", 100.0), ("s2", 100.0), ("s3", 100.0), ("outsider", 100.0)] {
            rep.insert(id.to_string(), r);
        }
        let mut reg = VouchRegistry::new();
        reg.sponsor_link("s1", "culprit");
        reg.sponsor_link("s2", "s1");
        reg.sponsor_link("s3", "s2");

        // (1)+(2): ample depth — losses strictly decrease up the chain; outsider untouched.
        let out = cascade_slashing(&rep, &reg, "culprit", 80.0, 0.5, 10);
        let loss = |id: &str| rep[id] - out[id];
        assert_eq!(loss("outsider"), 0.0, "consent-scoped: a non-sponsor is never slashed");
        assert_eq!(loss("culprit"), 80.0, "the culprit takes the full hit");
        assert!(loss("s1") > loss("s2") && loss("s2") > loss("s3"), "loss must strictly decay up the chain: s1={} s2={} s3={}", loss("s1"), loss("s2"), loss("s3"));
        assert!(loss("s1") < loss("culprit"), "each sponsor loses strictly less than the protege");

        // (3): depth limit respected — at depth=1 only the culprit's direct sponsor is touched.
        let shallow = cascade_slashing(&rep, &reg, "culprit", 80.0, 0.5, 1);
        assert!(rep["s1"] - shallow["s1"] > 0.0, "direct sponsor is touched within depth");
        assert_eq!(shallow["s2"], rep["s2"], "beyond depth, no loss");
        assert_eq!(shallow["s3"], rep["s3"], "beyond depth, no loss");

        // (4): floored at zero — a huge loss never pushes anyone negative.
        let mut poor = rep.clone();
        poor.insert("s1".to_string(), 5.0);
        let floored = cascade_slashing(&poor, &reg, "culprit", 1_000_000.0, 0.9, 10);
        for v in floored.values() {
            assert!(*v >= 0.0, "reputation is a reflection, not a debt: never negative, got {v}");
        }
    }

    #[test]
    fn cascade_is_cycle_safe_each_sponsor_slashed_once() {
        // Campaña estrés tick 12 (macroaudit §3 cycle guard): a CYCLIC vouch graph must slash each
        // sponsor AT MOST ONCE — its vouch is one act (§1.5d) — matching the on-chain pallet's BFS+visited.
        // a <-> b vouch each other; a sponsors the culprit x. The old recursion re-slashed a and b on the
        // cycle (a lost ~53, b ~26 for loss 80); BFS+visited gives the honest once-each (a=40, b=20).
        let mut rep = BTreeMap::new();
        for (k, v) in [("x", 100.0), ("a", 100.0), ("b", 100.0)] {
            rep.insert(k.to_string(), v);
        }
        let mut reg = VouchRegistry::new();
        reg.sponsor_link("a", "x"); // a vouches x
        reg.sponsor_link("a", "b"); // a <-> b cycle
        reg.sponsor_link("b", "a");
        let out = cascade_slashing(&rep, &reg, "x", 80.0, 0.5, 8);
        let loss = |k: &str| rep[k] - out[k];
        assert_eq!(loss("x"), 80.0, "culprit takes the full hit");
        assert!((loss("a") - 40.0).abs() < 1e-9, "a slashed ONCE (loss*0.5), was {}", loss("a"));
        assert!((loss("b") - 20.0).abs() < 1e-9, "b slashed ONCE (loss*0.25), not re-hit on the cycle, was {}", loss("b"));
        // fixed-point path agrees (determinism: core mirrors the pallet).
        let rep_fp: BTreeMap<String, i128> = rep.iter().map(|(k, v)| (k.clone(), to_fp(*v))).collect();
        let out_fp = cascade_slashing_fp(&rep_fp, &reg, "x", to_fp(80.0), to_fp(0.5), 8);
        assert_eq!(out_fp["a"], to_fp(60.0), "fp cycle-safe too (a: 100-40)");
        assert_eq!(out_fp["b"], to_fp(80.0), "fp cycle-safe too (b: 100-20)");
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

    // ---- vouch-graph distance (SPEC §4i-(8): depth-1 exclusion vs depth-2 ring) ----

    const VISIT: u32 = 10_000; // generous cap for the small graphs in these tests

    #[test]
    fn vouch_distance_is_undirected_and_capped() {
        // chain a -> b -> c -> d (directed sponsor links); relation is symmetric, so distance ignores
        // direction. d(a,a)=0, d(a,b)=1, d(a,c)=2, d(a,d)=3; cap hides anything beyond max_hops.
        let mut reg = VouchRegistry::new();
        reg.sponsor_link("a", "b");
        reg.sponsor_link("b", "c");
        reg.sponsor_link("c", "d");
        assert_eq!(reg.vouch_distance("a", "a", 3, VISIT), Some(0));
        assert_eq!(reg.vouch_distance("a", "b", 3, VISIT), Some(1));
        assert_eq!(reg.vouch_distance("a", "c", 3, VISIT), Some(2));
        assert_eq!(reg.vouch_distance("d", "a", 3, VISIT), Some(3)); // either direction
        assert_eq!(reg.vouch_distance("a", "d", 2, VISIT), None); // beyond the hop cap
        assert_eq!(reg.vouch_distance("a", "z", 9, VISIT), None); // unreachable
    }

    #[test]
    fn depth2_ring_evades_depth1_but_is_visible_at_depth2() {
        // The (8) residual: a colluding ring woven only at distance 2 (no direct edge to the party) is
        // NOT depth-1 related → passes a depth-1 (max_hops=1) interest test, but a depth-2 query sees it.
        // party P; juror J reaches P only through M (P—M—J): d(J,P)=2.
        let mut reg = VouchRegistry::new();
        reg.sponsor_link("P", "M");
        reg.sponsor_link("M", "J");
        let parties = ["P".to_string()];
        // depth-1 interest test: J looks independent (the gap that depth-1 exclusion misses).
        assert_eq!(reg.nearest_party_distance("J", &parties, 1, VISIT), None);
        // depth-2 query exposes the ring → justice can weight-penalise + audit J's vote (not hard-cut).
        assert_eq!(reg.nearest_party_distance("J", &parties, 2, VISIT), Some(2));
    }

    #[test]
    fn nearest_party_distance_picks_the_closest_party() {
        // J is 1 hop from party P2 and 2 hops from party P1 → nearest is 1 (most conservative).
        let mut reg = VouchRegistry::new();
        reg.sponsor_link("P1", "X");
        reg.sponsor_link("X", "J");
        reg.sponsor_link("J", "P2");
        let parties = ["P1".to_string(), "P2".to_string()];
        assert_eq!(reg.nearest_party_distance("J", &parties, 3, VISIT), Some(1));
        // no parties → no relation.
        assert_eq!(reg.nearest_party_distance("J", &[], 3, VISIT), None);
    }

    #[test]
    fn max_visit_bounds_the_search_anti_dos() {
        // Anti-DoS: a party with a huge fan-out projects a big depth-2 neighbourhood. With a
        // tight visit cap the BFS gives up (None) instead of exploring it all — cost stays bounded.
        // Star: hub H vouches H->s0..s199 (200 edges). A far node T sits 2 hops from any leaf via H.
        let mut reg = VouchRegistry::new();
        for i in 0..200 {
            reg.sponsor_link("H", &alloc::format!("s{i}"));
        }
        reg.sponsor_link("s0", "T"); // T is at distance 2 from H, 3 from the other leaves
        // Generous cap: the relation is found.
        assert_eq!(reg.vouch_distance("H", "T", 3, 10_000), Some(2));
        // Tight cap (5 nodes): the search bails out before reaching T → None (bounded, conservative).
        assert_eq!(reg.vouch_distance("H", "T", 3, 5), None);
    }
}
