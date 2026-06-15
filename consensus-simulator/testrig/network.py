"""
Discrete-event network with latency and loss — the faithfulness the round-synchronous simulator
(`wtc_sim/`) abstracts away. Messages do NOT arrive instantly in lockstep: each carries a random
delay, some are dropped, and a node acts on what it has RECEIVED by a deadline (timeout), not on a
global round. This lets us see whether the consensus still converges when the network is asynchronous
and lossy, instead of assuming a perfect synchronous round.

Model: a single event queue ordered by delivery time. `send` schedules a delivery (or a drop);
`run_until` drains events in time order, invoking each node's handler. Time is abstract units.
"""

from __future__ import annotations

import heapq
import itertools
import random
from dataclasses import dataclass, field
from typing import Callable


@dataclass(order=True)
class _Event:
    time: float
    seq: int
    cb: Callable[[], None] = field(compare=False)


class Network:
    """Async message bus: latency-delayed, lossy delivery over a single time-ordered event queue."""

    def __init__(
        self,
        rng: random.Random,
        latency: Callable[[], float] | None = None,
        loss: float = 0.0,
    ) -> None:
        self.rng = rng
        # default latency: ~lognormal-ish via uniform(1,3); callers can inject a distribution
        self.latency = latency or (lambda: rng.uniform(1.0, 3.0))
        self.loss = loss
        self._q: list[_Event] = []
        self._seq = itertools.count()
        self.now = 0.0
        self.sent = 0
        self.dropped = 0

    def schedule(self, delay: float, cb: Callable[[], None]) -> None:
        """Schedule a callback `delay` time units from now (used for timeouts/timers)."""
        heapq.heappush(self._q, _Event(self.now + max(0.0, delay), next(self._seq), cb))

    def send(self, deliver: Callable[[], None]) -> None:
        """Send a message: delivered after a random latency, or dropped with probability `loss`."""
        self.sent += 1
        if self.loss > 0.0 and self.rng.random() < self.loss:
            self.dropped += 1
            return
        self.schedule(self.latency(), deliver)

    def run_until(self, t_end: float, stop: Callable[[], bool] | None = None) -> None:
        """Drain events in time order up to t_end, or until `stop()` is true."""
        while self._q:
            if self._q[0].time > t_end:
                self.now = t_end
                break
            ev = heapq.heappop(self._q)
            self.now = ev.time
            ev.cb()
            if stop is not None and stop():
                break
