//! Chain spec — defines genesis. Adapted from the polkadot-sdk minimal template.
//! TODO (freeze): the production spec seals the manifesto (text + hash) into genesis (task #26, Art. XII)
//! and sets the founding cohort (§1.4). For now: a development preset so the node boots and produces blocks.

use harlequin_runtime::WASM_BINARY;
use polkadot_sdk::{
    sc_service::{ChainType, Properties},
    *,
};

/// Specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec;

fn props() -> Properties {
    let mut properties = Properties::new();
    // HLQ is the daily-use currency → it MUST be divisible (you pay fractions). 12 decimals is a sensible
    // standard (final value is an economy decision, SPEC §3). 0 decimals (the template default) would make
    // it indivisible — wrong for everyday money.
    properties.insert("tokenDecimals".to_string(), 12.into());
    properties.insert("tokenSymbol".to_string(), "HLQ".into());
    properties
}

pub fn development_chain_spec() -> Result<ChainSpec, String> {
    Ok(ChainSpec::builder(
        WASM_BINARY.expect("Development wasm not available"),
        Default::default(),
    )
    .with_name("Harlequin Development")
    .with_id("hlq_dev")
    .with_chain_type(ChainType::Development)
    .with_genesis_config_preset_name(sp_genesis_builder::DEV_RUNTIME_PRESET)
    .with_properties(props())
    .build())
}

/// The LAUNCH cold-start shape (§1.4b): 5 founders, a small declared evidence seed, NO sudo,
/// NO pre-funded balances, manifesto + beacon placeholders. Mirrors how the real genesis will look (the
/// founder accounts + sealed manifesto + BTC-beacon value are swapped in at the ceremony) — used to
/// validate cold-start (committee forms from the seed, no halt; the halt guards stay sane; the seed
/// dilutes). Accounts in the preset are dev placeholders.
pub fn launch_chain_spec() -> Result<ChainSpec, String> {
    Ok(ChainSpec::builder(
        WASM_BINARY.expect("Launch wasm not available"),
        Default::default(),
    )
    .with_name("Harlequin Launch (cold-start)")
    .with_id("hlq_launch")
    .with_chain_type(ChainType::Live)
    .with_genesis_config_preset_name("launch")
    .with_properties(props())
    .build())
}
