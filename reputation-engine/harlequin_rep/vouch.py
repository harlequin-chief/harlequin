"""
Vouching mechanics: quota, persistent liability, mentor dividend and cascade slashing.

Anchored in SPEC.md:
- §1.5c: incentive to sponsor is ONLY reputational; sponsor->protege link is PERSISTENT.
  - negative side: if the protege defrauds, the sponsor LOSES reputation (even later in time).
  - positive side: mentor dividend = a small echo of the protege's INDEPENDENT reputation;
    sponsoring puppets (reputation dependent on one's own cluster) does NOT pay off.
  - live-vouch quota = SUBLINEAR function of reputation (decreasing returns).
- §1.7: slashing for proven fraud.

This does NOT recompute EigenTrust; it operates on an already-computed reputation vector to show the
incentive DYNAMICS (what happens to the sponsor when the protege prospers or defrauds).
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field


@dataclass
class Sponsorship:
    sponsor: str
    protege: str
    live: bool = True  # released when the protege "graduates", but the liability does not expire


@dataclass
class VouchRegistry:
    """Keeps track of who vouched for whom (persistent liability, §1.5c)."""

    links: list[Sponsorship] = field(default_factory=list)

    def sponsor_link(self, sponsor: str, protege: str) -> None:
        self.links.append(Sponsorship(sponsor, protege))

    def sponsors_of(self, protege: str) -> list[str]:
        return [v.sponsor for v in self.links if v.protege == protege]

    def live_vouches(self, sponsor: str) -> int:
        return sum(1 for v in self.links if v.sponsor == sponsor and v.live)

    def graduate(self, sponsor: str, protege: str) -> bool:
        """
        Graduate the protege (§1.5c): release the LIVE link (it stops taking up the sponsor's quota)
        once the protege stands on its own independent reputation. The LIABILITY does not expire: the
        link stays in the registry (cascade slashing still reaches it), it just stops being `live`.
        Returns True if any were graduated.
        """
        graduated = False
        for v in self.links:
            if v.sponsor == sponsor and v.protege == protege and v.live:
                v.live = False
                graduated = True
        return graduated


def vouch_quota(aggregate_reputation: float, k: float = 3.0) -> int:
    """
    Live-vouch quota = sublinear function of reputation (§1.5c): more reputation -> more quota, with
    decreasing returns so that nobody monopolises sponsorship.

    quota = floor(k * log2(1 + rep))   (sublinear, grows slowly).
    """
    return int(k * math.log2(1.0 + max(0.0, aggregate_reputation)))


def mentor_dividend(
    protege_independent_rep: float,
    echo: float = 0.05,
) -> float:
    """
    Mentor dividend (§1.5c, positive side): the sponsor earns a SMALL echo of the protege's
    INDEPENDENT reputation (already discounted by graph closeness, §1.6). Sponsoring your own
    puppets -> their independent reputation ~ 0 -> dividend ~ 0 -> does not pay off.
    """
    return echo * max(0.0, protege_independent_rep)


def cascade_slashing(
    reputation: dict[str, float],
    registry: VouchRegistry,
    culprit: str,
    loss: float,
    sponsor_fraction: float = 0.5,
    depth: int = 3,
) -> dict[str, float]:
    """
    Slashing for proven fraud (§1.7) with PERSISTENT LIABILITY in cascade (§1.5c).

    The culprit loses `loss`. Each sponsor loses a FRACTION of what their protege lost (liability for
    whom they let in), and so on up the vouch chain up to `depth`. Returns a NEW reputation dict
    (does not mutate the input).

    This is what makes collusion expensive: building a ring and having one defraud drags down its
    vouchers. Sponsoring carelessly is costly.
    """
    updated = dict(reputation)

    def apply(agent: str, amount: float, level: int) -> None:
        if amount <= 0 or level < 0 or agent not in updated:
            return
        updated[agent] = max(0.0, updated[agent] - amount)
        passes_on = amount * sponsor_fraction
        for sponsor in registry.sponsors_of(agent):
            apply(sponsor, passes_on, level - 1)

    apply(culprit, loss, depth)
    return updated
