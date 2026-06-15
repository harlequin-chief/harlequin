//! Harlequin reputation engine — Rust port of the validated Python prototype
//! (`prototipos/reputacion/`, 17/17 tests). Foundation of the Substrate **reputation pallet**: the
//! full EigenTrust-with-anti-collusion-damping core, at parity with the prototype.
//!
//! SPEC.md anchors: §1 (reputation), §1.6 (anti-collusion damping). The four reputation dimensions are
//! the four suits of Harlequin (LORE.md): commerce ♦, technical_contribution ♣, judicial_function ♠,
//! governance ♥.
//!
//! Uses `BTreeMap` (not `HashMap`) so iteration — and therefore the EigenTrust summation order — is
//! DETERMINISTIC, a prerequisite for consensus reproducibility.
//!
//! **`no_std`-ready.** Default build (`std` feature) keeps the f64 oracle/prototype path for
//! cross-validation. With `default-features = false` the crate is `no_std` (alloc only) and exposes just
//! the deterministic fixed-point path — `reputation_dimension_fully_fixed` and the `*_fp` factor math,
//! all in integer i128 so the result is bit-identical across architectures. That is what the Substrate
//! reputation pallet links against (the f64 path uses libm methods a runtime cannot rely on).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

pub mod vouch;

/// The four reputation dimensions = the four suits (LORE.md, canon).
pub const DIMENSIONS: [&str; 4] = [
    "commerce",              // ♦ ambition
    "technical_contribution",// ♣ freedom
    "judicial_function",     // ♠ struggle
    "governance",            // ♥ love/family
];

/// A pseudonym. The physical identity is never modelled (Art. VII).
#[derive(Clone, Debug)]
pub struct Agent {
    pub id: String,
    pub unique_human: bool,
    pub genesis: bool,
    /// objective verifiable evidence per dimension (settled deals, proven work, §1.3a).
    pub evidence: BTreeMap<String, f64>,
}

impl Agent {
    pub fn new(id: &str) -> Self {
        Agent { id: id.into(), unique_human: true, genesis: false, evidence: BTreeMap::new() }
    }
    pub fn with_evidence(mut self, dim: &str, v: f64) -> Self {
        self.evidence.insert(dim.into(), v);
        self
    }
    pub fn genesis(mut self) -> Self {
        self.genesis = true;
        self
    }
    fn evidence_in(&self, dim: &str) -> f64 {
        *self.evidence.get(dim).unwrap_or(&0.0)
    }
}

/// Attestation edges per dimension: (source -> target -> weight).
#[derive(Default)]
pub struct TrustGraph {
    edges: BTreeMap<String, BTreeMap<String, BTreeMap<String, f64>>>,
}

impl TrustGraph {
    pub fn new() -> Self {
        TrustGraph { edges: BTreeMap::new() }
    }

    /// `source` vouches for `target` in `dim` (§1.3b). Adds to any existing weight.
    pub fn attest(&mut self, source: &str, target: &str, dim: &str, weight: f64) {
        if source == target {
            return; // nobody vouches for themselves
        }
        *self
            .edges
            .entry(dim.into())
            .or_default()
            .entry(source.into())
            .or_default()
            .entry(target.into())
            .or_insert(0.0) += weight;
    }

    fn outgoing(&self, source: &str, dim: &str) -> BTreeMap<String, f64> {
        self.edges
            .get(dim)
            .and_then(|m| m.get(source))
            .cloned()
            .unwrap_or_default()
    }

    fn out_neighbors(&self, node: &str, dim: &str) -> Vec<String> {
        self.edges
            .get(dim)
            .and_then(|m| m.get(node))
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn has_edge(&self, source: &str, target: &str, dim: &str) -> bool {
        self.edges
            .get(dim)
            .and_then(|m| m.get(source))
            .map(|m| m.contains_key(target))
            .unwrap_or(false)
    }

    /// Independence of the vouch i->j in [0,1] (§1.6): penalises reciprocity and neighbour overlap.
    /// `independence = 1/(1 + beta*reciprocal + gamma*overlap)`. An inbred ring vouch -> ~0.11.
    #[cfg(feature = "std")]
    pub fn independence(&self, i: &str, j: &str, dim: &str, beta: f64, gamma: f64) -> f64 {
        let reciprocal = if self.has_edge(j, i, dim) { 1.0 } else { 0.0 };

        let ni: Vec<String> = self.out_neighbors(i, dim).into_iter().filter(|x| x != j).collect();
        let nj: Vec<String> = self.out_neighbors(j, dim).into_iter().filter(|x| x != i).collect();
        let inter = ni.iter().filter(|x| nj.contains(x)).count();
        let union = {
            let mut u = ni.clone();
            for x in &nj {
                if !u.contains(x) {
                    u.push(x.clone());
                }
            }
            u.len()
        };
        let overlap = if union > 0 { inter as f64 / union as f64 } else { 0.0 };

        1.0 / (1.0 + beta * reciprocal + gamma * overlap)
    }

    /// Deterministic fixed-point `independence` (i128, FP_SCALE-scaled). `beta_fp`/`gamma_fp` are
    /// FP_SCALE-scaled. The overlap is a rational (inter/union) so it converts exactly; no floats. First
    /// factor converted toward a fully-deterministic trust matrix (the rest — community / in-concentration
    /// — follow the same fp_mul/fp_div pattern; tracked in PALLET-DESIGN).
    pub fn independence_fp(&self, i: &str, j: &str, dim: &str, beta_fp: i128, gamma_fp: i128) -> i128 {
        let reciprocal = if self.has_edge(j, i, dim) { FP_SCALE } else { 0 };
        let ni: Vec<String> = self.out_neighbors(i, dim).into_iter().filter(|x| x != j).collect();
        let nj: Vec<String> = self.out_neighbors(j, dim).into_iter().filter(|x| x != i).collect();
        let inter = ni.iter().filter(|x| nj.contains(x)).count() as i128;
        let union = {
            let mut u = ni.clone();
            for x in &nj {
                if !u.contains(x) {
                    u.push(x.clone());
                }
            }
            u.len() as i128
        };
        let overlap = if union > 0 { inter * FP_SCALE / union } else { 0 };
        let denom = FP_SCALE + fp_mul(beta_fp, reciprocal) + fp_mul(gamma_fp, overlap);
        fp_div(FP_SCALE, denom)
    }

    /// Community detection by label propagation over the undirected projection (§1.6). Deterministic
    /// (sorted order + smallest-label tie-break) so it is reproducible — matches the Python prototype.
    pub fn communities(&self, dim: &str, nodes: &[String]) -> BTreeMap<String, String> {
        let node_set: BTreeSet<&String> = nodes.iter().collect();
        let mut adj: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for i in nodes {
            for (j, _) in self.outgoing(i, dim) {
                if node_set.contains(&j) {
                    adj.entry(i.clone()).or_default().insert(j.clone());
                    adj.entry(j.clone()).or_default().insert(i.clone());
                }
            }
        }
        let mut label: BTreeMap<String, String> =
            nodes.iter().map(|n| (n.clone(), n.clone())).collect();
        let mut order = nodes.to_vec();
        order.sort();
        for _ in 0..15 {
            let mut changed = false;
            for n in &order {
                let neigh = match adj.get(n) {
                    Some(s) if !s.is_empty() => s,
                    _ => continue,
                };
                let mut count: BTreeMap<String, usize> = BTreeMap::new();
                for m in neigh {
                    *count.entry(label[m].clone()).or_insert(0) += 1;
                }
                // max count, tie-break = smallest label (Python: max(sorted(count), key=count))
                let mut keys: Vec<&String> = count.keys().collect();
                keys.sort();
                let mut best: Option<(&String, usize)> = None;
                for k in keys {
                    let c = count[k];
                    match best {
                        None => best = Some((k, c)),
                        Some((_, bc)) if c > bc => best = Some((k, c)),
                        _ => {}
                    }
                }
                let best = best.unwrap().0.clone();
                if label[n] != best {
                    label.insert(n.clone(), best);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        label
    }

    /// Community suspicion = internal edges / (1 + community evidence) (§1.6): high when lots of mutual
    /// vouching and little real work (collusion signature, dense AND scattered rings).
    #[cfg(feature = "std")]
    pub fn community_suspicion(
        &self,
        dim: &str,
        nodes: &[String],
        label: &BTreeMap<String, String>,
        evidence: &BTreeMap<String, f64>,
    ) -> BTreeMap<String, f64> {
        let node_set: BTreeSet<&String> = nodes.iter().collect();
        let mut internal: BTreeMap<String, f64> = BTreeMap::new();
        for i in nodes {
            for (j, _) in self.outgoing(i, dim) {
                if node_set.contains(&j) && label[i] == label[&j] {
                    *internal.entry(label[i].clone()).or_insert(0.0) += 1.0;
                }
            }
        }
        let mut ev: BTreeMap<String, f64> = BTreeMap::new();
        for n in nodes {
            *ev.entry(label[n].clone()).or_insert(0.0) += *evidence.get(n).unwrap_or(&0.0);
        }
        let comms: BTreeSet<&String> = label.values().collect();
        comms
            .into_iter()
            .map(|c| {
                let e = *internal.get(c).unwrap_or(&0.0) / (1.0 + *ev.get(c).unwrap_or(&0.0));
                (c.clone(), e)
            })
            .collect()
    }

    /// Deterministic fixed-point community suspicion (i128, FP_SCALE-scaled). `evidence_fp` is
    /// FP_SCALE-scaled. suspicion = internal_edges / (1 + evidence) → fp_div(internal·FP, FP + ev_fp).
    pub fn community_suspicion_fp(
        &self,
        dim: &str,
        nodes: &[String],
        label: &BTreeMap<String, String>,
        evidence_fp: &BTreeMap<String, i128>,
    ) -> BTreeMap<String, i128> {
        let node_set: BTreeSet<&String> = nodes.iter().collect();
        let mut internal: BTreeMap<String, i128> = BTreeMap::new();
        for i in nodes {
            for (j, _) in self.outgoing(i, dim) {
                if node_set.contains(&j) && label[i] == label[&j] {
                    *internal.entry(label[i].clone()).or_insert(0) += 1;
                }
            }
        }
        let mut ev: BTreeMap<String, i128> = BTreeMap::new();
        for n in nodes {
            *ev.entry(label[n].clone()).or_insert(0) += *evidence_fp.get(n).unwrap_or(&0);
        }
        let comms: BTreeSet<&String> = label.values().collect();
        comms
            .into_iter()
            .map(|c| {
                let internal_c = *internal.get(c).unwrap_or(&0);
                let denom = FP_SCALE + *ev.get(c).unwrap_or(&0);
                (c.clone(), fp_div(internal_c * FP_SCALE, denom))
            })
            .collect()
    }

    /// Deterministic fixed-point in-concentration signal: per target (conc, gate, shares), all i128
    /// FP_SCALE-scaled. shares = w/total, conc = Σ shares², gate = n/(n+k0). No floats.
    pub fn in_concentration_signals_fp(
        &self,
        dim: &str,
        nodes: &[String],
        label: &BTreeMap<String, String>,
        k0_fp: i128,
    ) -> BTreeMap<String, (i128, i128, BTreeMap<String, i128>)> {
        let node_set: BTreeSet<&String> = nodes.iter().collect();
        let mut incoming: BTreeMap<String, BTreeMap<String, i128>> = BTreeMap::new();
        let mut in_count: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for src in nodes {
            for (tgt, w) in self.outgoing(src, dim) {
                if node_set.contains(&tgt) && w > 0.0 {
                    let lab = label.get(src).cloned().unwrap_or_else(|| src.clone());
                    *incoming.entry(tgt.clone()).or_default().entry(lab).or_insert(0) += to_fp(w);
                    in_count.entry(tgt.clone()).or_default().insert(src.clone());
                }
            }
        }
        let mut out: BTreeMap<String, (i128, i128, BTreeMap<String, i128>)> = BTreeMap::new();
        for tgt in nodes {
            match incoming.get(tgt) {
                None => {
                    out.insert(tgt.clone(), (0, 0, BTreeMap::new()));
                }
                Some(comm_w) => {
                    let total: i128 = comm_w.values().sum();
                    let shares: BTreeMap<String, i128> =
                        comm_w.iter().map(|(c, w)| (c.clone(), fp_div(*w, total))).collect();
                    let conc: i128 = shares.values().map(|s| fp_mul(*s, *s)).sum();
                    let n = in_count[tgt].len() as i128;
                    let n_fp = n * FP_SCALE;
                    let gate = fp_div(n_fp, n_fp + k0_fp);
                    out.insert(tgt.clone(), (conc, gate, shares));
                }
            }
        }
        out
    }

    /// Asymmetric-funnel signal (§2d): per target, (concentration HHI over source communities, volume
    /// gate, shares per community). Cuts a directed PageRank funnel local independence misses.
    #[cfg(feature = "std")]
    pub fn in_concentration_signals(
        &self,
        dim: &str,
        nodes: &[String],
        label: &BTreeMap<String, String>,
        k0: f64,
    ) -> BTreeMap<String, (f64, f64, BTreeMap<String, f64>)> {
        let node_set: BTreeSet<&String> = nodes.iter().collect();
        let mut incoming: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        let mut in_count: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for src in nodes {
            for (tgt, w) in self.outgoing(src, dim) {
                if node_set.contains(&tgt) && w > 0.0 {
                    let lab = label.get(src).cloned().unwrap_or_else(|| src.clone());
                    *incoming.entry(tgt.clone()).or_default().entry(lab).or_insert(0.0) += w;
                    in_count.entry(tgt.clone()).or_default().insert(src.clone());
                }
            }
        }
        let mut out: BTreeMap<String, (f64, f64, BTreeMap<String, f64>)> = BTreeMap::new();
        for tgt in nodes {
            match incoming.get(tgt) {
                None => {
                    out.insert(tgt.clone(), (0.0, 0.0, BTreeMap::new()));
                }
                Some(comm_w) => {
                    let total: f64 = comm_w.values().sum();
                    let shares: BTreeMap<String, f64> =
                        comm_w.iter().map(|(c, w)| (c.clone(), w / total)).collect();
                    let conc: f64 = shares.values().map(|s| s * s).sum();
                    let n = in_count[tgt].len() as f64;
                    let gate = n / (n + k0);
                    out.insert(tgt.clone(), (conc, gate, shares));
                }
            }
        }
        out
    }

    /// Row-stochastic local trust matrix C with anti-collusion damping (§1.6): independence + (opt-in)
    /// community-suspicion brake + (opt-in) in-concentration funnel damping. Normalised by the sum of
    /// the UNDAMPED weights, so an inbred row leaks mass out (sub-stochastic) instead of propagating it.
    #[cfg(feature = "std")]
    #[allow(clippy::too_many_arguments)]
    fn damped_local_matrix(
        &self,
        dim: &str,
        nodes: &[String],
        p: &Params,
        evidence: &BTreeMap<String, f64>,
        dim_evidence: &BTreeMap<String, f64>,
    ) -> BTreeMap<String, BTreeMap<String, f64>> {
        let node_set: BTreeSet<&String> = nodes.iter().collect();

        let use_comm = p.damping && p.community;
        let use_inc = p.damping && p.in_concentration;
        let label = if use_comm || use_inc {
            self.communities(dim, nodes)
        } else {
            BTreeMap::new()
        };
        let suspicion = if use_comm {
            self.community_suspicion(dim, nodes, &label, evidence)
        } else {
            BTreeMap::new()
        };
        let in_conc = if use_inc {
            self.in_concentration_signals(dim, nodes, &label, p.k0)
        } else {
            BTreeMap::new()
        };

        let mut c: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
        for i in nodes {
            let outgoing: BTreeMap<String, f64> = self
                .outgoing(i, dim)
                .into_iter()
                .filter(|(j, _)| node_set.contains(j))
                .collect();
            let raw_sum: f64 = outgoing.values().sum();
            if raw_sum <= 0.0 {
                c.insert(i.clone(), BTreeMap::new());
                continue;
            }
            let mut row = BTreeMap::new();
            for (j, w) in outgoing {
                let mut f = if p.damping {
                    self.independence(i, &j, dim, p.beta, p.gamma)
                } else {
                    1.0
                };
                if use_comm {
                    if let (Some(li), Some(lj)) = (label.get(i), label.get(&j)) {
                        if li == lj {
                            f *= 1.0 / (1.0 + p.kappa * *suspicion.get(li).unwrap_or(&0.0));
                        }
                    }
                }
                if use_inc {
                    if let Some((conc, gate, shares)) = in_conc.get(&j) {
                        let dom = label
                            .get(i)
                            .and_then(|li| shares.get(li))
                            .copied()
                            .unwrap_or(0.0);
                        let deficit = 1.0 / (1.0 + p.rho * *dim_evidence.get(&j).unwrap_or(&0.0));
                        f *= 1.0 / (1.0 + p.mu * conc * conc * dom * gate * deficit);
                    }
                }
                row.insert(j, w * f / raw_sum);
            }
            c.insert(i.clone(), row);
        }
        c
    }

    /// DETERMINISTIC fixed-point local trust matrix (i128, FP_SCALE-scaled), all factors computed in
    /// fixed-point (independence + community + in-concentration). The full-determinism counterpart of
    /// `damped_local_matrix`; feeds `reputation_dimension_fully_fixed`. No floats in the result.
    #[allow(clippy::too_many_arguments)]
    fn damped_local_matrix_fp(
        &self,
        dim: &str,
        nodes: &[String],
        p: &Params,
        evidence_fp: &BTreeMap<String, i128>,
        dim_evidence_fp: &BTreeMap<String, i128>,
    ) -> BTreeMap<String, BTreeMap<String, i128>> {
        let node_set: BTreeSet<&String> = nodes.iter().collect();
        let (beta_fp, gamma_fp) = (to_fp(p.beta), to_fp(p.gamma));
        let (kappa_fp, mu_fp, rho_fp) = (to_fp(p.kappa), to_fp(p.mu), to_fp(p.rho));

        let use_comm = p.damping && p.community;
        let use_inc = p.damping && p.in_concentration;
        let label = if use_comm || use_inc {
            self.communities(dim, nodes)
        } else {
            BTreeMap::new()
        };
        let suspicion = if use_comm {
            self.community_suspicion_fp(dim, nodes, &label, evidence_fp)
        } else {
            BTreeMap::new()
        };
        let in_conc = if use_inc {
            self.in_concentration_signals_fp(dim, nodes, &label, to_fp(p.k0))
        } else {
            BTreeMap::new()
        };

        let mut c: BTreeMap<String, BTreeMap<String, i128>> = BTreeMap::new();
        for i in nodes {
            let outgoing: BTreeMap<String, f64> = self
                .outgoing(i, dim)
                .into_iter()
                .filter(|(j, _)| node_set.contains(j))
                .collect();
            let raw_sum_fp: i128 = outgoing.values().map(|w| to_fp(*w)).sum();
            if raw_sum_fp <= 0 {
                c.insert(i.clone(), BTreeMap::new());
                continue;
            }
            let mut row = BTreeMap::new();
            for (j, w) in outgoing {
                let mut f = if p.damping {
                    self.independence_fp(i, &j, dim, beta_fp, gamma_fp)
                } else {
                    FP_SCALE
                };
                if use_comm {
                    if let (Some(li), Some(lj)) = (label.get(i), label.get(&j)) {
                        if li == lj {
                            let brake = fp_div(
                                FP_SCALE,
                                FP_SCALE + fp_mul(kappa_fp, *suspicion.get(li).unwrap_or(&0)),
                            );
                            f = fp_mul(f, brake);
                        }
                    }
                }
                if use_inc {
                    if let Some((conc, gate, shares)) = in_conc.get(&j) {
                        let dom = label.get(i).and_then(|li| shares.get(li)).copied().unwrap_or(0);
                        let deficit = fp_div(
                            FP_SCALE,
                            FP_SCALE + fp_mul(rho_fp, *dim_evidence_fp.get(&j).unwrap_or(&0)),
                        );
                        // 1 / (1 + mu·conc²·dom·gate·deficit)
                        let mut inner = fp_mul(*conc, *conc);
                        inner = fp_mul(inner, dom);
                        inner = fp_mul(inner, *gate);
                        inner = fp_mul(inner, deficit);
                        let term = fp_div(FP_SCALE, FP_SCALE + fp_mul(mu_fp, inner));
                        f = fp_mul(f, term);
                    }
                }
                // C[i][j] = (w / raw_sum) · f   (all fixed-point)
                row.insert(j.clone(), fp_mul(fp_div(to_fp(w), raw_sum_fp), f));
            }
            c.insert(i.clone(), row);
        }
        c
    }
}

/// Pre-trust p per dimension: normalised objective evidence (§1.3a) + genesis seed (§1.4).
/// Falls back to uniform among unique humans if there is no anchor at all (degenerate, avoids /0).
fn pretrust(agents: &[Agent], dim: &str, genesis_weight: f64) -> BTreeMap<String, f64> {
    let mut raw: BTreeMap<String, f64> = BTreeMap::new();
    for a in agents {
        let seed = if a.genesis { genesis_weight } else { 0.0 };
        raw.insert(a.id.clone(), a.evidence_in(dim) + seed);
    }
    let total: f64 = raw.values().sum();
    if total <= 0.0 {
        let humans: Vec<&Agent> = agents.iter().filter(|a| a.unique_human).collect();
        if humans.is_empty() {
            return agents.iter().map(|a| (a.id.clone(), 0.0)).collect();
        }
        let u = 1.0 / humans.len() as f64;
        return agents
            .iter()
            .map(|a| (a.id.clone(), if a.unique_human { u } else { 0.0 }))
            .collect();
    }
    raw.into_iter().map(|(k, v)| (k, v / total)).collect()
}

/// Parameters of the reputation computation (§1, §1.6). Defaults match the Python prototype.
pub struct Params {
    pub alpha: f64,       // weight of the evidence anchor (teleport)
    pub iterations: usize,
    pub tol: f64,
    pub scale: f64,
    pub damping: bool,
    pub community: bool,        // opt-in community-suspicion brake (§1.6, scattered rings)
    pub in_concentration: bool, // opt-in asymmetric-funnel damping (§2d)
    pub beta: f64,        // reciprocity penalty
    pub gamma: f64,       // overlap penalty
    pub kappa: f64,       // community-suspicion strength
    pub mu: f64,          // in-concentration strength
    pub k0: f64,          // in-concentration volume gate
    pub rho: f64,         // in-concentration evidence-deficit strength
    pub genesis_weight: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            alpha: 0.30,
            iterations: 200,
            tol: 1e-12,
            scale: 1000.0,
            damping: true,
            community: false,
            in_concentration: false,
            beta: 4.0,
            gamma: 4.0,
            kappa: 0.5,
            mu: 8.0,
            k0: 26.0,
            rho: 2.0,
            genesis_weight: 1.0,
        }
    }
}

/// EARNED reputation per agent in one dimension (gate 2, §1.4): EigenTrust with teleport to the
/// pre-trust (the evidence anchor) and the row-deficit reinjected towards the pre-trust.
#[cfg(feature = "std")]
pub fn reputation_dimension(
    agents: &[Agent],
    graph: &TrustGraph,
    dim: &str,
    p: &Params,
) -> BTreeMap<String, f64> {
    let nodes: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
    let pre = pretrust(agents, dim, p.genesis_weight);
    // TOTAL evidence per node (community suspicion) + PER-DIM evidence (funnel deficit, cross-dim).
    let total_evidence: BTreeMap<String, f64> =
        agents.iter().map(|a| (a.id.clone(), a.evidence.values().sum())).collect();
    let dim_evidence: BTreeMap<String, f64> =
        agents.iter().map(|a| (a.id.clone(), a.evidence_in(dim))).collect();
    let c = graph.damped_local_matrix(dim, &nodes, p, &total_evidence, &dim_evidence);
    let row_sum: BTreeMap<String, f64> =
        nodes.iter().map(|i| (i.clone(), c[i].values().sum())).collect();

    let mut t = pre.clone();
    for _ in 0..p.iterations {
        let mut nt: BTreeMap<String, f64> =
            nodes.iter().map(|n| (n.clone(), p.alpha * pre[n])).collect();
        let mut leak_total = 0.0;
        for i in &nodes {
            let ti = t[i];
            if ti == 0.0 {
                continue;
            }
            let emitted = (1.0 - p.alpha) * ti;
            for (j, w) in &c[i] {
                *nt.get_mut(j).unwrap() += emitted * w;
            }
            leak_total += emitted * (1.0 - row_sum[i]);
        }
        if leak_total != 0.0 {
            for n in &nodes {
                *nt.get_mut(n).unwrap() += leak_total * pre[n];
            }
        }
        let delta: f64 = nodes.iter().map(|n| (nt[n] - t[n]).abs()).sum();
        t = nt;
        if delta < p.tol {
            break;
        }
    }
    t.into_iter().map(|(k, v)| (k, v * p.scale)).collect()
}

/// Fixed-point scale for the deterministic arithmetic. 1e9 in i128.
pub const FP_SCALE: i128 = 1_000_000_000;

#[inline]
fn to_fp(x: f64) -> i128 {
    // Manual round-half-away-from-zero so this works in `no_std` (f64::round is a libm method the
    // runtime cannot use). f64 arithmetic and the float->int cast are available in `core`.
    let scaled = x * FP_SCALE as f64;
    let rounded = if scaled >= 0.0 { scaled + 0.5 } else { scaled - 0.5 };
    rounded as i128
}

/// Fixed-point multiply: (a·b) / SCALE. Inputs and output are FP_SCALE-scaled.
#[inline]
pub(crate) fn fp_mul(a: i128, b: i128) -> i128 {
    a * b / FP_SCALE
}

/// Fixed-point divide: (a·SCALE) / b. Inputs and output are FP_SCALE-scaled.
#[inline]
pub(crate) fn fp_div(a: i128, b: i128) -> i128 {
    a * FP_SCALE / b
}

/// DETERMINISTIC EigenTrust in fixed-point (i128). Same algorithm as `reputation_dimension`, but the
/// iteration runs in integer fixed-point so the result is bit-identical across architectures — a
/// requirement for on-chain consensus (f64 summation is not reproducible). Cross-validates against the
/// f64 path within a tiny tolerance.
///
/// NOTE (honest, milestone in progress): the local trust matrix C is still computed in f64 and then
/// quantised; converting the FACTOR math (independence/community/in-concentration) to fixed-point is
/// the next step toward full cross-architecture determinism (see `../PALLET-DESIGN.md`).
#[cfg(feature = "std")]
pub fn reputation_dimension_fixed(
    agents: &[Agent],
    graph: &TrustGraph,
    dim: &str,
    p: &Params,
) -> BTreeMap<String, f64> {
    let nodes: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
    let pre = pretrust(agents, dim, p.genesis_weight);
    let total_evidence: BTreeMap<String, f64> =
        agents.iter().map(|a| (a.id.clone(), a.evidence.values().sum())).collect();
    let dim_evidence: BTreeMap<String, f64> =
        agents.iter().map(|a| (a.id.clone(), a.evidence_in(dim))).collect();
    let c = graph.damped_local_matrix(dim, &nodes, p, &total_evidence, &dim_evidence);

    // quantise pre-trust and C to fixed-point
    let one = FP_SCALE;
    let alpha = to_fp(p.alpha);
    let pre_fp: BTreeMap<String, i128> = pre.iter().map(|(k, v)| (k.clone(), to_fp(*v))).collect();
    let c_fp: BTreeMap<String, BTreeMap<String, i128>> = c
        .iter()
        .map(|(i, row)| (i.clone(), row.iter().map(|(j, w)| (j.clone(), to_fp(*w))).collect()))
        .collect();
    let row_sum_fp: BTreeMap<String, i128> =
        nodes.iter().map(|i| (i.clone(), c_fp[i].values().sum())).collect();

    let mut t = pre_fp.clone();
    for _ in 0..p.iterations {
        let mut nt: BTreeMap<String, i128> =
            nodes.iter().map(|n| (n.clone(), alpha * pre_fp[n] / one)).collect();
        let mut leak: i128 = 0;
        for i in &nodes {
            let ti = t[i];
            if ti == 0 {
                continue;
            }
            let emitted = (one - alpha) * ti / one;
            for (j, w) in &c_fp[i] {
                *nt.get_mut(j).unwrap() += emitted * w / one;
            }
            leak += emitted * (one - row_sum_fp[i]) / one;
        }
        if leak != 0 {
            for n in &nodes {
                *nt.get_mut(n).unwrap() += leak * pre_fp[n] / one;
            }
        }
        let delta: i128 = nodes.iter().map(|n| (nt[n] - t[n]).abs()).sum();
        t = nt;
        if delta == 0 {
            break;
        }
    }
    t.into_iter().map(|(k, v)| (k, v as f64 / one as f64 * p.scale)).collect()
}

/// FULLY DETERMINISTIC reputation in one dimension, RAW fixed-point (i128, FP_SCALE-scaled). Every step
/// in fixed-point (factors AND the EigenTrust iteration), no f64 anywhere — the exact values a runtime
/// computes, before any f64 presentation scaling. The vector sums to ~FP_SCALE; only ratios matter for
/// committee weighting, so this feeds the sortition directly.
pub fn reputation_dimension_fully_fixed_fp(
    agents: &[Agent],
    graph: &TrustGraph,
    dim: &str,
    p: &Params,
) -> BTreeMap<String, i128> {
    let nodes: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
    let pre = pretrust(agents, dim, p.genesis_weight);
    let evidence_fp: BTreeMap<String, i128> =
        agents.iter().map(|a| (a.id.clone(), to_fp(a.evidence.values().sum()))).collect();
    let dim_evidence_fp: BTreeMap<String, i128> =
        agents.iter().map(|a| (a.id.clone(), to_fp(a.evidence_in(dim)))).collect();
    let c_fp = graph.damped_local_matrix_fp(dim, &nodes, p, &evidence_fp, &dim_evidence_fp);

    let one = FP_SCALE;
    let alpha = to_fp(p.alpha);
    let pre_fp: BTreeMap<String, i128> = pre.iter().map(|(k, v)| (k.clone(), to_fp(*v))).collect();
    let row_sum_fp: BTreeMap<String, i128> =
        nodes.iter().map(|i| (i.clone(), c_fp[i].values().sum())).collect();

    let mut t = pre_fp.clone();
    for _ in 0..p.iterations {
        let mut nt: BTreeMap<String, i128> =
            nodes.iter().map(|n| (n.clone(), alpha * pre_fp[n] / one)).collect();
        let mut leak: i128 = 0;
        for i in &nodes {
            let ti = t[i];
            if ti == 0 {
                continue;
            }
            let emitted = (one - alpha) * ti / one;
            for (j, w) in &c_fp[i] {
                *nt.get_mut(j).unwrap() += emitted * w / one;
            }
            leak += emitted * (one - row_sum_fp[i]) / one;
        }
        if leak != 0 {
            for n in &nodes {
                *nt.get_mut(n).unwrap() += leak * pre_fp[n] / one;
            }
        }
        let delta: i128 = nodes.iter().map(|n| (nt[n] - t[n]).abs()).sum();
        t = nt;
        if delta == 0 {
            break;
        }
    }
    t
}

/// FULLY DETERMINISTIC reputation in one dimension, presented as f64 (raw fixed-point × `scale`). Thin
/// wrapper over [`reputation_dimension_fully_fixed_fp`] for hosts/tests; same deterministic values.
pub fn reputation_dimension_fully_fixed(
    agents: &[Agent],
    graph: &TrustGraph,
    dim: &str,
    p: &Params,
) -> BTreeMap<String, f64> {
    let scale = p.scale / FP_SCALE as f64;
    reputation_dimension_fully_fixed_fp(agents, graph, dim, p)
        .into_iter()
        .map(|(k, v)| (k, v as f64 * scale))
        .collect()
}

/// EARNED reputation as a VECTOR over all four suits (§1.2b).
#[cfg(feature = "std")]
pub fn reputation_vector(
    agents: &[Agent],
    graph: &TrustGraph,
    p: &Params,
) -> BTreeMap<String, BTreeMap<String, f64>> {
    let mut per_dim: BTreeMap<&str, BTreeMap<String, f64>> = BTreeMap::new();
    for d in DIMENSIONS {
        per_dim.insert(d, reputation_dimension(agents, graph, d, p));
    }
    let mut out: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for a in agents {
        let v: BTreeMap<String, f64> =
            DIMENSIONS.iter().map(|d| (d.to_string(), per_dim[d][&a.id])).collect();
        out.insert(a.id.clone(), v);
    }
    out
}

/// Conservative aggregation of the vector (§1.2b): min (default) or mean — NEVER a sum. For powers
/// that need global reliability (consensus, vouching) a high suit does not buy a low one: you cannot
/// buy authority in one suit with another.
#[cfg(feature = "std")]
pub fn conservative_aggregate(vector: &BTreeMap<String, f64>, min: bool) -> f64 {
    if vector.is_empty() {
        return 0.0;
    }
    if min {
        vector.values().cloned().fold(f64::INFINITY, f64::min)
    } else {
        vector.values().sum::<f64>() / vector.len() as f64
    }
}

/// Decay by inactivity (§1.7): uncontributed reputation evaporates. Farming then sitting still does
/// not pay off long-term (extra anti-collusion defence). `r <- r * factor`.
pub fn decay(reputation: &BTreeMap<String, f64>, factor: f64) -> BTreeMap<String, f64> {
    reputation.iter().map(|(k, v)| (k.clone(), v * factor)).collect()
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn independence_fp_matches_f64() {
        let mut g = TrustGraph::new();
        g.attest("a", "b", "commerce", 1.0);
        g.attest("b", "a", "commerce", 1.0);
        g.attest("a", "x", "commerce", 1.0);
        g.attest("b", "x", "commerce", 1.0);
        g.attest("p", "m", "commerce", 1.0);
        g.attest("q", "n", "commerce", 1.0);
        let (bfp, gfp) = (to_fp(4.0), to_fp(4.0));
        for (i, j) in [("a", "b"), ("p", "q"), ("a", "x")] {
            let f = g.independence(i, j, "commerce", 4.0, 4.0);
            let x = g.independence_fp(i, j, "commerce", bfp, gfp) as f64 / FP_SCALE as f64;
            assert!((f - x).abs() < 1e-6, "{i}->{j}: f64 {f} vs fp {x}");
        }
    }

    #[test]
    fn community_suspicion_fp_matches_f64() {
        let mut g = TrustGraph::new();
        // ring + an honest with evidence
        g.attest("c0", "c1", "commerce", 1.0);
        g.attest("c1", "c2", "commerce", 1.0);
        g.attest("c2", "c0", "commerce", 1.0);
        g.attest("g0", "h0", "commerce", 1.0);
        let nodes: Vec<String> = ["c0", "c1", "c2", "g0", "h0"].iter().map(|s| s.to_string()).collect();
        let label = g.communities("commerce", &nodes);
        let mut ev = BTreeMap::new();
        ev.insert("h0".to_string(), 5.0);
        let ev_fp: BTreeMap<String, i128> = ev.iter().map(|(k, v)| (k.clone(), to_fp(*v))).collect();
        let f = g.community_suspicion("commerce", &nodes, &label, &ev);
        let x = g.community_suspicion_fp("commerce", &nodes, &label, &ev_fp);
        for (c, fv) in &f {
            let xv = x[c] as f64 / FP_SCALE as f64;
            assert!((fv - xv).abs() < 1e-6, "comm {c}: f64 {fv} vs fp {xv}");
        }
    }

    #[test]
    fn in_concentration_fp_matches_f64() {
        let mut g = TrustGraph::new();
        for i in 0..5 {
            g.attest(&format!("f{i}"), "c0", "commerce", 1.0);
        }
        g.attest("g0", "h0", "commerce", 1.0);
        let mut nodes: Vec<String> = (0..5).map(|i| format!("f{i}")).collect();
        nodes.push("c0".into());
        nodes.push("g0".into());
        nodes.push("h0".into());
        let label = g.communities("commerce", &nodes);
        let f = g.in_concentration_signals("commerce", &nodes, &label, 26.0);
        let x = g.in_concentration_signals_fp("commerce", &nodes, &label, to_fp(26.0));
        for (t, (conc, gate, _)) in &f {
            let (cx, gx, _) = &x[t];
            assert!((conc - *cx as f64 / FP_SCALE as f64).abs() < 1e-6, "{t} conc");
            assert!((gate - *gx as f64 / FP_SCALE as f64).abs() < 1e-6, "{t} gate");
        }
    }

    #[test]
    fn fixed_point_matches_f64() {
        // the deterministic fixed-point EigenTrust matches the f64 path within a tiny tolerance.
        let agents = vec![
            Agent::new("g0").genesis().with_evidence("commerce", 2.0),
            Agent::new("h0").with_evidence("commerce", 5.0),
            Agent::new("sybil"),
        ];
        let mut g = TrustGraph::new();
        g.attest("g0", "h0", "commerce", 1.0);
        let p = Params::default();
        let f = reputation_dimension(&agents, &g, "commerce", &p);
        let x = reputation_dimension_fixed(&agents, &g, "commerce", &p);
        for id in ["g0", "h0", "sybil"] {
            assert!((f[id] - x[id]).abs() < 0.5, "{id}: f64 {} vs fixed {}", f[id], x[id]);
        }
    }

    #[test]
    fn fully_fixed_matches_f64_with_all_defenses() {
        // funnel + community + in-concentration, the full anti-collusion path. The fully fixed-point
        // engine must match the f64 one within tolerance.
        let mut agents = vec![
            Agent::new("g0").genesis().with_evidence("commerce", 2.0),
            Agent::new("h0").with_evidence("commerce", 5.0),
            Agent::new("c0"),
        ];
        let mut g = TrustGraph::new();
        g.attest("g0", "h0", "commerce", 1.0);
        for i in 0..5 {
            let fid = format!("f{i}");
            agents.push(Agent::new(&fid).with_evidence("commerce", 1.0));
            g.attest(&fid, "c0", "commerce", 1.0);
        }
        let p = Params { community: true, in_concentration: true, ..Default::default() };
        let f = reputation_dimension(&agents, &g, "commerce", &p);
        let x = reputation_dimension_fully_fixed(&agents, &g, "commerce", &p);
        for a in &agents {
            assert!((f[&a.id] - x[&a.id]).abs() < 1.0, "{}: f64 {} vs fully-fixed {}", a.id, f[&a.id], x[&a.id]);
        }
    }

    #[test]
    fn fixed_point_is_deterministic() {
        // identical inputs -> bit-identical fixed-point output across repeated runs.
        let agents = vec![
            Agent::new("g0").genesis().with_evidence("commerce", 2.0),
            Agent::new("h0").with_evidence("commerce", 5.0),
        ];
        let mut g = TrustGraph::new();
        g.attest("g0", "h0", "commerce", 1.0);
        let p = Params::default();
        let a = reputation_dimension_fixed(&agents, &g, "commerce", &p);
        let b = reputation_dimension_fixed(&agents, &g, "commerce", &p);
        assert_eq!(a, b);
    }

    #[test]
    fn decay_evaporates_inactive() {
        let mut r = BTreeMap::new();
        r.insert("a".to_string(), 100.0);
        let d = decay(&r, 0.9);
        assert!((d["a"] - 90.0).abs() < 1e-9);
    }

    #[test]
    fn independence_penalises_inbreeding() {
        // a<->b reciprocal and both vouch the same cluster (high overlap) -> ~0.11; strangers -> ~1.0
        let mut g = TrustGraph::new();
        g.attest("a", "b", "commerce", 1.0);
        g.attest("b", "a", "commerce", 1.0);
        g.attest("a", "x", "commerce", 1.0);
        g.attest("b", "x", "commerce", 1.0);
        let inbred = g.independence("a", "b", "commerce", 4.0, 4.0);
        assert!(inbred < 0.2, "inbred vouch should be heavily damped, was {inbred}");
        g.attest("p", "m", "commerce", 1.0);
        g.attest("q", "n", "commerce", 1.0);
        let stranger = g.independence("p", "q", "commerce", 4.0, 4.0);
        assert!(stranger > 0.9, "stranger vouch should stay near 1, was {stranger}");
    }

    #[test]
    fn mass_is_conserved() {
        // EigenTrust with leak to the pre-trust conserves mass: the dimension sums to ~scale.
        let agents = vec![
            Agent::new("g0").genesis().with_evidence("commerce", 2.0),
            Agent::new("h0").with_evidence("commerce", 3.0),
            Agent::new("h1").with_evidence("commerce", 4.0),
        ];
        let mut g = TrustGraph::new();
        g.attest("g0", "h0", "commerce", 1.0);
        g.attest("h1", "h0", "commerce", 1.0);
        let rep = reputation_dimension(&agents, &g, "commerce", &Params::default());
        let total: f64 = rep.values().sum();
        assert!((total - 1000.0).abs() < 1.0, "mass not conserved: {total}");
    }

    #[test]
    fn matches_python_prototype() {
        // Cross-validation: identical scenario to the Python prototype must give identical reputation.
        // g0 (genesis, ev 2) -> h0 (ev 5); sybil isolated. Python: g0=297.03, h0=702.97, sybil=0.
        let agents = vec![
            Agent::new("g0").genesis().with_evidence("commerce", 2.0),
            Agent::new("h0").with_evidence("commerce", 5.0),
            Agent::new("sybil"),
        ];
        let mut g = TrustGraph::new();
        g.attest("g0", "h0", "commerce", 1.0);
        let rep = reputation_dimension(&agents, &g, "commerce", &Params::default());
        assert!((rep["g0"] - 297.0297).abs() < 0.01, "g0 diverged: {}", rep["g0"]);
        assert!((rep["h0"] - 702.9703).abs() < 0.01, "h0 diverged: {}", rep["h0"]);
        assert!(rep["sybil"].abs() < 0.01, "sybil should be 0: {}", rep["sybil"]);
    }

    #[test]
    fn community_crushes_collusion_ring_matches_python() {
        // ring c0->c1->c2->c0 (no evidence) with community damping -> all ~0; honest keeps its power.
        // Python: h0=699.2925, c0=c1=c2=0.0.
        let agents = vec![
            Agent::new("g0").genesis().with_evidence("commerce", 2.0),
            Agent::new("h0").with_evidence("commerce", 5.0),
            Agent::new("c0"),
            Agent::new("c1"),
            Agent::new("c2"),
        ];
        let mut g = TrustGraph::new();
        g.attest("g0", "h0", "commerce", 1.0);
        g.attest("c0", "c1", "commerce", 1.0);
        g.attest("c1", "c2", "commerce", 1.0);
        g.attest("c2", "c0", "commerce", 1.0);
        let p = Params { community: true, ..Default::default() };
        let rep = reputation_dimension(&agents, &g, "commerce", &p);
        assert!((rep["h0"] - 699.2925).abs() < 0.01, "h0 diverged: {}", rep["h0"]);
        for c in ["c0", "c1", "c2"] {
            assert!(rep[c].abs() < 0.01, "{c} should be ~0, was {}", rep[c]);
        }
    }

    #[test]
    fn in_concentration_cuts_funnel_matches_python() {
        // 5 feeders funnel onto c0 (no evidence). With the in-concentration signal Python gives
        // c0=67.4052, h0=432.6991.
        let mut agents = vec![
            Agent::new("g0").genesis().with_evidence("commerce", 2.0),
            Agent::new("h0").with_evidence("commerce", 5.0),
            Agent::new("c0"),
        ];
        let mut g = TrustGraph::new();
        g.attest("g0", "h0", "commerce", 1.0);
        for i in 0..5 {
            let fid = format!("f{i}");
            agents.push(Agent::new(&fid).with_evidence("commerce", 1.0));
            g.attest(&fid, "c0", "commerce", 1.0);
        }
        let p = Params { community: true, in_concentration: true, ..Default::default() };
        let rep = reputation_dimension(&agents, &g, "commerce", &p);
        assert!((rep["c0"] - 67.4052).abs() < 0.01, "c0 diverged: {}", rep["c0"]);
        assert!((rep["h0"] - 432.6991).abs() < 0.01, "h0 diverged: {}", rep["h0"]);
    }

    #[test]
    fn sybil_without_evidence_or_vouches_has_no_power() {
        // a fake identity with no evidence and no vouches from the reputed -> reputation ~0.
        let agents = vec![
            Agent::new("g0").genesis().with_evidence("commerce", 2.0),
            Agent::new("h0").with_evidence("commerce", 5.0),
            Agent::new("sybil"),
        ];
        let mut g = TrustGraph::new();
        g.attest("g0", "h0", "commerce", 1.0);
        let rep = reputation_dimension(&agents, &g, "commerce", &Params::default());
        assert!(rep["sybil"] < 1.0, "sybil should have ~0 reputation, was {}", rep["sybil"]);
        assert!(rep["h0"] > rep["sybil"] * 10.0, "honest must dominate the sybil");
    }
}
