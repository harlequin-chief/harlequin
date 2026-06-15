"""
Woven-Trust Consensus test-rig: a more faithful consensus model than `wtc_sim/` — epoch committees
elected by VRF sortition weighted by reputation (SPEC §2.2), rotating each epoch (Art. VI), running
sub-sampled Snowball voting over an asynchronous, lossy network.

Step 1 of the chain path (DECISION-STACK-CADENA §5): validate the design under realistic conditions
and fix parameters BEFORE writing any Substrate/Rust runtime.
"""

from .vrf import vrf, vrf_verify, sortition_seats, elect_committee
from .network import Network
from .engine import RigParams, run_epoch, run_epochs

__all__ = [
    "vrf",
    "vrf_verify",
    "sortition_seats",
    "elect_committee",
    "Network",
    "RigParams",
    "run_epoch",
    "run_epochs",
]
