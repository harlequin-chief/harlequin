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

## Path ahead (`DECISION-STACK-CADENA.md §5`)
1. ~~Faithful consensus test-rig~~ — done (`../prototipos/consenso/testrig/`).
2. **Done — full parity:** the engine is ported to Rust (`reputation-core/`), every piece
   cross-validated against the Python prototype: trust graph + independence damping, EigenTrust
   anchored in evidence, community suspicion, per-suit in-concentration, inactivity decay, the
   sponsorship economy (quota, mentor dividend, graduation) and cascade slashing. **10/10 tests.**
3. Wrap it as a **reputation pallet**; minimal Substrate solochain + genesis seed cohort (§1.4).
4. Implement the reputation-weighted VRF sortition consensus (the test-rig is the reference).
5. Sub-sampled finality + light clients.

The full Substrate node toolchain (rustup nightly + wasm, multi-GB) is **not** installed yet; only
Debian's stable `rustc`/`cargo`, enough for the library crate. Installing the full node environment
(here vs a separate dev machine) is an open infra decision.

## Toolchain
Debian `rustc`/`cargo` (`apt`, stable). No external/third-party installers (OPSEC: this is the isolated
admin station). License: AGPL-3.0-or-later, like the rest of the project.
