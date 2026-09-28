//! CLI command dispatch. Adapted from the polkadot-sdk minimal template.

use crate::{
    chain_spec,
    cli::{Cli, Subcommand},
    service,
};
use polkadot_sdk::{
    sc_cli::SubstrateCli,
    sc_service::PartialComponents,
    *,
};

impl SubstrateCli for Cli {
    fn impl_name() -> String {
        "Harlequin Node".into()
    }

    fn impl_version() -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn description() -> String {
        env!("CARGO_PKG_DESCRIPTION").into()
    }

    fn author() -> String {
        env!("CARGO_PKG_AUTHORS").into()
    }

    fn support_url() -> String {
        "https://harlequinproject.org".into()
    }

    fn copyright_start_year() -> i32 {
        2026
    }

    fn load_spec(&self, id: &str) -> Result<Box<dyn sc_service::ChainSpec>, String> {
        Ok(match id {
            "dev" | "" => Box::new(chain_spec::development_chain_spec()?),
            "launch" => Box::new(chain_spec::launch_chain_spec()?),
            path => Box::new(chain_spec::ChainSpec::from_json_file(
                std::path::PathBuf::from(path),
            )?),
        })
    }
}

pub fn run() -> sc_cli::Result<()> {
    let mut cli = Cli::from_args();
    // Upstream defaults `--pool-type` to fork-aware, and that pool's background task takes the whole node down
    // ("Essential task `txpool-background` failed. Shutting down service.") — reproduced 27-sep-2026 with a
    // hand-started node receiving a newcomer's first transactions. The installer and the Dockerfile pass
    // `--pool-type single-state`; a node started by hand must not depend on remembering it. An explicit
    // `--pool-type` still wins.
    if !std::env::args().any(|a| a == "--pool-type" || a.starts_with("--pool-type=")) {
        cli.run.pool_config.pool_type = sc_cli::TransactionPoolType::SingleState;
    }
    // Harlequin's registered SS58 prefix. Without this the node's RPC only understands the generic 42 and
    // REJECTS every Harlequin address ("Unknown SS58 address format `1728`", measured on the live node
    //): `system_accountNextIndex` had never answered for a real mask, and the mediator had to
    // translate addresses to prefix 42 to ask for a nonce. Set once, before anything parses an address.
    sp_core::crypto::set_default_ss58_version(sp_core::crypto::Ss58AddressFormat::custom(1728));

    match &cli.subcommand {
        Some(Subcommand::Key(cmd)) => cmd.run(&cli),
        Some(Subcommand::BuildSpec(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| cmd.run(config.chain_spec, config.network))
        },
        Some(Subcommand::CheckBlock(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents { client, task_manager, import_queue, .. } =
                    service::new_partial(&config)?;
                Ok((cmd.run(client, import_queue), task_manager))
            })
        },
        Some(Subcommand::ExportBlocks(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents { client, task_manager, .. } =
                    service::new_partial(&config)?;
                Ok((cmd.run(client, config.database), task_manager))
            })
        },
        Some(Subcommand::ExportState(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents { client, task_manager, .. } =
                    service::new_partial(&config)?;
                Ok((cmd.run(client, config.chain_spec), task_manager))
            })
        },
        Some(Subcommand::ImportBlocks(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents { client, task_manager, import_queue, .. } =
                    service::new_partial(&config)?;
                Ok((cmd.run(client, import_queue), task_manager))
            })
        },
        Some(Subcommand::PurgeChain(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| cmd.run(config.database))
        },
        Some(Subcommand::Revert(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.async_run(|config| {
                let PartialComponents { client, task_manager, backend, .. } =
                    service::new_partial(&config)?;
                Ok((cmd.run(client, backend, None), task_manager))
            })
        },
        Some(Subcommand::ChainInfo(cmd)) => {
            let runner = cli.create_runner(cmd)?;
            runner.sync_run(|config| cmd.run::<harlequin_runtime::interface::OpaqueBlock>(&config))
        },
        None => {
            let runner = cli.create_runner(&cli.run)?;
            // Resolve the finality-signing secret: prefer `--vote-as-file` (read 0600 file, keeps the
            // hot session secret out of argv); fall back to `--vote-as`. clap enforces they aren't both set.
            let vote_as: Option<String> = match (cli.vote_as.clone(), cli.vote_as_file.clone()) {
                (Some(s), _) => Some(s),
                (None, Some(path)) => Some(
                    std::fs::read_to_string(&path)
                        .map_err(|e| {
                            sc_cli::Error::Input(format!("cannot read --vote-as-file `{path}`: {e}"))
                        })?
                        .trim()
                        .to_string(),
                ),
                (None, None) => None,
            };
            runner.run_node_until_exit(|config| async move {
                match config.network.network_backend {
                    sc_network::config::NetworkBackendType::Libp2p => service::new_full::<
                        sc_network::NetworkWorker<
                            harlequin_runtime::interface::OpaqueBlock,
                            <harlequin_runtime::interface::OpaqueBlock as sp_runtime::traits::Block>::Hash,
                        >,
                    >(config, cli.consensus, vote_as.clone(), cli.seal_without_peers)
                    .map_err(sc_cli::Error::Service),
                    sc_network::config::NetworkBackendType::Litep2p =>
                        service::new_full::<sc_network::Litep2pNetworkBackend>(
                            config,
                            cli.consensus,
                            vote_as.clone(),
                            cli.seal_without_peers,
                        )
                        .map_err(sc_cli::Error::Service),
                }
            })
        },
    }
}
