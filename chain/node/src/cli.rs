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
    /// Like `WovenTrust` but the node does NOT author blocks — it only follows the chain and runs the
    /// Snowball finality gadget (votes). Lets a devnet separate the single author from extra voters so
    /// the committee reaches quorum without every node forking by authoring each slot.
    WovenTrustVoteOnly(u64),
    /// Pure follower: no authoring AND no finality gadget — it only syncs blocks and accepts finality
    /// EXCLUSIVELY through verified imported justifications (`JustificationImport`). Used to isolate and
    /// prove step-3b import-time verification: this node finalises only when a peer hands it a sound proof.
    Follower,
}

impl std::str::FromStr for Consensus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if s == "instant-seal" {
            Consensus::InstantSeal
        } else if s == "follower" {
            Consensus::Follower
        } else if let Some(block_time) = s.strip_prefix("manual-seal-") {
            Consensus::ManualSeal(block_time.parse().map_err(|_| "invalid block time")?)
        } else if let Some(block_time) = s.strip_prefix("woven-trust-voteonly-") {
            // Checked before `woven-trust-` so the longer prefix wins.
            Consensus::WovenTrustVoteOnly(block_time.parse().map_err(|_| "invalid block time")?)
        } else if let Some(block_time) = s.strip_prefix("woven-trust-") {
            Consensus::WovenTrust(block_time.parse().map_err(|_| "invalid block time")?)
        } else {
            return Err("incorrect consensus identifier".into());
        })
    }
}

/// Default `--consensus`. A production (`mainnet`) binary defaults to the engine every published door
/// passes explicitly — installer unit, foreground script, Dockerfile CMD — so a stranger who runs
/// `harlequin-node --chain mainnet-raw.json` with no flags follows the real chain. It used to default to
/// `instant-seal` in every build (): that stranger got a node sealing on its own, and the
/// published docker-compose, which forgot the flag, shipped exactly that. `woven-trust-<ms>` only
/// authors when sortition elects the node, which a node without reputation never is.
/// Test/dev builds keep `instant-seal` for quick local chains.
#[cfg(feature = "mainnet")]
pub const DEFAULT_CONSENSUS: &str = "woven-trust-12000";
#[cfg(not(feature = "mainnet"))]
pub const DEFAULT_CONSENSUS: &str = "instant-seal";

#[derive(Debug, clap::Parser)]
pub struct Cli {
    #[command(subcommand)]
    pub subcommand: Option<Subcommand>,

    /// The consensus (block authoring) to use: `instant-seal`, `manual-seal-<ms>`,
    /// `woven-trust-<ms>` (reputation-weighted sortition gates authoring — the real engine), or
    /// `woven-trust-voteonly-<ms>` (no authoring; only follows the chain and runs the finality vote).
    /// Default: `woven-trust-12000` in a mainnet build, `instant-seal` otherwise (see `DEFAULT_CONSENSUS`).
    #[clap(long, default_value = DEFAULT_CONSENSUS)]
    pub consensus: Consensus,

    /// Sign Woven-Trust finality votes as this account (sr25519 secret URI, e.g. `//Alice`). The signer
    /// must be a member of the elected committee for its vote to count. Omit on observer/non-voting nodes.
    /// On real nodes prefer `--vote-as-file` so the secret never lands in argv (`ps` / `/proc/cmdline`).
    #[clap(long, conflicts_with = "vote_as_file")]
    pub vote_as: Option<String>,

    /// Like `--vote-as`, but reads the sr25519 secret URI from a file (keep it `0600`) instead of the
    /// command line, so the hot session secret never appears in the process arguments. The file's whole
    /// contents are trimmed and used as the secret URI.
    #[clap(long)]
    pub vote_as_file: Option<String>,

    /// Keep sealing when this node sees NO peers or is still major-syncing. Off by default: an isolated
    /// woven-trust leader skips its slot instead of building a private branch (ct103, 27-ago and 17-sep).
    /// Only for a deliberate one-node chain, where "no peers" is the whole network by design.
    #[clap(long)]
    pub seal_without_peers: bool,

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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn default_consensus_matches_build() {
        let cli = Cli::try_parse_from(["harlequin-node"]).expect("no-flag parse");
        #[cfg(feature = "mainnet")]
        assert!(matches!(cli.consensus, Consensus::WovenTrust(12000)), "mainnet default must follow the real chain");
        #[cfg(not(feature = "mainnet"))]
        assert!(matches!(cli.consensus, Consensus::InstantSeal));
    }

    #[test]
    fn explicit_consensus_still_wins() {
        let cli = Cli::try_parse_from(["harlequin-node", "--consensus", "follower"]).expect("parse");
        assert!(matches!(cli.consensus, Consensus::Follower));
    }
}
