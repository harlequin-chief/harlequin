//! CLI definition. Adapted from the polkadot-sdk minimal template.

use polkadot_sdk::{sc_cli::RunCmd, *};

/// Block authoring engine. `instant-seal` / `manual-seal-<ms>` are the bootstrap engines; `woven-trust-
/// <ms>` is the real one: each slot the node runs the reputation-weighted sortition and only authors
/// when it is elected to the committee (SPEC §2). Step 1 gates authorship by sortition with trivial
/// finality; the finality gadget (Snowball voting) lands in a later step.
#[derive(Debug, Clone)]
pub enum Consensus {
    ManualSeal(u64),
    InstantSeal,
    WovenTrust(u64),
}

impl std::str::FromStr for Consensus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if s == "instant-seal" {
            Consensus::InstantSeal
        } else if let Some(block_time) = s.strip_prefix("manual-seal-") {
            Consensus::ManualSeal(block_time.parse().map_err(|_| "invalid block time")?)
        } else if let Some(block_time) = s.strip_prefix("woven-trust-") {
            Consensus::WovenTrust(block_time.parse().map_err(|_| "invalid block time")?)
        } else {
            return Err("incorrect consensus identifier".into());
        })
    }
}

#[derive(Debug, clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: Option<Subcommand>,

    /// The consensus (block authoring) to use: `instant-seal` (default), `manual-seal-<ms>`, or
    /// `woven-trust-<ms>` (reputation-weighted sortition gates authoring — the real engine).
    #[clap(long, default_value = "instant-seal")]
    pub consensus: Consensus,

    #[clap(flatten)]
    pub run: RunCmd,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    /// Key management cli utilities
    #[command(subcommand)]
    Key(sc_cli::KeySubcommand),

    /// Build a chain specification.
    BuildSpec(sc_cli::BuildSpecCmd),

    /// Validate blocks.
    CheckBlock(sc_cli::CheckBlockCmd),

    /// Export blocks.
    ExportBlocks(sc_cli::ExportBlocksCmd),

    /// Export the state of a given block into a chain spec.
    ExportState(sc_cli::ExportStateCmd),

    /// Import blocks.
    ImportBlocks(sc_cli::ImportBlocksCmd),

    /// Remove the whole chain.
    PurgeChain(sc_cli::PurgeChainCmd),

    /// Revert the chain to a previous state.
    Revert(sc_cli::RevertCmd),

    /// Db meta columns information.
    ChainInfo(sc_cli::ChainInfoCmd),
}
