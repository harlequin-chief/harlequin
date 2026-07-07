//! Harlequin node — the service that runs the chain.
#![warn(missing_docs)]

mod chain_spec;
mod cli;
mod command;
mod finality;
mod participation_inherent;
mod rpc;
mod service;

fn main() -> polkadot_sdk::sc_cli::Result<()> {
    command::run()
}
