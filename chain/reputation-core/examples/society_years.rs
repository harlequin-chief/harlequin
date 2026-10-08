//! A society over the years, run on the REAL reputation engine (2026-10-01, matrix 4.3).
//!
//! Weekly steps for up to 10 years. Each step: honest members who are active earn evidence (service, settled
//! deals) and vouch for others; everyone's evidence decays at the mainnet rate (half-life 2 years); then the
//! engine (`reputation_dimension_fully_fixed_fp`, same params as the pallet) recomputes the four suits and we
//! take the consensus aggregate the pallet uses for power: (1-λ)·min + λ·median over the suits, λ = 1/2.
//! What we watch: the share of power held by the biggest single actor and by each attacker group, against the
//! 1/3 line where the entrenchment guard freezes finality.
//!
//! Scenarios (one per run):
//!   growth     — honest society grows from 4 founders to N; founders keep serving.
//!   stagnant   — the society never grows past 12; founders keep serving (can they stay above 1/3?).
//!   sybil      — a swarm of S fresh masks with no evidence vouching densely for each other and for a leader.
//!   ring       — k real members collude: they attest commerce to each other every week (within the per-epoch
//!                budget) and vouch for each other.
//!   slow       — one member serves honestly for 3 years, then spawns 50 sockpuppets and vouches them all.
//!   abandon    — founders stop serving after year 1; does power drain to the newcomers?
//! Simplifications (said, not hidden): attestation limits are modelled as a weekly cap per pair and per
//! attester (the pallet's budget 8/epoch, pair decay, stake cap); justice and slashing are not modelled.
//!
//! Run: cargo run --release --example society_years -- <scenario> [years] [final_size] [seed]
use reputation_core::{reputation_dimension_fully_fixed_fp, Agent, Params, TrustGraph, DIMENSIONS};
use std::collections::BTreeMap;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 1_000_000) as f64 / 1_000_000.0 < p
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Founder,
    Honest,
    Sybil,
    Ring,
    Slow,
    Puppet,
}

struct M {
    kind: Kind,
    ev: [f64; 4],
    active: bool,
    joined_week: u32,
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let scenario = a.first().map(|s| s.as_str()).unwrap_or("growth").to_string();
    let years: u32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let final_size: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(400);
    let seed: u64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(7);
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let weeks = years * 52;
    let decay_week = 0.999_052f64.powi(7); // mainnet EvidenceRetention, daily epochs
    let params = Params { community: true, in_concentration: true, ..Default::default() };

    let mut ms: Vec<M> = (0..4).map(|_| M { kind: Kind::Founder, ev: [10.0; 4], active: true, joined_week: 0 }).collect();
    let mut vouches: Vec<(usize, usize, usize, f64)> = Vec::new(); // (from, to, suit, weight)
    let target = |w: u32| -> usize {
        match scenario.as_str() {
            "stagnant" => 12,
            _ => {
                let y = w as f64 / 52.0;
                4 + ((final_size as f64 - 4.0) / (1.0 + (-(y - 3.0) * 1.3).exp())) as usize
            }
        }
    };
    println!("# scenario={scenario} years={years} final_size={final_size} seed={seed}");
    println!("year\tmembers\ttop1_share\ttop1_kind\tfounders_share\tattackers_share\tabove_1/3");
    for w in 0..weeks {
        // arrivals
        while ms.iter().filter(|m| m.kind == Kind::Honest || m.kind == Kind::Founder).count() < target(w) {
            ms.push(M { kind: Kind::Honest, ev: [0.0; 4], active: true, joined_week: w });
        }
        // attackers arrive
        if w == 26 && scenario == "sybil" {
            for _ in 0..200 {
                ms.push(M { kind: Kind::Sybil, ev: [0.0; 4], active: true, joined_week: w });
            }
        }
        if w == 26 && scenario == "ring" {
            for _ in 0..8 {
                ms.push(M { kind: Kind::Ring, ev: [0.0; 4], active: true, joined_week: w });
            }
        }
        if w == 1 && scenario == "slow" {
            ms.push(M { kind: Kind::Slow, ev: [0.0; 4], active: true, joined_week: w });
        }
        if w == 3 * 52 && scenario == "slow" {
            for _ in 0..50 {
                ms.push(M { kind: Kind::Puppet, ev: [0.0; 4], active: true, joined_week: w });
            }
        }
        if w == 52 && scenario == "abandon" {
            for m in ms.iter_mut().filter(|m| m.kind == Kind::Founder) {
                m.active = false;
            }
        }
        // churn: 0.4%/week of honest members go quiet
        for m in ms.iter_mut().filter(|m| m.kind == Kind::Honest) {
            if m.active && r.chance(0.004) {
                m.active = false;
            }
        }
        // evidence accrual for active real members (service + settled deals), heavy-tailed effort
        let n = ms.len();
        for i in 0..n {
            let m = &ms[i];
            let earns = m.active && matches!(m.kind, Kind::Founder | Kind::Honest | Kind::Slow | Kind::Ring);
            if earns {
                let effort = 1.0 + (r.below(100) as f64 / 25.0).powi(2);
                let s = r.below(4) as usize;
                ms[i].ev[s] += effort;
                ms[i].ev[1] += 0.5; // technical: node service
            }
        }
        // ring: members attest commerce to each other within the weekly cap (budget 8/epoch ≈ capped pair flow)
        if scenario == "ring" {
            let ring: Vec<usize> = (0..n).filter(|&i| ms[i].kind == Kind::Ring).collect();
            for &i in &ring {
                for &j in &ring {
                    if i != j {
                        let cap = ms[i].ev[0].min(2.0); // stake cap: never more than the attester's own evidence
                        ms[j].ev[0] += cap * 0.25; // pair decay keeps repeated attestations shrinking
                    }
                }
            }
        }
        // vouches: real members vouch occasionally, preferential to well-known members
        for i in 0..n {
            let k = ms[i].kind;
            if ms[i].active && matches!(k, Kind::Founder | Kind::Honest) && r.chance(0.05) {
                let j = if r.chance(0.3) { r.below(8.min(n as u64)) as usize } else { r.below(n as u64) as usize };
                if j != i && matches!(ms[j].kind, Kind::Founder | Kind::Honest) {
                    vouches.push((i, j, r.below(4) as usize, 1.0 + r.below(5) as f64));
                }
            }
        }
        if w == 26 && scenario == "sybil" {
            let sy: Vec<usize> = (0..n).filter(|&i| ms[i].kind == Kind::Sybil).collect();
            for &i in &sy {
                for _ in 0..6 {
                    let j = sy[r.below(sy.len() as u64) as usize];
                    if j != i {
                        vouches.push((i, j, r.below(4) as usize, 10.0));
                    }
                }
                vouches.push((i, sy[0], 0, 10.0));
            }
        }
        if w == 26 && scenario == "ring" {
            let ring: Vec<usize> = (0..n).filter(|&i| ms[i].kind == Kind::Ring).collect();
            for &i in &ring {
                for &j in &ring {
                    if i != j {
                        for s in 0..4 {
                            vouches.push((i, j, s, 10.0));
                        }
                    }
                }
            }
        }
        if w == 3 * 52 && scenario == "slow" {
            let slow = (0..n).find(|&i| ms[i].kind == Kind::Slow).unwrap();
            for j in (0..n).filter(|&i| ms[i].kind == Kind::Puppet) {
                for s in 0..4 {
                    vouches.push((slow, j, s, 10.0));
                    vouches.push((j, slow, s, 10.0));
                }
            }
        }
        // decay
        for m in ms.iter_mut() {
            for e in m.ev.iter_mut() {
                *e *= decay_week;
            }
        }
        // measure once a year (and at half a year)
        let measure = (w + 1) % 52 == 0 || w + 1 == 26;
        if !measure {
            continue;
        }
        let agents: Vec<Agent> = ms
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let mut ag = Agent::new(&i.to_string());
                if m.kind == Kind::Founder {
                    ag = ag.genesis();
                }
                for (s, d) in DIMENSIONS.iter().enumerate() {
                    if m.ev[s] > 0.0 {
                        ag = ag.with_evidence(d, m.ev[s]);
                    }
                }
                ag
            })
            .collect();
        let mut g = TrustGraph::new();
        for (f, t, s, wgt) in &vouches {
            g.attest(&f.to_string(), &t.to_string(), DIMENSIONS[*s], *wgt);
        }
        let mut per: BTreeMap<usize, [i128; 4]> = BTreeMap::new();
        for (s, d) in DIMENSIONS.iter().enumerate() {
            let rep = reputation_dimension_fully_fixed_fp(&agents, &g, d, &params);
            for (id, v) in rep {
                per.entry(id.parse().unwrap()).or_insert([0; 4])[s] = v;
            }
        }
        // consensus aggregate: (1-λ)·min + λ·median (λ = 1/2), as the pallet
        let agg: Vec<f64> = (0..ms.len())
            .map(|i| {
                let mut v = per.get(&i).copied().unwrap_or([0; 4]);
                v.sort();
                let min = v[0] as f64;
                let med = (v[1] + v[2]) as f64 / 2.0;
                (0.5 * min + 0.5 * med).max(0.0)
            })
            .collect();
        let total: f64 = agg.iter().sum::<f64>().max(1e-9);
        let (top_i, top_v) = agg.iter().enumerate().fold((0, 0.0), |b, (i, v)| if *v > b.1 { (i, *v) } else { b });
        let share = |pred: &dyn Fn(Kind) -> bool| -> f64 {
            ms.iter().zip(&agg).filter(|(m, _)| pred(m.kind)).map(|(_, v)| v).sum::<f64>() / total
        };
        let founders = share(&|k| k == Kind::Founder);
        let attackers = match scenario.as_str() {
            "sybil" => share(&|k| k == Kind::Sybil),
            "ring" => share(&|k| k == Kind::Ring),
            "slow" => share(&|k| k == Kind::Slow || k == Kind::Puppet),
            _ => 0.0,
        };
        let kind = match ms[top_i].kind {
            Kind::Founder => "founder",
            Kind::Honest => "honest",
            Kind::Sybil => "sybil",
            Kind::Ring => "ring",
            Kind::Slow => "slow",
            Kind::Puppet => "puppet",
        };
        let worst = (top_v / total).max(attackers).max(founders);
        println!(
            "{:.1}\t{}\t{:.3}\t{}\t{:.3}\t{:.3}\t{}",
            (w + 1) as f64 / 52.0,
            ms.len(),
            top_v / total,
            kind,
            founders,
            attackers,
            if worst > 1.0 / 3.0 { "YES" } else { "no" }
        );
        let _ = ms.iter().map(|m| m.joined_week).max();
    }
}
