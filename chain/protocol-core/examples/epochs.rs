//! Run a few epochs of a small Harlequin network and print the telemetry each node would publish.
//! `cargo run -p protocol-core --example epochs`

use protocol_core::Protocol;
use reputation_core::{Agent, Params, DIMENSIONS};

fn founder(id: &str) -> Agent {
    let mut a = Agent::new(id).genesis();
    for d in DIMENSIONS {
        a = a.with_evidence(d, 5.0);
    }
    a
}

fn main() {
    let cohort: Vec<Agent> = (0..50).map(|i| founder(&format!("g{i}"))).collect();
    let params = Params { community: true, in_concentration: true, ..Default::default() };
    let mut chain = Protocol::genesis(cohort, params, 33.0);

    // a swarm of sybils joins (no evidence, no vouches) — they should stay powerless.
    for i in 0..1000 {
        chain.admit(Agent::new(&format!("s{i}")));
    }

    println!("Harlequin epoch telemetry (the shape every node serves to the public panel):\n");
    for _ in 0..3 {
        let report = chain.advance_epoch("woven-trust-beacon");
        println!("{}", report.to_json());
    }
    println!(
        "\nmembers={}  (50 founders + 1000 sybils). Note active_nodes and the committee: the sybils \
never enter, and Gini watches whether power concentrates.",
        chain.member_count()
    );
}
