"""
Data model: reputation dimensions and agents (pseudonyms).

Anchored in SPEC.md:
- §1.2b: VECTORIAL reputation. Initial dimensions (extensible): commerce, technical contribution,
  judicial function, governance. Earning in one does NOT contaminate the others.
- §1.4: TWO GATES. Gate 1 = personhood (unique human) -> base citizenship = 1, no sponsor.
  Gate 2 = reputation earned above the base.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum


# §1.2b — initial dimensions of the vectorial reputation (extensible set).
DIMENSIONS: tuple[str, ...] = (
    "commerce",
    "technical_contribution",
    "judicial_function",
    "governance",
)


class AgentKind(str, Enum):
    """Label used ONLY for analysing the simulation (the protocol does not know it)."""

    GENESIS = "genesis"          # founding cohort (seed, §1.4); designed to dilute away
    HONEST = "honest"            # real member with a verifiable history
    SYBIL = "sybil"              # fake identity with no real work (§1.5)
    COLLUDER = "colluder"        # part of a mutual-vouching ring (§1.6)
    WHITEWASHER = "whitewasher"  # abandons a "burned" pseudonym and restarts (whitewashing)


@dataclass
class Agent:
    """
    A pseudonym of the network. The physical identity is NEVER modelled: only the pseudonym (Art. VII).

    - unique_human: passed the personhood test (§1.5). Grants base citizenship = 1.
    - evidence: work/deals VALIDATED per dimension (§1.3a). The "real" anchor of reputation: without
      objective evidence, vouches alone cannot manufacture power.
    - the earned reputation vector is NOT stored here: it is DERIVED from the graph + evidence
      (see reputation.reputation_vector). Only the model inputs live here.
    """

    id: str
    kind: AgentKind
    unique_human: bool = True
    # objective verifiable evidence per dimension (settled deals, proven work).
    evidence: dict[str, float] = field(default_factory=dict)
    cluster: str | None = None  # ring/cluster label, only to build/analyse scenarios

    @property
    def base(self) -> float:
        """Base citizenship (§1.4): 1 if a verified unique person, 0 otherwise."""
        return 1.0 if self.unique_human else 0.0

    def evidence_in(self, dim: str) -> float:
        return self.evidence.get(dim, 0.0)
