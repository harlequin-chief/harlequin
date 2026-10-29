//! Harlequin node — the service that runs the chain.
#![warn(missing_docs)]

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
    command::run()
}
