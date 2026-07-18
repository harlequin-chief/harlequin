# Validation evidence — reproducible scripts

Standalone scripts behind entries in the public [validation ledger](../../VALIDATION-LEDGER.md)
and the whitepaper's reproducibility appendix. Each one drives a local devnet node over WebSocket
(`ws://127.0.0.1:9944`, `substrate-interface`) and exits 0 only if every check passes.

| Script | Validates | Ledger context |
|---|---|---|
| `validate_decay.py` | Evidence decay ≈ ρ per epoch (leaky integrator): a non-injecting account evaporates, a re-injecting control holds. ρ=0.90 is the devnet placeholder, not the mainnet parameter. | #33 |
| `validate_epoch19.py` | Epoch recompute/decay fires automatically at every epoch boundary — the `advance_epoch` extrinsic no longer exists; no privileged button drives the clock. | #19 |
| `validate_justice.py` | Jury selection excludes parties and depth-1 vouch-interested accounts; guilty verdict slashes reputation only; the deterministic depth-1/depth-2 interest boundary. | #35 |

These run against dev binaries with well-known dev keys (`//Alice`…) on a throwaway local chain;
nothing here touches a live network.
