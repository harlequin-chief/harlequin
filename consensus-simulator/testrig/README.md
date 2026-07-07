# Woven-Trust Consensus — faithful test-rig

Step 1 of the chain path (the chain roadmap, decided: Substrate). Before writing
any Rust/Substrate runtime, validate the consensus under conditions the synchronous simulator
(`../wtc_sim/`) abstracts away.

## What it adds over `wtc_sim/`
1. **VRF sortition committees (SPEC §2.2).** Each epoch, a committee is *elected* by a verifiable
   random function weighted by **reputation** (Algorand-style, Poisson form). Only the committee votes.
   A Sybil with reputation ≈0 wins ≈0 seats. `vrf.py`.
2. **Committee rotation (Art. VI).** Each epoch chains a new randomness beacon → a different committee.
   No node retains the validating role. `engine.run_epochs`.
3. **Asynchronous, lossy network.** Messages carry latency and may be dropped on either leg (query and
   response), driven by a discrete-event queue — not synchronous lockstep rounds. `network.py`.
4. **Partition + quorum mitigation.** A node samples only its group while partitioned; finality is
   gated on reaching a committee-reputation quorum (tied to the sample's provenance), so an isolated
   group halts instead of forking. `engine.run_epoch(group=..., partition_until=..., network_quorum=...)`.

## Layout
- `vrf.py` — simulated verifiable VRF + reputation-weighted Poisson sortition (`elect_committee`).
- `network.py` — discrete-event message bus (latency + loss).
- `engine.py` — `run_epoch` (elect committee, run async sub-sampled voting) and `run_epochs` (rotation).
- `scenarios.py` — populations with VRF keys (reputation fraction, Sybil crowd).
- `run_testrig.py` — writes `../RESULTS-testrig.md`.
- `tests/test_testrig.py` — 9 self-audit tests.

## Run
```
python3 testrig/run_testrig.py        # from consensus-simulator/ ; writes RESULTS-testrig.md
python3 testrig/tests/test_testrig.py # 9/9
```

## Findings (see `../RESULTS-testrig.md`)
- Safety threshold stays at **~0.4 reputation** even with a reputation-elected, rotating committee over
  an async network — matching the simulator and `../ANALYSIS-safety-liveness.md` (`f<α/k`).
- Sybils are **excluded from the committee** by sortition (0 seats at reputation ≈0).
- Committees **rotate** each epoch (low overlap).
- Round-trip loss costs **liveness, not safety**; because both legs can drop, the liveness bound
  tightens to **`α≤k(1−p)²`**.
- A long **partition** forks the network (100%); the **reputation-quorum** mitigation eliminates it
  (0% fork at quorum 0.6) by halting the isolated group — safety over liveness.

## Honesty
The VRF is **simulated by a hash** (the interface matches a real ECVRF so the engine ports unchanged);
the decision is binary; parameters (k, α, β, τ, window, epoch length) are stand-ins to be fixed with
this rig before production. A machine-checked proof remains open (`../ANALYSIS-safety-liveness.md §8`).
