"""
Consensus: committee sortition weighted by REPUTATION, not by wealth (SPEC §2.2, Art. VI).

Models Algorand-style validator selection (VRF sortition): each epoch a rotating committee is chosen
with probability proportional to reputation (conservative aggregate of the vector, §1.2b). Here the
VRF is simulated with a seeded PRNG (deterministic and reproducible); in production it would be a
real verifiable random function.

Thesis to verify (§2.4): creating fake nodes does NOT grant weight (they are born with reputation 0).
The weight in consensus inherits the robustness of the reputation system.
"""

from __future__ import annotations

import random


def _selection_weight(aggregate_rep: float, base: float, base_weight: float = 0.0) -> float:
    """
    Weight of an agent in the sortition. Dominated by EARNED reputation.

    `base_weight` allows, if desired, base citizenship to contribute a minimum (>0). Default 0:
    consensus is governed by earned reputation (gate 2), not mere citizenship (§1.4, Art. VI:
    "without reputation no voice is raised").
    """
    return max(0.0, aggregate_rep) + base_weight * base


def weighted_sortition(
    weights: dict[str, float],
    committee_size: int,
    epochs: int = 1000,
    seed: int = 1984,
) -> dict[str, int]:
    """
    Simulates `epochs` draws of a committee of `committee_size` members, without replacement within
    each committee, with probability proportional to `weights`. Returns id -> number of times picked.

    Used to measure the committee SHARE each faction captures (honest vs attackers).
    """
    rng = random.Random(seed)
    eligible = [k for k, w in weights.items() if w > 0]
    count: dict[str, int] = {k: 0 for k in weights}

    size = min(committee_size, len(eligible))
    if size == 0:
        return count

    for _ in range(epochs):
        available = eligible.copy()
        w = {k: weights[k] for k in available}
        for _ in range(size):
            total = sum(w[k] for k in available)
            if total <= 0:
                break
            r = rng.uniform(0.0, total)
            acc = 0.0
            picked = available[-1]
            for k in available:
                acc += w[k]
                if r <= acc:
                    picked = k
                    break
            count[picked] += 1
            available.remove(picked)

    return count
