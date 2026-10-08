//! How long does one reputation epoch take as the society grows? (2026-10-01)
//!
//! `pallet-reputation::recompute_reputation` runs `reputation_dimension_fully_fixed_fp` for the 4 suits over
//! EVERY participant, inside `on_initialize`, under a placeholder weight. This measures the same call with the
//! same params on synthetic societies of growing size, natively. The runtime runs it as wasm (slower, assume
//! ~2-3x) and the block slot is 12 s with a typical 2 s compute budget, so anything near a second here is a
//! block-production risk.
//!
//! Run: cargo run --release --example epoch_scale [-- N1 N2 ...]
use reputation_core::{reputation_dimension_fully_fixed_fp, Agent, Params, TrustGraph, DIMENSIONS};
use std::time::Instant;

// Small deterministic PRNG (xorshift) so every run builds the same society.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn society(n: usize, vouches_per: usize, seed: u64) -> (Vec<Agent>, TrustGraph) {
    let mut r = Rng(seed | 1);
    let mut agents = Vec::with_capacity(n);
    for i in 0..n {
        let mut a = Agent::new(&i.to_string());
        if i < 4 {
            a = a.genesis();
        }
        for d in DIMENSIONS {
            // roughly half the society has evidence in a given suit, heavy-tailed amounts
            if r.below(2) == 0 {
                a = a.with_evidence(d, (1 + r.below(1000) * r.below(10)) as f64);
            }
        }
        agents.push(a);
    }
    let mut g = TrustGraph::new();
    for i in 0..n {
        for _ in 0..vouches_per {
            // preferential: a third of the vouches go to the first 5% (well-known members)
            let j = if r.below(3) == 0 { r.below((n as u64 / 20).max(1)) } else { r.below(n as u64) } as usize;
            if j != i {
                let d = DIMENSIONS[r.below(4) as usize];
                g.attest(&i.to_string(), &j.to_string(), d, (1 + r.below(10)) as f64);
            }
        }
    }
    (agents, g)
}

fn main() {
    let sizes: Vec<usize> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let sizes = if sizes.is_empty() { vec![50, 200, 500, 1000, 2000] } else { sizes };
    let params = Params { community: true, in_concentration: true, ..Default::default() };
    println!("participants\tvouches_per\tepoch_ms_native(4 suits)");
    for n in sizes {
        let (agents, graph) = society(n, 5, n as u64 * 7919);
        let t = Instant::now();
        let mut total: i128 = 0;
        for d in DIMENSIONS {
            let rep = reputation_dimension_fully_fixed_fp(&agents, &graph, d, &params);
            total += rep.values().sum::<i128>();
        }
        println!("{}\t5\t{}\t(check {})", n, t.elapsed().as_millis(), total % 1000);
    }
}
