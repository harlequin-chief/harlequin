//! Harlequin Woven-Trust Consensus — Rust port of the validated test-rig
//! (`prototipos/consenso/testrig/`, 11/11). Reputation-weighted VRF sortition committees, rotating
//! each epoch, the foundation of the consensus the Substrate node will run. Dependency-free (the VRF's
//! SHA-256 is hand-rolled; OPSEC: nothing pulled onto the isolated station).
//!
//! This v0 ports the **sortition** (committee election by reputation, SPEC §2.2). The async voting
//! engine (sub-sampled Snowball over a lossy network, partition + quorum) lands in later increments;
//! the test-rig stays the behavioural reference and the analytical bounds live in
//! `prototipos/consenso/ANALYSIS-safety-liveness.md` and `PARAMETERS.md`.

pub mod sha256;
pub mod vrf;

pub use vrf::{elect_committee, sortition_seats, vrf, vrf_verify};
