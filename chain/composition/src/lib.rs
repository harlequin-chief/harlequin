//! Composition: the reputation engine feeds the consensus sortition. This is the end-to-end claim of
//! Harlequin, in Rust — the same as the Python `integration_demo.py`: **committee power is reputation,
//! and reputation cannot be bought or faked.** The two crates compose: `reputation-core` derives each
//! pseudonym's vectorial reputation (anchored in evidence, anti-collusion damped), it is aggregated
//! conservatively (min over the four suits — you must be trusted in all of them), and that scalar is
//! the weight `consensus-core` uses to elect the committee by VRF sortition. A Sybil or a collusion
//! ring earns ~0 reputation → ~0 sortition weight → ~0 committee seats.

use std::collections::HashMap;

use reputation_core::{conservative_aggregate, reputation_vector, Agent, Params, TrustGraph};

/// Aggregate each agent's reputation vector (min over the four suits, §1.2b) into the scalar weight
/// the consensus sortition draws on. `min=true` is the conservative aggregate used for consensus.
pub fn committee_weights(agents: &[Agent], graph: &TrustGraph, p: &Params) -> HashMap<String, f64> {
    reputation_vector(agents, graph, p)
        .into_iter()
        .map(|(id, vec)| (id, conservative_aggregate(&vec, true)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use consensus_core::elect_committee;

    fn keys(ids: &[String]) -> HashMap<String, String> {
        ids.iter().map(|n| (n.clone(), format!("sk-{n}"))).collect()
    }

    #[test]
    fn power_is_earned_reputation_not_number_nor_collusion() {
        let dims = reputation_core::DIMENSIONS;
        let mut agents: Vec<Agent> = Vec::new();
        let mut g = TrustGraph::new();

        // 5 genesis with evidence in every suit; they vouch for honest members in every suit.
        let genesis: Vec<String> = (0..5).map(|i| format!("g{i}")).collect();
        for gid in &genesis {
            let mut a = Agent::new(gid).genesis();
            for d in dims {
                a = a.with_evidence(d, 2.0);
            }
            agents.push(a);
        }
        // 20 honest with real evidence in every suit, vouched by the genesis in every suit.
        for i in 0..20 {
            let hid = format!("h{i}");
            let mut a = Agent::new(&hid);
            for d in dims {
                a = a.with_evidence(d, 3.0);
            }
            agents.push(a);
            for gid in genesis.iter().take(3) {
                for d in dims {
                    g.attest(gid, &hid, d, 1.0);
                }
            }
        }
        // a collusion ring of 5: no evidence, just vouch each other in a circle (commerce).
        let ring: Vec<String> = (0..5).map(|i| format!("c{i}")).collect();
        for cid in &ring {
            agents.push(Agent::new(cid));
        }
        for i in 0..ring.len() {
            g.attest(&ring[i], &ring[(i + 1) % ring.len()], "commerce", 1.0);
        }
        // 50 sybils: no evidence, no vouches.
        for i in 0..50 {
            agents.push(Agent::new(&format!("s{i}")));
        }

        let p = Params { community: true, in_concentration: true, ..Default::default() };
        let weights = committee_weights(&agents, &g, &p);

        // bad actors carry ~0 conservative reputation
        for cid in &ring {
            assert!(weights[cid] < 1.0, "colluder {cid} should have ~0 weight, got {}", weights[cid]);
        }
        for i in 0..50 {
            assert!(weights[&format!("s{i}")] < 1.0, "sybil should have ~0 weight");
        }

        // elect a committee weighted by reputation -> only honest/genesis win seats
        let ids: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
        let committee = elect_committee(&weights, &keys(&ids), "epoch0", 30.0);
        let bad_seats: u32 = committee
            .iter()
            .filter(|(n, _)| n.starts_with('c') || n.starts_with('s'))
            .map(|(_, s)| *s)
            .sum();
        let good_seats: u32 = committee.values().sum::<u32>() - bad_seats;
        assert_eq!(bad_seats, 0, "no committee power for collusion/sybils, got {bad_seats}");
        assert!(good_seats > 0, "honest must hold the committee");
    }
}
