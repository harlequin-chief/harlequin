# chain/ — the Harlequin chain (Substrate)

The real implementation of the Harlequin chain. Stack **decided 2026-06-15: Substrate** (Rust,
sovereign solochain, custom Woven-Trust Consensus) — see `../DECISION-STACK-CADENA.md` and `SPEC §2.0`.

**Method (quality over speed):** the model was first validated as executable prototypes
(`../prototipos/reputacion/` 17/17, `../prototipos/consenso/` 13/13 + `testrig/` 11/11). Those remain
the **guardrail**: the Rust implementation must reproduce their numbers. Nothing is written here on
faith.

## Layout
- **`reputation-core/`** — Rust port of the validated reputation engine (the heart, SPEC §1/§1.6).
  A plain library crate (compiles on stable Rust), so the logic is validated **before** it is wrapped
  as a Substrate pallet — the lean path that avoids the heavy node toolchain up front.
  - It cross-validates against the Python prototype: `matches_python_prototype` asserts identical
    reputation values (g0=297.03, h0=702.97, sybil=0) for a shared scenario. Fidelity is a test.
  - Run: `cd reputation-core && cargo test`.

- **`consensus-core/`** — Rust port of the consensus sortition (SPEC §2.2): reputation-weighted VRF
  committee election, rotating each epoch. Dependency-free — includes a hand-rolled **SHA-256**
  (matches the standard vectors / Python `hashlib`, so the simulated VRF is bit-identical; a real
  ECVRF replaces it in production). 4/4 tests (Sybils excluded, committee ~τ, rotation). The async
  voting engine (Snowball + partition/quorum) is a later increment; the test-rig stays the reference.
  - Run: `cd consensus-core && cargo test`.

- **`composition/`** — the end-to-end claim, in Rust: `reputation-core` derives each pseudonym's
  vectorial reputation, it is aggregated conservatively (min over the four suits), and that scalar
  weights the `consensus-core` VRF sortition. Test: a collusion ring and 50 Sybils earn ~0 reputation
  → **0 committee seats**; honest members hold the committee. *Power is earned reputation — the two
  prototypes compose.* (Cargo workspace ties the three crates: `cargo test` from `chain/`.)
  - Runnable demo: `cargo run -p composition` — prints the committee by class (55 of 80 agents are
    colluders/Sybils and hold 0 seats; honest + genesis hold 100%).

- **`protocol-core/`** — the **epoch state machine**: the living chain that ties the two engines
  together. `genesis(cohort)` seeds the founding members (§1.4); `advance_epoch(beacon)` recomputes
  every member's reputation (deterministic fixed-point), elects the committee by reputation-weighted
  sortition, and returns the **telemetry the nodes publish** (`EpochReport`) — node count, committee,
  reputation per suit, and a **Gini coefficient** of reputation: the network's own *"is an elite
  forming?"* alarm (SPEC §5c, served by the nodes, no central dashboard to switch off). 6/6 tests.
  - Runnable demo: `cargo run -p protocol-core --example epochs` — 50 founders + 1000 Sybils over 3
    epochs; the Sybils never enter the committee and the telemetry JSON is printed each epoch.

## Design
- **`PALLET-DESIGN.md`** — how `reputation-core` becomes an on-chain reputation pallet: inputs on-chain
  (evidence + vouch graph), reputation **derived** and recomputed each epoch by an offchain worker
  (verify-by-recompute, no trusted oracle); committee/jury read the previous epoch's snapshot. The two
  prerequisites it flagged are **done**: **deterministic fixed-point arithmetic** (the whole
  factor math — independence/community/in-concentration — and the EigenTrust iteration run in i128,
  bit-identical across architectures, cross-validated against f64) and the **`no_std` feature-gate**.
  `reputation-core` now builds for `wasm32-unknown-unknown` with `default-features = false`: the f64
  oracle/prototype path sits behind the `std` feature; the runtime links only the fixed-point path plus
  the vouch registry. Remaining for the pallet: port the sortition (consensus-core) and vouch scoring
  (`log2`) to fixed-point so the whole epoch machine is `no_std`.

## Path ahead (`DECISION-STACK-CADENA.md §5`)
1. ~~Faithful consensus test-rig~~ — done (`../prototipos/consenso/testrig/`).
2. **Done — full parity:** the engine is ported to Rust (`reputation-core/`), every piece
   cross-validated against the Python prototype: trust graph + independence damping, EigenTrust
   anchored in evidence, community suspicion, per-suit in-concentration, inactivity decay, the
   sponsorship economy (quota, mentor dividend, graduation) and cascade slashing. **10/10 tests.**
3. **Done:** `reputation-core` is `no_std` / `wasm32`-ready (fixed-point path + vouch registry).
4. **Done:** the **epoch state machine** (`protocol-core/`) composes reputation + sortition into a
   living chain (genesis → epochs → committee → telemetry).
5. Wrap it as a **reputation pallet** + a minimal Substrate solochain with the genesis cohort (§1.4).
6. Port sortition + vouch scoring to fixed-point so the whole epoch machine is `no_std`.
7. Sub-sampled finality + light clients.

## Toolchain
`rustup` (stable + nightly + `wasm32-unknown-unknown`) is installed on the station — enough to build the
library crates **and** the runtime wasm. No external/third-party application installers beyond the Rust
toolchain (OPSEC: this is the isolated admin station). License: AGPL-3.0-or-later, like the rest of the
project.
