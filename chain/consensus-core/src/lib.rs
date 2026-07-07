//! Harlequin Woven-Trust Consensus — Rust port of the validated test-rig
//! (`consensus-simulator/testrig/`, 11/11). Reputation-weighted VRF sortition committees, rotating
//! each epoch, the foundation of the consensus the Substrate node will run. Dependency-free (the VRF's
//! SHA-256 is hand-rolled; OPSEC: nothing pulled onto the isolated station).
//!
//! This v0 ports the **sortition** (committee election by reputation, SPEC §2.2). The async voting
//! engine (sub-sampled Snowball over a lossy network, partition + quorum) lands in later increments;
//! the test-rig stays the behavioural reference and the analytical bounds live in
//! `consensus-simulator/ANALYSIS-safety-liveness.md` and `PARAMETERS.md`.

//! **`no_std`-ready.** Default build (`std` feature) keeps the f64 VRF oracle (`vrf.rs`) for the host
//! and cross-validation. With `default-features = false` the crate is `no_std` (alloc only) and exposes
//! the deterministic fixed-point sortition (`sortition_fp.rs`) plus the SHA-256 — what a Substrate
//! runtime links against (f64 `exp` is not reproducible across architectures).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod beacon;
pub mod finality;
pub mod sha256;
#[cfg(feature = "std")]
pub mod sim;
pub mod snowball;
pub mod sortition_fp;
#[cfg(feature = "std")]
pub mod vrf;

pub use finality::{FinalityRound, Hash as FinalityHash};
pub use snowball::{SnowballNode, SnowballParams};
pub use sortition_fp::{
    elect_committee_fp, elect_finality_committee, exp_neg_fp, poisson_cdf_fp, sortition_seats_fp,
    vrf_value_fp, FINALITY_COMMITTEE_SEED,
};
#[cfg(feature = "std")]
pub use vrf::{elect_committee, sortition_seats, vrf, vrf_verify};
