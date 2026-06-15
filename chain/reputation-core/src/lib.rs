//! Harlequin reputation engine — Rust port of the validated Python prototype
//! (`prototipos/reputacion/`, 17/17 tests). This is the foundation the Substrate **reputation pallet**
//! will build on: the same EigenTrust-with-anti-collusion-damping core, in `no_std`-friendly Rust.
//!
//! SPEC.md anchors: §1 (reputation), §1.6 (anti-collusion damping). The four reputation dimensions are
//! the four suits of Harlequin (LORE.md): commerce ♦, technical_contribution ♣, judicial_function ♠,
//! governance ♥. This v0 ports the BASE engine (independence damping + EigenTrust anchored in
//! evidence); the community / in-concentration signals (RESULTS §2b–2d) land in later increments.

use std::collections::HashMap;

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
    pub evidence: HashMap<String, f64>,
}

impl Agent {
    pub fn new(id: &str) -> Self {
        Agent { id: id.into(), unique_human: true, genesis: false, evidence: HashMap::new() }
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
    edges: HashMap<String, HashMap<String, HashMap<String, f64>>>,
}

impl TrustGraph {
    pub fn new() -> Self {
        TrustGraph { edges: HashMap::new() }
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

    fn outgoing(&self, source: &str, dim: &str) -> HashMap<String, f64> {
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

    /// Community detection by label propagation over the undirected projection (§1.6). Deterministic
    /// (sorted order + smallest-label tie-break) so it is reproducible — matches the Python prototype.
    pub fn communities(&self, dim: &str, nodes: &[String]) -> HashMap<String, String> {
        let node_set: std::collections::HashSet<&String> = nodes.iter().collect();
        let mut adj: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
        for i in nodes {
            for (j, _) in self.outgoing(i, dim) {
                if node_set.contains(&j) {
                    adj.entry(i.clone()).or_default().insert(j.clone());
                    adj.entry(j.clone()).or_default().insert(i.clone());
                }
            }
        }
        let mut label: HashMap<String, String> =
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
                let mut count: HashMap<String, usize> = HashMap::new();
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
    pub fn community_suspicion(
        &self,
        dim: &str,
        nodes: &[String],
        label: &HashMap<String, String>,
        evidence: &HashMap<String, f64>,
    ) -> HashMap<String, f64> {
        let node_set: std::collections::HashSet<&String> = nodes.iter().collect();
        let mut internal: HashMap<String, f64> = HashMap::new();
        for i in nodes {
            for (j, _) in self.outgoing(i, dim) {
                if node_set.contains(&j) && label[i] == label[&j] {
                    *internal.entry(label[i].clone()).or_insert(0.0) += 1.0;
                }
            }
        }
        let mut ev: HashMap<String, f64> = HashMap::new();
        for n in nodes {
            *ev.entry(label[n].clone()).or_insert(0.0) += *evidence.get(n).unwrap_or(&0.0);
        }
        let comms: std::collections::HashSet<&String> = label.values().collect();
        comms
            .into_iter()
            .map(|c| {
                let e = *internal.get(c).unwrap_or(&0.0) / (1.0 + *ev.get(c).unwrap_or(&0.0));
                (c.clone(), e)
            })
            .collect()
    }

    /// Asymmetric-funnel signal (§2d): per target, (concentration HHI over source communities, volume
    /// gate, shares per community). Cuts a directed PageRank funnel local independence misses.
    pub fn in_concentration_signals(
        &self,
        dim: &str,
        nodes: &[String],
        label: &HashMap<String, String>,
        k0: f64,
    ) -> HashMap<String, (f64, f64, HashMap<String, f64>)> {
        let node_set: std::collections::HashSet<&String> = nodes.iter().collect();
        let mut incoming: HashMap<String, HashMap<String, f64>> = HashMap::new();
        let mut in_count: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
        for src in nodes {
            for (tgt, w) in self.outgoing(src, dim) {
                if node_set.contains(&tgt) && w > 0.0 {
                    let lab = label.get(src).cloned().unwrap_or_else(|| src.clone());
                    *incoming.entry(tgt.clone()).or_default().entry(lab).or_insert(0.0) += w;
                    in_count.entry(tgt.clone()).or_default().insert(src.clone());
                }
            }
        }
        let mut out: HashMap<String, (f64, f64, HashMap<String, f64>)> = HashMap::new();
        for tgt in nodes {
            match incoming.get(tgt) {
                None => {
                    out.insert(tgt.clone(), (0.0, 0.0, HashMap::new()));
                }
                Some(comm_w) => {
                    let total: f64 = comm_w.values().sum();
                    let shares: HashMap<String, f64> =
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
    #[allow(clippy::too_many_arguments)]
    fn damped_local_matrix(
        &self,
        dim: &str,
        nodes: &[String],
        p: &Params,
        evidence: &HashMap<String, f64>,
        dim_evidence: &HashMap<String, f64>,
    ) -> HashMap<String, HashMap<String, f64>> {
        let node_set: std::collections::HashSet<&String> = nodes.iter().collect();

        let use_comm = p.damping && p.community;
        let use_inc = p.damping && p.in_concentration;
        let label = if use_comm || use_inc {
            self.communities(dim, nodes)
        } else {
            HashMap::new()
        };
        let suspicion = if use_comm {
            self.community_suspicion(dim, nodes, &label, evidence)
        } else {
            HashMap::new()
        };
        let in_conc = if use_inc {
            self.in_concentration_signals(dim, nodes, &label, p.k0)
        } else {
            HashMap::new()
        };

        let mut c: HashMap<String, HashMap<String, f64>> = HashMap::new();
        for i in nodes {
            let outgoing: HashMap<String, f64> = self
                .outgoing(i, dim)
                .into_iter()
                .filter(|(j, _)| node_set.contains(j))
                .collect();
            let raw_sum: f64 = outgoing.values().sum();
            if raw_sum <= 0.0 {
                c.insert(i.clone(), HashMap::new());
                continue;
            }
            let mut row = HashMap::new();
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
}

/// Pre-trust p per dimension: normalised objective evidence (§1.3a) + genesis seed (§1.4).
/// Falls back to uniform among unique humans if there is no anchor at all (degenerate, avoids /0).
fn pretrust(agents: &[Agent], dim: &str, genesis_weight: f64) -> HashMap<String, f64> {
    let mut raw: HashMap<String, f64> = HashMap::new();
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
pub fn reputation_dimension(
    agents: &[Agent],
    graph: &TrustGraph,
    dim: &str,
    p: &Params,
) -> HashMap<String, f64> {
    let nodes: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
    let pre = pretrust(agents, dim, p.genesis_weight);
    // TOTAL evidence per node (community suspicion) + PER-DIM evidence (funnel deficit, cross-dim).
    let total_evidence: HashMap<String, f64> =
        agents.iter().map(|a| (a.id.clone(), a.evidence.values().sum())).collect();
    let dim_evidence: HashMap<String, f64> =
        agents.iter().map(|a| (a.id.clone(), a.evidence_in(dim))).collect();
    let c = graph.damped_local_matrix(dim, &nodes, p, &total_evidence, &dim_evidence);
    let row_sum: HashMap<String, f64> =
        nodes.iter().map(|i| (i.clone(), c[i].values().sum())).collect();

    let mut t = pre.clone();
    for _ in 0..p.iterations {
        let mut nt: HashMap<String, f64> =
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

/// Decay by inactivity (§1.7): uncontributed reputation evaporates. Farming then sitting still does
/// not pay off long-term (extra anti-collusion defence). `r <- r * factor`.
pub fn decay(reputation: &HashMap<String, f64>, factor: f64) -> HashMap<String, f64> {
    reputation.iter().map(|(k, v)| (k.clone(), v * factor)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_evaporates_inactive() {
        let mut r = HashMap::new();
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
