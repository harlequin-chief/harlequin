//! Runnable end-to-end demo: reputation engine -> consensus sortition. Build a network of honest
//! members, a collusion ring and a Sybil crowd; derive reputation, aggregate it conservatively, and
//! elect a committee weighted by it. Prints who holds power. Run: `cargo run -p composition`.

use std::collections::HashMap;

use composition::committee_weights;
use consensus_core::elect_committee;
use reputation_core::{Agent, Params, TrustGraph, DIMENSIONS};

fn main() {
    let mut agents: Vec<Agent> = Vec::new();
    let mut g = TrustGraph::new();

    // 5 genesis (evidence in every suit), vouching honest members in every suit.
    let genesis: Vec<String> = (0..5).map(|i| format!("g{i}")).collect();
    for gid in &genesis {
        let mut a = Agent::new(gid).genesis();
        for d in DIMENSIONS {
            a = a.with_evidence(d, 2.0);
        }
        agents.push(a);
    }
    // 20 honest with real evidence in every suit.
    for i in 0..20 {
        let hid = format!("h{i}");
        let mut a = Agent::new(&hid);
        for d in DIMENSIONS {
            a = a.with_evidence(d, 3.0);
        }
        agents.push(a);
        for gid in genesis.iter().take(3) {
            for d in DIMENSIONS {
                g.attest(gid, &hid, d, 1.0);
            }
        }
    }
    // collusion ring of 5 (no evidence, vouch in a circle).
    let ring: Vec<String> = (0..5).map(|i| format!("c{i}")).collect();
    for cid in &ring {
        agents.push(Agent::new(cid));
    }
    for i in 0..ring.len() {
        g.attest(&ring[i], &ring[(i + 1) % ring.len()], "commerce", 1.0);
    }
    // 50 sybils (nothing).
    for i in 0..50 {
        agents.push(Agent::new(&format!("s{i}")));
    }

    let p = Params { community: true, in_concentration: true, ..Default::default() };
    let weights = committee_weights(&agents, &g, &p);

    let keys: HashMap<String, String> =
        agents.iter().map(|a| (a.id.clone(), format!("sk-{}", a.id))).collect();
    let committee = elect_committee(&weights, &keys, "epoch0", 30.0);

    let class = |id: &str| -> &'static str {
        match id.chars().next().unwrap() {
            'g' => "genesis",
            'h' => "honest",
            'c' => "colluder",
            _ => "sybil",
        }
    };
    let mut seats: HashMap<&str, u32> = HashMap::new();
    let mut pop: HashMap<&str, u32> = HashMap::new();
    for a in &agents {
        *pop.entry(class(&a.id)).or_default() += 1;
    }
    for (id, s) in &committee {
        *seats.entry(class(id)).or_default() += *s;
    }

    println!("Harlequin — reputation ⨉ consensus, end-to-end\n");
    println!("{:<10} {:>4}  {:>8}  {:>13}", "class", "pop", "seats", "consensus %");
    let total: u32 = committee.values().sum();
    for c in ["genesis", "honest", "colluder", "sybil"] {
        let s = *seats.get(c).unwrap_or(&0);
        let pct = if total > 0 { 100.0 * s as f64 / total as f64 } else { 0.0 };
        println!("{:<10} {:>4}  {:>8}  {:>12.1}%", c, pop.get(c).unwrap_or(&0), s, pct);
    }
    println!(
        "\n{} sybils + {} colluders hold {} committee seats. Power is earned reputation, not number.",
        pop["sybil"], pop["colluder"],
        seats.get("colluder").unwrap_or(&0) + seats.get("sybil").unwrap_or(&0)
    );
}
