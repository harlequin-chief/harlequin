//! Vouching mechanics: sponsorship quota, persistent liability, mentor dividend and cascade slashing.
//! Rust port of the validated prototype (`prototipos/reputacion/vouch.py`). SPEC §1.5c, §1.7.
//!
//! The incentive to sponsor is ONLY reputational and the sponsor->protege link is PERSISTENT:
//! - negative: if the protege defrauds, the cascade slashing climbs the vouch chain (½ per hop) —
//!   *you answer for whom you bring in*.
//! - positive: the mentor dividend is a small echo of the protege's INDEPENDENT reputation, so
//!   sponsoring puppets (reputation dependent on one's own cluster) does not pay off.

use alloc::string::String;
use alloc::vec::Vec;
// Used only by the std-gated f64 scoring (cascade_slashing); the registry itself is alloc-only.
#[cfg(feature = "std")]
use alloc::collections::BTreeMap;

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
}
