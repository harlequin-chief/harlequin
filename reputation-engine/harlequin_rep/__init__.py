"""
harlequin_rep — Prototype of Harlequin's reputation engine.

Implements, in executable code, the core described in SPEC.md §1 (reputation) and §2.2 (consensus by
reputation-weighted sortition). Python standard library only: no external dependencies (the project's
OPSEC criterion).

Goal: stop having the design only on paper and be able to MEASURE whether the anti-Sybil and
anti-collusion defences (the SPEC's risk frontier #1, §1.6) do what they promise.

SPEC -> code map:
- §1.2b vectorial (contextual) reputation     -> model.DIMENSIONS, reputation.reputation_vector
- §1.3  how it is earned (evidence + vouch)    -> graph.TrustGraph, reputation (pretrust anchoring)
- §1.4  two gates (base vs earned citizenship) -> model.Agent.base / .unique_human
- §1.6  anti-collusion (per-cluster damping)   -> graph.independence + reputation (damped EigenTrust)
- §1.7  decay and slashing                     -> vouch.cascade_slashing, reputation.decay
- §2.2  consensus: reputation-weighted draw    -> consensus.weighted_sortition
"""

from .model import Agent, AgentKind, DIMENSIONS
from .graph import TrustGraph
from .reputation import reputation_vector, conservative_aggregate
from .consensus import weighted_sortition

__all__ = [
    "Agent",
    "AgentKind",
    "DIMENSIONS",
    "TrustGraph",
    "reputation_vector",
    "conservative_aggregate",
    "weighted_sortition",
]
