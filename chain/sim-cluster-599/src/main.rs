//! #599 cluster-guard trajectory simulation — input for the ACTIVATION decision (post-#918).
//!
//! Runs the REAL reputation-core pipeline (EigenTrust fully-fixed + communities + guard step),
//! never re-derived math, over deterministic growth trajectories seeded from the LIVE premeasure
//! of (mainnet 0xa1ab: 4 founders, equal shares ≈25.03%, zero vouch edges, unarmed).
//!
//! Questions it answers:
//!   S1  With founders ringed and externals joining at rate 1/E epochs, WHEN does the leg arm?
//!   S2  Once armed, how fast can the founder ring re-cross 1/3 (counter starts) if it out-grows
//!       the externals — i.e. what growth margin exists before a legitimate halt clock starts?
//!   S3  A sybil ring joining post-arm with aggressive growth: how many epochs until its cluster
//!       crosses 1/3 (detection horizon) and until the halt lands (+7)?
//!   S4  DECLARED RESIDUAL, quantified: a mask-split into UNVOUCHED singletons never forms a
//!       community — the cluster leg is blind to it and single-entity only sees each mask alone.
//!
//! One dimension is simulated; the live pallet takes the WORST suit of four, and every scenario
//! here is suit-symmetric, so one dim == the max. Deterministic: fixed schedules, no randomness.

use reputation_core::{
    entrenchment_halt_step, max_cluster_share_fp, max_single_entity_share_fp,
    reputation_dimension_fully_fixed_fp, Agent, Params, TrustGraph, FP_SCALE,
};
use std::collections::BTreeMap;

const DIM: &str = "commerce";
const THRESHOLD_FP: i128 = FP_SCALE / 3;
const REQUIRED: u32 = 7;

/// One simulated population: evidence per agent + vouch edges; recomputed like the pallet does.
struct World {
    evidence: BTreeMap<String, f64>,
    graph: TrustGraph,
}

impl World {
    fn new() -> Self {
        World { evidence: BTreeMap::new(), graph: TrustGraph::new() }
    }
    fn seed(&mut self, id: &str, ev: f64) {
        self.evidence.insert(id.to_string(), ev);
    }
    fn vouch_pair(&mut self, a: &str, b: &str) {
        self.graph.attest(a, b, DIM, 1.0);
        self.graph.attest(b, a, DIM, 1.0);
    }
    fn vouch_ring(&mut self, ids: &[&str]) {
        for (i, a) in ids.iter().enumerate() {
            let b = ids[(i + 1) % ids.len()];
            self.graph.attest(a, b, DIM, 1.0);
        }
    }
    /// The pallet pipeline for one epoch: EigenTrust (community+concentration damping, fixed-point)
    /// → reps map → per-suit labels → guard shares.
    fn measure(&self) -> Measure {
        let agents: Vec<Agent> = self
            .evidence
            .iter()
            .map(|(id, ev)| Agent::new(id).with_evidence(DIM, *ev))
            .collect();
        let params = Params { community: true, in_concentration: true, ..Default::default() };
        let rep = reputation_dimension_fully_fixed_fp(&agents, &self.graph, DIM, &params);
        let reps: BTreeMap<String, i128> = rep.into_iter().collect();
        let nodes: Vec<String> = reps.keys().cloned().collect();
        let labels = self.graph.communities(DIM, &nodes);
        let cluster = max_cluster_share_fp(&labels, &reps);
        let single = {
            let v: Vec<i128> = reps.values().copied().collect();
            max_single_entity_share_fp(&v)
        };
        let mut sizes: BTreeMap<&String, u32> = BTreeMap::new();
        let mut real_cluster = false;
        for (node, label) in labels.iter() {
            if reps.get(node).is_some_and(|r| *r > 0) {
                let n = sizes.entry(label).or_insert(0);
                *n += 1;
                if *n >= 2 {
                    real_cluster = true;
                }
            }
        }
        Measure { cluster, single, real_cluster }
    }
}

struct Measure {
    cluster: i128,
    single: i128,
    real_cluster: bool,
}

fn pct(fp: i128) -> f64 {
    fp as f64 * 100.0 / FP_SCALE as f64
}

/// Walk epochs applying `step` to the world; run the guard's arming + halt machinery exactly as
/// wired (hysteresis with the R1 real-cluster condition; effective share = max(single, armed?cluster)).
/// Returns (arm_epoch, counter_start_epoch, halt_epoch) as Options.
fn walk(
    world: &mut World,
    epochs: u32,
    label: &str,
    mut armed: bool,
    mut step: impl FnMut(&mut World, u32),
) -> (Option<u32>, Option<u32>, Option<u32>, bool) {
    let mut counter = 0u32;
    let (mut arm_at, mut count_at, mut halt_at) = (None, None, None);
    for e in 1..=epochs {
        step(world, e);
        let m = world.measure();
        if !armed && m.real_cluster && m.cluster > 0 && m.cluster < THRESHOLD_FP {
            armed = true;
            arm_at = Some(e);
        }
        let effective = m.single.max(if armed { m.cluster } else { 0 });
        let (c, halted) = entrenchment_halt_step(effective, THRESHOLD_FP, counter, REQUIRED);
        if counter == 0 && c > 0 && count_at.is_none() {
            count_at = Some(e);
        }
        counter = c;
        if halted && halt_at.is_none() {
            halt_at = Some(e);
            println!(
                "  [{label}] e{e:>3}  cluster={:>6.2}%  single={:>6.2}%  HALT",
                pct(m.cluster),
                pct(m.single)
            );
            break;
        }
        if e % 5 == 0 || arm_at == Some(e) {
            println!(
                "  [{label}] e{e:>3}  cluster={:>6.2}%  single={:>6.2}%  armed={armed} counter={counter}",
                pct(m.cluster),
                pct(m.single)
            );
        }
    }
    (arm_at, count_at, halt_at, armed)
}

fn founders_world() -> World {
    // Live premeasure shape: 4 equal founders (0xa1ab, 25.03% each). Ring assumed formed (the
    // trajectory of interest starts once they vouch — until then communities are singletons).
    let mut w = World::new();
    for f in ["f1", "f2", "f3", "f4"] {
        w.seed(f, 1000.0);
    }
    w.vouch_ring(&["f1", "f2", "f3", "f4"]);
    w
}

fn main() {
    println!("== SIM #599: cluster-guard trajectories (real reputation-core pipeline) ==");
    println!(
        "seeded from a live premeasure: 4 founders, equal, ringed; THRESHOLD=1/3 REQUIRED={REQUIRED}\n"
    );

    // ── S1: organic growth — externals join every E epochs, grow g%/epoch from 50.
    println!("[S1] arming horizon vs onboarding rate (externals unvouched, worst case for arming)");
    // Additive accrual: attest→claim pays per epoch of honest service (≈linear), founders keep
    // serving too (+25/epoch each); an external earns +60/epoch. E = epochs between joins
    // (mainnet epoch = 2h → E=12 ≈ one new member/day).
    for (rate, paired) in [(4u32, false), (12, false), (4, true), (12, true)] {
        let mut w = founders_world();
        let mut joined = 0u32;
        let (arm, _, _, _) = walk(&mut w, 400, &format!("S1 E={rate} paired={paired}"), false, |w, e| {
            if e % rate == 0 {
                joined += 1;
                w.seed(&format!("x{joined}"), 10.0);
                if paired && joined % 2 == 0 {
                    let a = format!("x{}", joined - 1);
                    let b = format!("x{joined}");
                    w.vouch_pair(&a, &b);
                }
            }
            let keys: Vec<String> = w.evidence.keys().cloned().collect();
            for k in keys {
                let v = w.evidence[&k];
                w.evidence.insert(k.clone(), v + if k.starts_with('x') { 60.0 } else { 25.0 });
            }
        });
        println!("  → S1 E={rate} paired={paired}: ARMS at epoch {:?}\n", arm);
    }

    // ── S2/S3 run as CONTINUATIONS of the organic sponsored web (the only state that actually
    // arms — hand-built "armed steady states" are unreachable because vouched founders hold a rep
    // premium the raw evidence ratio hides). First grow until armed, then apply the scenario.
    fn grow_sponsored(w: &mut World, joined: &mut u32, e: u32) {
        if e % 4 == 0 {
            *joined += 1;
            let id = format!("x{joined}");
            w.seed(&id, 10.0);
            let sponsor = if *joined <= 8 {
                format!("f{}", (*joined - 1) % 4 + 1)
            } else {
                format!("x{}", *joined / 2)
            };
            w.graph.attest(&sponsor, &id, DIM, 1.0);
        }
        let keys: Vec<String> = w.evidence.keys().cloned().collect();
        for k in keys {
            let v = w.evidence[&k];
            w.evidence.insert(k.clone(), v + if k.starts_with('x') { 60.0 } else { 25.0 });
        }
    }

    println!("[S2] post-arm founder re-concentration: founders multiply their service rate ×(1+Δ)");
    for delta in [10.0f64, 30.0] {
        let mut w = founders_world();
        let mut joined = 0u32;
        let (arm, _, _, armed) =
            walk(&mut w, 400, "S2 grow", false, |w, e| grow_sponsored(w, &mut joined, e));
        let (_, count, halt, _) = walk(&mut w, 2000, &format!("S2 Δ={delta}x"), armed, |w, _| {
            let keys: Vec<String> = w.evidence.keys().cloned().collect();
            for k in keys {
                let v = w.evidence[&k];
                let g = if k.starts_with('f') { 25.0 * (1.0 + delta) } else { 60.0 };
                w.evidence.insert(k, v + g);
            }
        });
        println!(
            "  → S2 Δ={delta}x: armed@{arm:?} then counter-starts@{count:?} HALT@{halt:?} (epochs post-arm)\n"
        );
    }

    println!("[S3] sybil ring joining post-arm (M masks, mutual ring, 3× honest rate)");
    for (m, srate) in [(8u32, 300.0f64), (8, 600.0), (16, 300.0)] {
        let mut w = founders_world();
        let mut joined = 0u32;
        let (arm, _, _, armed) =
            walk(&mut w, 400, "S3 grow", false, |w, e| grow_sponsored(w, &mut joined, e));
        let sybs: Vec<String> = (1..=m).map(|i| format!("s{i}")).collect();
        for sb in &sybs {
            w.seed(sb, 10.0);
        }
        let syb_refs: Vec<&str> = sybs.iter().map(|x| x.as_str()).collect();
        w.vouch_ring(&syb_refs);
        let (_, count, halt, _) = walk(&mut w, 2000, &format!("S3 M={m} r={srate}"), armed, |w, _| {
            let keys: Vec<String> = w.evidence.keys().cloned().collect();
            for k in keys {
                let v = w.evidence[&k];
                let g = if k.starts_with('s') { srate } else { 60.0 };
                w.evidence.insert(k, v + g);
            }
        });
        println!("  → S3 M={m} r={srate}: armed@{arm:?} then sybil-counter@{count:?} HALT@{halt:?} (epochs post-arm)\n");
    }

    // ── S4: declared residual — unvouched mask-split is invisible to the cluster leg.
    println!("[S4] residual: N unvouched split-masks, each just under 1/3 single-entity");
    {
        let mut w = World::new();
        for f in ["m1", "m2", "m3"] {
            w.seed(f, 1000.0); // one entity in 3 masks, NO vouch edges between them
        }
        for i in 1..=2 {
            w.seed(&format!("x{i}"), 40.0); // marginal honest rest
        }
        let m = w.measure();
        println!(
            "  3 masks, no edges: cluster={:.2}% single={:.2}% real_cluster={} → guard BLIND while each mask < 1/3 (known residual: needs behavioural clustering, #754 v2)\n",
            pct(m.cluster),
            pct(m.single),
            m.real_cluster
        );
    }
    // ── S5: the CONNECTED organic web — every newcomer is vouched by an existing member (sponsor
    // tree rooted in the founders, as the society is designed to grow). The question that decides
    // everything: does label propagation keep sub-communities apart, or fuse the whole web into ONE
    // community (cluster share ≈ 100% forever → the leg could never arm)?
    println!("[S5] connected sponsor-tree web: does communities() ever split a connected society?");
    {
        let mut w = founders_world();
        let mut joined = 0u32;
        let (arm, _, _, _) = walk(&mut w, 400, "S5 sponsored", false, |w, e| {
            if e % 4 == 0 {
                joined += 1;
                let id = format!("x{joined}");
                w.seed(&id, 10.0);
                // sponsor = round-robin founder for the first wave, then earlier externals
                let sponsor = if joined <= 8 {
                    format!("f{}", (joined - 1) % 4 + 1)
                } else {
                    format!("x{}", joined / 2)
                };
                w.graph.attest(&sponsor, &id, DIM, 1.0);
            }
            let keys: Vec<String> = w.evidence.keys().cloned().collect();
            for k in keys {
                let v = w.evidence[&k];
                w.evidence.insert(k.clone(), v + if k.starts_with('x') { 60.0 } else { 25.0 });
            }
        });
        println!("  → S5 sponsored (connected web): ARMS at epoch {:?}\n", arm);
    }
    println!("== SIM END ==");
}
