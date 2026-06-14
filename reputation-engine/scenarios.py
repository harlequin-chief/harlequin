"""
Network scenarios for the attack simulations.

Each scenario builds a population of pseudonyms + a vouch graph and labels the factions so we can
measure how much power each kind of actor captures. Everything seeded with a PRNG -> reproducible.

Scenarios:
  1. honest               — baseline: genesis + honest members with real, independent evidence.
  2. sybil                — base + 200 fake identities (broken personhood, worst case) with no evidence.
  3. collusion            — base + a ring of 30 vouching in a circle to farm reputation.
  4. scattered_collusion  — sophisticated: a sparse ring with low inbreeding (open frontier §1.6).
  5. adaptive_collusion   — fragments the ring to evade community detection (§1.6).
  6. asymmetric_collusion — a directed PageRank funnel onto one target (§1.6).
  7. whitewashing         — a consolidated vs a fresh pseudonym: what is lost on restart.
"""

from __future__ import annotations

import random
from dataclasses import dataclass

from harlequin_rep.graph import TrustGraph
from harlequin_rep.model import DIMENSIONS, Agent, AgentKind


@dataclass
class Scenario:
    name: str
    description: str
    agents: list[Agent]
    graph: TrustGraph
    factions: dict[str, str]  # id -> faction ("genesis"/"honest"/"sybil"/"colluder"/"whitewasher")


def _build_base(rng: random.Random) -> tuple[list[Agent], TrustGraph, dict[str, str]]:
    """
    Honest base network: 5 genesis + 40 honest.

    - Genesis: moderate evidence in every dimension (seed, §1.4) and vouch for those they vetted.
    - Honest: real evidence in 1-2 dimensions (settled deals, §1.3a). They receive vouches from
      several INDEPENDENT counterparties (low overlap -> high independence, §1.6).
    """
    agents: list[Agent] = []
    factions: dict[str, str] = {}
    graph = TrustGraph()

    genesis = [f"g{i}" for i in range(5)]
    for gid in genesis:
        a = Agent(
            id=gid,
            kind=AgentKind.GENESIS,
            unique_human=True,
            evidence={d: 2.0 for d in DIMENSIONS},
        )
        agents.append(a)
        factions[gid] = "genesis"

    honest = [f"h{i}" for i in range(40)]
    for hid in honest:
        ndims = rng.randint(2, 3)
        dims = rng.sample(DIMENSIONS, ndims)
        evidence = {d: rng.uniform(1.0, 5.0) for d in dims}
        a = Agent(id=hid, kind=AgentKind.HONEST, unique_human=True, evidence=evidence)
        agents.append(a)
        factions[hid] = "honest"

    # genesis vouch for a handful of honest members who "vetted" (cohort sponsorship, §1.4)
    established = genesis + honest
    for gid in genesis:
        for hid in rng.sample(honest, 4):
            for d in DIMENSIONS:
                graph.attest(gid, hid, d, 1.0)

    # each honest member receives vouches from 2-4 INDEPENDENT counterparties in their evidence
    # dimension(s) (whoever did the deal attests, §1.3a). Distinct vouchers -> low overlap -> high
    # independence (§1.6).
    for a in agents:
        if a.kind is not AgentKind.HONEST:
            continue
        for d in a.evidence:
            k = rng.randint(2, 4)
            vouchers = rng.sample([e for e in established if e != a.id], k)
            for v in vouchers:
                graph.attest(v, a.id, d, 1.0)

    return agents, graph, factions


def scenario_honest(seed: int = 7) -> Scenario:
    rng = random.Random(seed)
    agents, graph, factions = _build_base(rng)
    return Scenario(
        name="honest",
        description="Baseline: genesis + 40 honest members with real evidence and independent vouches.",
        agents=agents,
        graph=graph,
        factions=factions,
    )


def scenario_sybil(seed: int = 7, n_sybil: int = 200) -> Scenario:
    """
    Worst-case Sybil: the 200 fake accounts DO pass the personhood test (unique_human True) — we
    assume AI broke the base gate (§1.5). But they have no real evidence nor vouches from the reputed;
    they only vouch a little among themselves. Thesis (§1.5, §2.4): power ~ 0.
    """
    rng = random.Random(seed)
    agents, graph, factions = _build_base(rng)

    sybils = [f"s{i}" for i in range(n_sybil)]
    for sid in sybils:
        agents.append(Agent(id=sid, kind=AgentKind.SYBIL, unique_human=True, evidence={}))
        factions[sid] = "sybil"

    # they vouch among themselves at random (no real cost), but no reputed account vouches for them
    for sid in sybils:
        for other in rng.sample(sybils, 3):
            if other != sid:
                graph.attest(sid, other, "commerce", 1.0)

    return Scenario(
        name="sybil",
        description=f"Base + {n_sybil} fake identities (broken personhood, worst case) with no evidence.",
        agents=agents,
        graph=graph,
        factions=factions,
    )


def scenario_collusion(seed: int = 7, ring_size: int = 30, fooled_honest: int = 3) -> Scenario:
    """
    Collusion ring (§1.6): 30 accounts vouching in a circle (dense reciprocal clique) to farm
    reputation, with no real work. To make it HARD, 3 "fooled" honest members each vouch for a ring
    member (injecting real trust). Without damping the ring would recirculate that injection and
    inflate; with damping (independence ~0 on mutual edges) it cannot.
    """
    rng = random.Random(seed)
    agents, graph, factions = _build_base(rng)

    ring = [f"c{i}" for i in range(ring_size)]
    for idx, cid in enumerate(ring):
        # c0 DID real work (has evidence): the anchor the ring will try to amplify and split among
        # the other 29 via mutual vouches. The damping (§1.6) must prevent that propagation: c0 keeps
        # ITS legitimate reputation, the rest stay at ~0.
        evidence = {"commerce": 20.0} if idx == 0 else {}
        agents.append(Agent(id=cid, kind=AgentKind.COLLUDER, unique_human=True,
                            evidence=evidence, cluster="ring1"))
        factions[cid] = "colluder"

    # reciprocal clique: each colluder vouches for every other ring member in commerce (farm)
    for a in ring:
        for b in ring:
            if a != b:
                graph.attest(a, b, "commerce", 1.0)

    # injection: 3 fooled honest members vouch for 3 ring members
    honest = [aid for aid, f in factions.items() if f == "honest"]
    for hid, cid in zip(rng.sample(honest, fooled_honest), ring):
        graph.attest(hid, cid, "commerce", 1.0)

    return Scenario(
        name="collusion",
        description=f"Base + a ring of {ring_size} vouching in a circle + {fooled_honest} fooled "
        f"honest members vouching for it.",
        agents=agents,
        graph=graph,
        factions=factions,
    )


def scenario_scattered_collusion(
    seed: int = 7, ring_size: int = 30, degree: int = 3, fooled_honest: int = 3
) -> Scenario:
    """
    SOPHISTICATED collusion (open frontier §1.6): instead of a dense clique (everyone vouches for
    everyone), a SCATTERED ring where each colluder vouches for only `degree` others, chosen to
    minimise reciprocity and neighbour overlap (mimicking honest patterns). The hardest to detect:
    few obvious inbreeding signatures. c0 has real reputation the ring tries to launder. Used to
    MEASURE whether the damping (designed against cliques) also stops the scattered ring, or leaks
    (revealing the frontier).
    """
    rng = random.Random(seed)
    agents, graph, factions = _build_base(rng)

    ring = [f"c{i}" for i in range(ring_size)]
    for idx, cid in enumerate(ring):
        evidence = {"commerce": 20.0} if idx == 0 else {}
        agents.append(Agent(id=cid, kind=AgentKind.COLLUDER, unique_human=True,
                            evidence=evidence, cluster="scattered_ring"))
        factions[cid] = "colluder"

    # scattered ring: each colluder vouches for `degree` others, avoiding direct reciprocity when
    # possible (lowers the inbreeding signature). A sparse directed random-graph topology.
    for a in ring:
        candidates = [b for b in ring if b != a]
        for b in rng.sample(candidates, min(degree, len(candidates))):
            graph.attest(a, b, "commerce", 1.0)

    honest = [aid for aid, f in factions.items() if f == "honest"]
    for hid, cid in zip(rng.sample(honest, fooled_honest), ring):
        graph.attest(hid, cid, "commerce", 1.0)

    return Scenario(
        name="scattered_collusion",
        description=f"SCATTERED ring of {ring_size} (degree {degree}, low inbreeding) + {fooled_honest} "
        f"fooled honest members. Sophisticated collusion (frontier §1.6).",
        agents=agents,
        graph=graph,
        factions=factions,
    )


def scenario_adaptive_collusion(
    seed: int = 7,
    ring_size: int = 30,
    n_fragments: int = 3,
    degree: int = 3,
    bridges: int = 1,
    fooled_honest: int = 3,
) -> Scenario:
    """
    ADAPTIVE collusion (open frontier §1.6, evading community detection).

    The attacker knows the defence labels communities and punishes dense-without-evidence ones. Its
    countermeasure: **fragment** the ring into `n_fragments` small sub-rings, each internally SCATTERED
    (every member vouches for `degree` others of ITS fragment). So each fragment looks like a small,
    low-inbreeding community, below the radar of the community suspicion.

    BUT the attacker still wants to LAUNDER the real reputation of c0 (its only node with evidence) to
    the 29 puppets. For reputation to FLOW between fragments it must connect them: `bridges` directed
    edges between consecutive fragments (c0 is in fragment 0). That is the tension this scenario
    measures: **more fragmentation evades the community label better, but chokes the reputation flow**
    (fewer bridges, scattered sub-rings -> local independence and the bridge bottlenecks cut the
    laundering anyway).

    Degenerate case `n_fragments=1` == the classic scattered ring (serves as the sweep control).
    """
    rng = random.Random(seed)
    agents, graph, factions = _build_base(rng)

    ring = [f"c{i}" for i in range(ring_size)]
    for idx, cid in enumerate(ring):
        evidence = {"commerce": 20.0} if idx == 0 else {}
        agents.append(Agent(id=cid, kind=AgentKind.COLLUDER, unique_human=True,
                            evidence=evidence, cluster="adaptive_ring"))
        factions[cid] = "colluder"

    # split the ring into n_fragments interleaved groups (c0 lands in fragment 0, with the evidence)
    n_fragments = max(1, min(n_fragments, ring_size))
    fragments: list[list[str]] = [ring[i::n_fragments] for i in range(n_fragments)]
    # note: interleaved cut keeps c0 in fragment 0 and spreads the puppets uniformly.

    # SCATTERED internal vouches within each fragment (low local inbreeding, small community)
    for frag in fragments:
        for a in frag:
            candidates = [b for b in frag if b != a]
            if not candidates:
                continue
            for b in rng.sample(candidates, min(degree, len(candidates))):
                graph.attest(a, b, "commerce", 1.0)

    # BRIDGES between consecutive fragments: c0's reputation must pass through here to launder.
    # Few bridges -> bottleneck; the total flow is bounded by their damped capacity.
    for fi in range(len(fragments) - 1):
        src_frag, dst_frag = fragments[fi], fragments[fi + 1]
        for _ in range(bridges):
            o = rng.choice(src_frag)
            d = rng.choice(dst_frag)
            graph.attest(o, d, "commerce", 1.0)

    honest = [aid for aid, f in factions.items() if f == "honest"]
    for hid, cid in zip(rng.sample(honest, fooled_honest), ring):
        graph.attest(hid, cid, "commerce", 1.0)

    return Scenario(
        name="adaptive_collusion",
        description=f"Ring of {ring_size} fragmented into {n_fragments} scattered sub-rings "
        f"(degree {degree}, {bridges} bridge(s)/pair) evading the community label. Measures the "
        f"evasion<->flow tension (frontier §1.6).",
        agents=agents,
        graph=graph,
        factions=factions,
    )


def scenario_asymmetric_collusion(
    seed: int = 7,
    n_feeders: int = 25,
    feeder_evidence: float = 4.0,
    diversify: int = 0,
    fooled_honest: int = 3,
) -> Scenario:
    """
    ASYMMETRIC / PageRank-pump collusion (frontier §1.6): instead of a reciprocal ring, a directed
    FUNNEL. `n_feeders` accounts all vouch for the same target `c0` WITHOUT reciprocity, to evade the
    reciprocity penalty of `independence()`. c0 has no evidence: all its reputation would come from
    the funnel. Realistic hard case: the feeders DID some real work (`feeder_evidence`), so they have
    reputation of their own that they try to FUNNEL/concentrate onto c0 (funnel laundering, not ring).

    `diversify`: if >0, each feeder also vouches for `diversify` random honest members, to LOWER the
    neighbour overlap (the other signature `independence()` looks at) and try to slip through. Measures
    the tension: dedicating to c0 -> high overlap (penalised); diversifying -> less pump per feeder.
    """
    rng = random.Random(seed)
    agents, graph, factions = _build_base(rng)
    honest = [aid for aid, f in factions.items() if f == "honest"]

    # pump target: no evidence (its power could only come from the funnel)
    agents.append(Agent(id="c0", kind=AgentKind.COLLUDER, unique_human=True,
                        evidence={}, cluster="funnel"))
    factions["c0"] = "colluder"

    feeders = [f"c{i}" for i in range(1, n_feeders + 1)]
    for fid in feeders:
        # feeders with some real work -> they have their own reputation to funnel onto c0
        agents.append(Agent(id=fid, kind=AgentKind.COLLUDER, unique_human=True,
                            evidence={"commerce": feeder_evidence}, cluster="funnel"))
        factions[fid] = "colluder"
        graph.attest(fid, "c0", "commerce", 1.0)          # the directed funnel onto c0
        if diversify > 0:
            for h in rng.sample(honest, min(diversify, len(honest))):
                graph.attest(fid, h, "commerce", 1.0)     # noise to lower the overlap

    # some fooled honest member vouches for c0 (injecting real trust onto the target)
    for hid in rng.sample(honest, fooled_honest):
        graph.attest(hid, "c0", "commerce", 1.0)

    return Scenario(
        name="asymmetric_collusion",
        description=f"PageRank funnel: {n_feeders} feeders (with real work) vouch for c0 without "
        f"reciprocity (diversify={diversify}). Asymmetric collusion (frontier §1.6).",
        agents=agents,
        graph=graph,
        factions=factions,
    )


def scenario_whitewashing(seed: int = 7) -> Scenario:
    """
    Whitewashing (§5, §1): a CONSOLIDATED pseudonym (ww_old, with evidence + vouches) vs a NEW one
    (ww_new, only base citizenship). Shows that abandoning the pseudonym to "start clean" costs ALL
    the earned reputation: reputation is bound to the mask, not transferred (Art. VII/VIII).
    """
    rng = random.Random(seed)
    agents, graph, factions = _build_base(rng)
    established = [a.id for a in agents]

    old = Agent(id="ww_old", kind=AgentKind.WHITEWASHER, unique_human=True,
                evidence={"commerce": 4.0})
    agents.append(old)
    factions["ww_old"] = "whitewasher"
    for v in rng.sample(established, 4):
        graph.attest(v, "ww_old", "commerce", 1.0)

    # same human, fresh pseudonym: only base citizenship, no inherited evidence nor vouches
    new = Agent(id="ww_new", kind=AgentKind.WHITEWASHER, unique_human=True, evidence={})
    agents.append(new)
    factions["ww_new"] = "whitewasher"

    return Scenario(
        name="whitewashing",
        description="Consolidated pseudonym vs a fresh pseudonym of the same human (whitewashing).",
        agents=agents,
        graph=graph,
        factions=factions,
    )


ALL = [scenario_honest, scenario_sybil, scenario_collusion, scenario_whitewashing]
