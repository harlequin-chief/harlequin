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
    properties.insert("tokenDecimals".to_string(), 0.into());
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
