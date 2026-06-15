"""
Verifiable sortition by reputation (SPEC §2.2: committee election by VRF weighted by REPUTATION, not
stake). Algorand-style cryptographic sortition (Gilad et al. 2017), here with the VRF SIMULATED by a
hash for the prototype — the shape is what we validate, not the cryptography:

- Each node has a secret key `sk` (a string seed). `vrf(sk, seed)` returns a value in [0,1) and a
  proof; anyone holding the public key (here = `sk`, since it is a simulation) can VERIFY the value
  was produced from (sk, seed) and could not be ground for. In production this is a real VRF
  (e.g. ECVRF); the simulator keeps the same interface so the engine code ports unchanged.
- `seed` per epoch is the public, unpredictable randomness beacon (e.g. the previous block's VRF
  output). The same seed for everyone makes the draw common-coin yet per-node secret until revealed.

SORTITION (Poisson form): a node with reputation `r` out of total `R`, for a target committee size
`tau`, expects `lam = tau * r/R` committee seats. The VRF value selects the actual integer number of
seats `j` via the inverse Poisson CDF. So:
  - total expected committee size = sum_i lam_i = tau  (independent of how reputation is split);
  - seats are proportional to REPUTATION → a Sybil with r≈0 gets lam≈0 → j=0 almost surely (Art. VI);
  - the result is verifiable: recompute lam and the quantile from the public (seed, r, R, value).
The Poisson form is the large-population limit of Algorand's binomial sortition and handles a
continuous reputation weight cleanly (binomial needs an integer stake).
"""

from __future__ import annotations

import hashlib
import math

_HASH_BITS = 256
_HASH_MAX = float(1 << _HASH_BITS)


def vrf(sk: str, seed: str) -> tuple[float, str]:
    """
    Simulated VRF. Returns (value in [0,1), proof). Deterministic in (sk, seed): a node cannot grind
    for a better value without changing its key. `proof` lets a verifier recompute the value.
    """
    proof = hashlib.sha256(f"{sk}|{seed}".encode()).hexdigest()
    value = int(proof, 16) / _HASH_MAX
    return value, proof


def vrf_verify(sk: str, seed: str, value: float, proof: str) -> bool:
    """Recompute the VRF from (sk, seed) and check it matches the claimed value+proof (no grinding)."""
    exp_proof = hashlib.sha256(f"{sk}|{seed}".encode()).hexdigest()
    if proof != exp_proof:
        return False
    return abs(int(exp_proof, 16) / _HASH_MAX - value) < 1e-12


def _poisson_cdf(j: int, lam: float) -> float:
    """P(X <= j) for X ~ Poisson(lam). Stable for the small lam used here."""
    if lam <= 0.0:
        return 1.0
    term = math.exp(-lam)  # P(X=0)
    cdf = term
    for i in range(1, j + 1):
        term *= lam / i
        cdf += term
    return min(cdf, 1.0)


def sortition_seats(value: float, lam: float, max_seats: int = 64) -> int:
    """
    Inverse Poisson CDF: smallest j such that value < CDF(j; lam). Number of committee seats this
    node wins. lam=0 -> 0 seats. Capped at max_seats (anti-pathology for huge reputation).
    """
    if lam <= 0.0:
        return 0
    j = 0
    while j < max_seats:
        if value < _poisson_cdf(j, lam):
            return j
        j += 1
    return max_seats


def elect_committee(
    reputation: dict[str, float],
    secret_keys: dict[str, str],
    seed: str,
    tau: float,
) -> dict[str, int]:
    """
    Run sortition for an epoch. Returns {node: seats} for nodes that won >=1 seat. The committee is
    reputation-weighted (expected total seats ~ tau) and verifiable.

    `seed` is the epoch's public randomness; changing it (each epoch) ROTATES the committee
    (SPEC §2.2, Art. VI anti-entrenchment): the same node does not stay in power across epochs.
    """
    R = sum(max(r, 0.0) for r in reputation.values())
    if R <= 0.0:
        return {}
    committee: dict[str, int] = {}
    for node, r in reputation.items():
        r = max(r, 0.0)
        if r <= 0.0:
            continue
        lam = tau * r / R
        value, _ = vrf(secret_keys[node], seed)
        seats = sortition_seats(value, lam)
        if seats > 0:
            committee[node] = seats
    return committee
