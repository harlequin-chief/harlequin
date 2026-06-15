"""
Trust graph: the attestations (vouches) between pseudonyms, per dimension.

Anchored in SPEC.md:
- §1.3b: vouching = putting reputation at stake. Here a vouch is a directed, weighted edge.
- §1.6: ANTI-COLLUSION. Reputation arriving from a closed ring of mutual voters is worth little;
  reputation from independent, already-reputed sources is worth more. Implemented with
  `independence(i, j)`: it penalises reciprocity (i<->j) and neighbour overlap (living in the same
  cluster). An inbred vouch tends to 0.
"""

from __future__ import annotations

from collections import Counter, defaultdict


class TrustGraph:
    """Attestation edges per dimension: (source -> target, weight) within a dimension."""

    def __init__(self) -> None:
        # dim -> source -> target -> weight
        self._edges: dict[str, dict[str, dict[str, float]]] = defaultdict(
            lambda: defaultdict(dict)
        )

    def attest(self, source: str, target: str, dim: str, weight: float = 1.0) -> None:
        """source vouches for target in dimension dim (§1.3b). Adds if already present."""
        if source == target:
            return  # nobody vouches for themselves
        current = self._edges[dim][source].get(target, 0.0)
        self._edges[dim][source][target] = current + weight

    def outgoing(self, source: str, dim: str) -> dict[str, float]:
        return dict(self._edges[dim].get(source, {}))

    def out_neighbors(self, node: str, dim: str) -> set[str]:
        return set(self._edges[dim].get(node, {}).keys())

    def has_edge(self, source: str, target: str, dim: str) -> bool:
        return target in self._edges[dim].get(source, {})

    def independence(
        self,
        i: str,
        j: str,
        dim: str,
        beta: float = 4.0,
        gamma: float = 4.0,
    ) -> float:
        """
        Independence factor of the vouch i->j in [0, 1] (§1.6).

        Penalises two collusion signatures:
          - reciprocity: if j also vouches for i (mutual-vouching ring).
          - neighbour overlap (Jaccard of whom i and j vouch for): living in the same closed cluster
            -> vouching for the same people -> inbreeding.

        independence = 1 / (1 + beta*reciprocal + gamma*overlap)
        A vouch between two members of a closed ring (reciprocal=1, high overlap) falls to ~0.14 or
        less; a vouch between strangers well connected to the rest of the network stays near 1.
        """
        reciprocal = 1.0 if self.has_edge(j, i, dim) else 0.0

        ni = self.out_neighbors(i, dim) - {j}
        nj = self.out_neighbors(j, dim) - {i}
        union = ni | nj
        overlap = (len(ni & nj) / len(union)) if union else 0.0

        return 1.0 / (1.0 + beta * reciprocal + gamma * overlap)

    def communities(self, dim: str, nodes: list[str]) -> dict[str, str]:
        """
        Community detection by **label propagation** over the undirected projection of the vouches.
        A GLOBAL signal that a collusion ring leaves even when it is scattered (low local inbreeding):
        it is still a densely interconnected community. Deterministic (fixed order and tie-breaks) ->
        reproducible.
        """
        node_set = set(nodes)
        adj: dict[str, set[str]] = defaultdict(set)
        for i in nodes:
            for j in self.outgoing(i, dim):
                if j in node_set:
                    adj[i].add(j)
                    adj[j].add(i)
        label = {n: n for n in nodes}
        order = sorted(nodes)
        for _ in range(15):
            changed = False
            for n in order:
                neighbors = adj.get(n)
                if not neighbors:
                    continue
                count = Counter(label[m] for m in neighbors)
                best = max(sorted(count), key=lambda k: count[k])
                if label[n] != best:
                    label[n] = best
                    changed = True
            if not changed:
                break
        return label

    def community_suspicion(
        self, dim: str, nodes: list[str], label: dict[str, str], evidence: dict[str, float]
    ) -> dict[str, float]:
        """
        Community suspicion = internal edges / (1 + community evidence). High when there is a lot of
        mutual vouching and little real work -> collusion signature, valid for DENSE and SCATTERED
        rings (unlike local independence, which only sees cliques).
        """
        internal_edges: Counter = Counter()
        node_set = set(nodes)
        for i in nodes:
            for j in self.outgoing(i, dim):
                if j in node_set and label[i] == label[j]:
                    internal_edges[label[i]] += 1
        ev: dict[str, float] = defaultdict(float)
        for n in nodes:
            ev[label[n]] += evidence.get(n, 0.0)
        return {
            comm: internal_edges.get(comm, 0) / (1.0 + ev.get(comm, 0.0))
            for comm in set(label.values())
        }

    def in_concentration_signals(
        self,
        dim: str,
        nodes: list[str],
        label: dict[str, str],
        k0: float = 26.0,
    ) -> dict[str, tuple[float, float, dict[str, float]]]:
        """
        ASYMMETRIC-FUNNEL signal (§1.6, frontier §2d). Closes the gap that local independence and
        community suspicion leave open: a directed PageRank funnel onto a single target c0 (c0 vouches
        for nobody, so reciprocity=0 and neighbour overlap=0 -> independence(feeder->c0)=1).

        For each target j it measures **where its incoming attestation weight comes from, grouped by
        community**, and returns (concentration, gate, shares):
          - shares[c] = fraction of j's incoming weight coming from community c.
          - concentration = Herfindahl index (sum shares^2) in [1/#comms, 1]. ~1 = a single community
            funnels almost all of j's inflow (funnel signature). Low = inflow spread across many
            communities (legitimately popular honest node) -> NOT penalised. THIS is the false-positive
            guard: honest popularity is diverse-source, a funnel is single-source.
          - gate = in_count/(in_count+k0) in [0,1): ramps with the NUMBER of distinct vouchers. A node
            with 1-2 vouchers (a normal newcomer that happens to share a community) gets gate~0 -> no
            penalty; a funnel needs many feeders to pump -> gate~1. Second false-positive guard.

        KEY: the very act of funnelling (all feeders pointing at c0) binds the feeders + c0 into one
        tight star-community under label propagation, so the funnel CREATES the concentration this
        signal detects. Diversifying the feeders to escape it costs pump per feeder.
        """
        node_set = set(nodes)
        incoming: dict[str, dict[str, float]] = defaultdict(lambda: defaultdict(float))
        in_count: dict[str, set] = defaultdict(set)
        for src in nodes:
            for tgt, w in self.outgoing(src, dim).items():
                if tgt in node_set and w > 0:
                    incoming[tgt][label.get(src, src)] += w
                    in_count[tgt].add(src)
        out: dict[str, tuple[float, float, dict[str, float]]] = {}
        for tgt in nodes:
            comm_w = incoming.get(tgt)
            if not comm_w:
                out[tgt] = (0.0, 0.0, {})
                continue
            total = sum(comm_w.values())
            shares = {c: w / total for c, w in comm_w.items()}
            conc = sum(s * s for s in shares.values())
            n = len(in_count[tgt])
            gate = n / (n + k0)
            out[tgt] = (conc, gate, shares)
        return out

    def damped_local_matrix(
        self,
        dim: str,
        nodes: list[str],
        damping: bool = True,
        community: bool = False,
        in_concentration: bool = False,
        evidence: dict[str, float] | None = None,
        kappa: float = 0.5,
        mu: float = 8.0,
        k0: float = 26.0,
        rho: float = 2.0,
        dim_evidence: dict[str, float] | None = None,
    ) -> dict[str, dict[str, float]]:
        """
        Local trust matrix C, row-stochastic, with anti-collusion damping applied (§1.6).

        C[i][j] = weight(i->j) * independence(i, j), normalised so that row i sums to 1.
        If i vouches for nobody (empty row), it is left empty -> EigenTrust treats it as "dangling"
        and redistributes its mass towards the pre-trust (anchoring in real evidence).

        `damping=False` disables the independence factor (useful to MEASURE how much the
        anti-collusion contributes: compare reputation with vs without damping under a collusion ring).

        KEY (design fix): normalisation uses the sum of the UNDAMPED weights. So when a node only
        vouches inside an inbred ring (low independence on ALL its edges), its row sums to << 1 -> it
        is SUB-stochastic -> most of its trust "leaks out" instead of propagating to the ring. If we
        normalised by the already-damped sum, a uniform factor would cancel and the damping would do
        nothing. The row deficit (1 - sum) is reinjected by the reputation computation towards the
        pre-trust (anchoring in evidence).
        """
        node_set = set(nodes)

        # community damping (opt-in): a global factor punishing edges inside a suspicious community
        # (many internal edges, little evidence). Closes the scattered-ring gap.
        label: dict[str, str] = {}
        suspicion: dict[str, float] = {}
        in_conc: dict[str, tuple[float, float, dict[str, float]]] = {}
        ev = evidence or {}
        # deficit uses PER-DIMENSION evidence (falls back to total): you cannot buy authority in one
        # suit with another, so a funnel in this dim onto a target with no evidence IN THIS DIM is cut
        # even if the target is established elsewhere (closes the cross-dimension funnel, RESULTS §2d).
        dev = dim_evidence if dim_evidence is not None else ev
        use_community = damping and community
        use_in_conc = damping and in_concentration
        if use_community or use_in_conc:
            label = self.communities(dim, nodes)
        if use_community and evidence is not None:
            suspicion = self.community_suspicion(dim, nodes, label, evidence)
        if use_in_conc:
            in_conc = self.in_concentration_signals(dim, nodes, label, k0=k0)

        def factor(i: str, j: str) -> float:
            if not damping:
                return 1.0
            f = self.independence(i, j, dim)
            if use_community and label.get(i) == label.get(j):
                f *= 1.0 / (1.0 + kappa * suspicion.get(label[i], 0.0))
            if use_in_conc:
                conc, gate, shares = in_conc.get(j, (0.0, 0.0, {}))
                dom = shares.get(label.get(i), 0.0)
                # discriminator: a funnel pumps an UNEARNED target. Weight by the target's evidence
                # DEFICIT: concentration into a node that earned its own evidence (a legitimately
                # popular member) is NOT funnelling; concentration into a 0-evidence target is.
                deficit = 1.0 / (1.0 + rho * dev.get(j, 0.0))
                f *= 1.0 / (1.0 + mu * (conc * conc) * dom * gate * deficit)
            return f

        C: dict[str, dict[str, float]] = {}
        for i in nodes:
            outgoing = {j: w for j, w in self.outgoing(i, dim).items() if j in node_set}
            raw_sum = sum(outgoing.values())
            if raw_sum <= 0:
                C[i] = {}
                continue
            C[i] = {j: w * factor(i, j) / raw_sum for j, w in outgoing.items()}
        return C
