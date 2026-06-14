"""
wtc_sim — Woven Trust Consensus simulator.

Measures, in figures, the paper's thesis (`woven-trust-consensus.md`): power in consensus depends on
REPUTATION, not on the number of nodes. An attacker with many identities but little reputation (Sybil)
does not break the network; the security threshold is expressed as a FRACTION OF THE ADVERSARY'S
REPUTATION, not a fraction of nodes.

Implements sub-sampled Snowball/Avalanche voting (SPEC §2.2), with WTC's own contribution: sampling is
weighted by REPUTATION (instead of uniform). Standard library only.
"""

from .consensus import run_once, ConsensusParams
from .population import population_sybil, population_reputation_fraction

__all__ = ["run_once", "ConsensusParams", "population_sybil", "population_reputation_fraction"]
