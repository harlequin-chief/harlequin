//! Harlequin node — the service that runs the chain.
#![warn(missing_docs)]

mod chain_spec;
mod cli;
mod command;
mod finality;
mod participation_inherent;
mod rpc;
mod service;
// #30: guarda que rechaza basura en la puerta antes de que el runtime la decodifique (ver txguard.rs).
// Conectada a las dos puertas (p2p vía build_network, RPC vía spawn_tasks).
mod txguard;

fn main() -> polkadot_sdk::sc_cli::Result<()> {
    command::run()
}
