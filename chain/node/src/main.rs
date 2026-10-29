//! Harlequin node — the service that runs the chain.
#![warn(missing_docs)]

// L6-01 (candidate 9): the slot-leader rule shared by author and import verifier, and the authorship seal.
mod authorship;
mod chain_spec;
mod cli;
mod command;
mod finality;
mod participation_inherent;
mod rpc;
mod service;
mod sphinx_relay;
// #30: guard that rejects garbage at the door before the runtime decodes it (see txguard.rs).
// Wired into both doors (p2p via build_network, RPC via spawn_tasks).
mod txguard;

fn main() -> polkadot_sdk::sc_cli::Result<()> {
    #[cfg(all(feature = "rogue-author", feature = "mainnet"))]
    eprintln!("🃏 THIS IS A LAB BUILD (rogue-author): it seals out of turn on purpose. Never run it on a real chain.");
    command::run()
}
