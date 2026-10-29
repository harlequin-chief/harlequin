# Harlequin — the canonical parameters (mainnet build)

One table, read from the source, for the binary built with `--features mainnet` (the only one that runs a
real chain). Where the paper, the website or an older note says something else, **this table and the code
win**; every row names the constant so anyone can check it. Testnet values exist only to make validation
fast and are listed for contrast.

| What | Mainnet | Testnet | Where in the code |
|---|---|---|---|
| Slot (one block per slot, one leader per slot) | **12 s** | instant seal by default | `node/src/cli.rs` default `woven-trust:12000` |
| Blocks per day | 7 200 | — | 86 400 s / 12 s |
| Finality step (target = last finalised + step) | **15 blocks ≈ 3 min** | 8 | `node/src/finality.rs` `FINALITY_STEP` |
| Confirmations before a height is irreversible (β) | 12 | 4 | `finality.rs` `FINALITY_BETA` |
| Finality quorum (α) | ⌈0.70·k⌉, at least a strict majority | same | `finality.rs` `verify_proof`, `pallet-participation` |
| Finality committee: expected seats (τ) | 60 | 4 | `finality.rs` / `pallet-participation` `COMMITTEE_TAU` |
| Finality committee: cached seats at most | 64 | 64 | `runtime` `MaxCommittee` |
| Committee epoch (the jury is re-drawn) | **300 blocks ≈ 1 h** | 10 | `finality.rs` `EPOCH_LENGTH`, `pallet-participation` `COMMITTEE_EPOCH_LENGTH` |
| Slot-leader sortition size | 60 | 3 | `node/src/authorship.rs` `TAU` |
| Blocks a leader may claim ahead of a node's clock | 2 slots | 2 | `authorship.rs` `MAX_FUTURE_SLOTS` |
| Reputation epoch (recompute + decay) | **7 200 blocks = 1 day** | 10 | `runtime` `pallet_reputation::EpochLength` |
| Evidence retention per epoch (ρ) | 0.999052 → **half-life ≈ 731 days (24 months)** | 0.90 | `runtime` `EVIDENCE_RETENTION_PARTS` |
| Self-heal burial (skip a halted band) | 32 blocks | 15 | `finality.rs` `RECOVERY_BURIAL` |
| Emission era | 1 317 600 blocks = 183 days | 10 | `runtime` `EraLength` |
| HLQ hard cap | 540 000 000 HLQ | 1 000 000 raw | `runtime` `HlqCurve` |
| SOV hard cap | 54 000 000 SOV | 10 000 raw | `runtime` `SovCurve` |
| Block hashes kept (votes older than this are not credited) | 2 400 blocks ≈ 8 h | same | `runtime` `BlockHashCount` |
| Finality proof: records at most | 256 (one per member) | same | `finality.rs` `MAX_PROOF_VOTES` |

Derived figures, so nobody has to redo the arithmetic: a finalisation takes about β rounds, during which
about 12 blocks are produced at 12 s, so the step of 15 keeps finality ahead of production and the gap
settles near one step behind the tip; 24 committees are drawn per reputation epoch, all over the same
reputation and a fresh beacon.
