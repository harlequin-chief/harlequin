//! Harlequin solochain runtime — the program a node executes (packaged to WASM, forkless-upgradeable).
//!
//! Composes `frame_system` + the standard pallets a working node needs (timestamp, balances, sudo,
//! transaction-payment) + the Harlequin **reputation** pallet, via `#[frame_construct_runtime]`.
//! Consensus: starts on the minimal-template path (block authoring driven by the node — manual/instant
//! seal, no Aura/Grandpa) so a node boots and produces blocks; the **Woven-Trust** committee-by-reputation
//! engine replaces it afterwards. Sovereign chain. See `../PALLET-DESIGN.md`, SPEC §2.
//!
//! NOTE: `Sudo` (root) is a TESTNET placeholder for privileged ops and runtime upgrades; pre-mainnet it
//! MUST be replaced by reputation governance (SPEC §2.5) — a single upgrade key is a centralisation
//! backdoor (Art. VI/XII).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;
use alloc::vec::Vec;

use frame::deps::sp_genesis_builder;
use frame::prelude::*;
use frame::runtime::{apis, prelude::*};
use pallet_transaction_payment::{FeeDetails, RuntimeDispatchInfo};

/// Runtime version. `spec_version` bumps on every runtime upgrade (forkless, SPEC §2.5).
#[runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: alloc::borrow::Cow::Borrowed("harlequin-runtime"),
    impl_name: alloc::borrow::Cow::Borrowed("harlequin-runtime"),
    authoring_version: 1,
    spec_version: 0,
    impl_version: 1,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 1,
    system_version: 1,
};

parameter_types! {
    pub const Version: RuntimeVersion = VERSION;
}

/// Transaction extensions (the checks a signed extrinsic carries).
type TxExtension = (
    frame_system::AuthorizeCall<Runtime>,
    frame_system::CheckNonZeroSender<Runtime>,
    frame_system::CheckSpecVersion<Runtime>,
    frame_system::CheckTxVersion<Runtime>,
    frame_system::CheckGenesis<Runtime>,
    frame_system::CheckEra<Runtime>,
    frame_system::CheckNonce<Runtime>,
    frame_system::CheckWeight<Runtime>,
    pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
    frame_system::WeightReclaim<Runtime>,
);

type Block = frame::runtime::types_common::BlockOf<Runtime, TxExtension>;
type Header = HeaderFor<Runtime>;

type RuntimeExecutive =
    Executive<Runtime, Block, frame_system::ChainContext<Runtime>, Runtime, AllPalletsWithSystem>;

#[frame_construct_runtime]
mod runtime {
    #[runtime::runtime]
    #[runtime::derive(
        RuntimeCall,
        RuntimeEvent,
        RuntimeError,
        RuntimeOrigin,
        RuntimeTask,
        RuntimeHoldReason,
        RuntimeFreezeReason,
    )]
    pub struct Runtime;

    #[runtime::pallet_index(0)]
    pub type System = frame_system::Pallet<Runtime>;

    #[runtime::pallet_index(1)]
    pub type Reputation = pallet_reputation::Pallet<Runtime>;

    #[runtime::pallet_index(2)]
    pub type Timestamp = pallet_timestamp::Pallet<Runtime>;

    #[runtime::pallet_index(3)]
    pub type Balances = pallet_balances::Pallet<Runtime>;

    #[runtime::pallet_index(4)]
    pub type Sudo = pallet_sudo::Pallet<Runtime>;

    #[runtime::pallet_index(5)]
    pub type TransactionPayment = pallet_transaction_payment::Pallet<Runtime>;
}

#[derive_impl(frame_system::config_preludes::SolochainDefaultConfig)]
impl frame_system::Config for Runtime {
    type Block = Block;
    type Version = Version;
    type AccountData = pallet_balances::AccountData<<Runtime as pallet_balances::Config>::Balance>;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Runtime {
    type AccountStore = System;
}

#[derive_impl(pallet_sudo::config_preludes::TestDefaultConfig)]
impl pallet_sudo::Config for Runtime {}

#[derive_impl(pallet_timestamp::config_preludes::TestDefaultConfig)]
impl pallet_timestamp::Config for Runtime {}

#[derive_impl(pallet_transaction_payment::config_preludes::TestDefaultConfig)]
impl pallet_transaction_payment::Config for Runtime {
    type OnChargeTransaction = pallet_transaction_payment::FungibleAdapter<Balances, ()>;
    type WeightToFee = NoFee<<Self as pallet_balances::Config>::Balance>;
    type LengthToFee = FixedFee<1, <Self as pallet_balances::Config>::Balance>;
}

impl pallet_reputation::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    /// Evidence authority: root for now (testnet placeholder for the pluggable proof system → governance).
    type EvidenceOrigin = EnsureRoot<Self::AccountId>;
    type JusticeOrigin = EnsureRoot<Self::AccountId>;
    type MaxVouches = ConstU32<128>;
    type VouchQuotaK = ConstU32<3>;
}

/// Minimal genesis presets. Concrete genesis (founding cohort §1.4 + the manifesto sealed in block 0,
/// task #26) is defined by the chain spec in the node crate; no built-in presets yet.
mod genesis_config_presets {
    use super::*;
    pub fn get_preset(_id: &PresetId) -> Option<Vec<u8>> {
        None
    }
    pub fn preset_names() -> Vec<PresetId> {
        Vec::new()
    }
}

/// Public-facing type aliases used by the runtime APIs.
pub mod interface {
    use super::Runtime;
    pub type AccountId = <Runtime as frame_system::Config>::AccountId;
    pub type Nonce = <Runtime as frame_system::Config>::Nonce;
    pub type Balance = <Runtime as pallet_balances::Config>::Balance;
}

impl_runtime_apis! {
    impl apis::Core<Block> for Runtime {
        fn version() -> RuntimeVersion {
            VERSION
        }

        fn execute_block(block: <Block as frame::traits::Block>::LazyBlock) {
            RuntimeExecutive::execute_block(block)
        }

        fn initialize_block(header: &Header) -> ExtrinsicInclusionMode {
            RuntimeExecutive::initialize_block(header)
        }
    }

    impl apis::Metadata<Block> for Runtime {
        fn metadata() -> OpaqueMetadata {
            OpaqueMetadata::new(Runtime::metadata().into())
        }

        fn metadata_at_version(version: u32) -> Option<OpaqueMetadata> {
            Runtime::metadata_at_version(version)
        }

        fn metadata_versions() -> Vec<u32> {
            Runtime::metadata_versions()
        }
    }

    impl apis::BlockBuilder<Block> for Runtime {
        fn apply_extrinsic(extrinsic: ExtrinsicFor<Runtime>) -> ApplyExtrinsicResult {
            RuntimeExecutive::apply_extrinsic(extrinsic)
        }

        fn finalize_block() -> HeaderFor<Runtime> {
            RuntimeExecutive::finalize_block()
        }

        fn inherent_extrinsics(data: InherentData) -> Vec<ExtrinsicFor<Runtime>> {
            data.create_extrinsics()
        }

        fn check_inherents(
            block: <Block as frame::traits::Block>::LazyBlock,
            data: InherentData,
        ) -> CheckInherentsResult {
            data.check_extrinsics(&block)
        }
    }

    impl apis::TaggedTransactionQueue<Block> for Runtime {
        fn validate_transaction(
            source: TransactionSource,
            tx: ExtrinsicFor<Runtime>,
            block_hash: <Runtime as frame_system::Config>::Hash,
        ) -> TransactionValidity {
            RuntimeExecutive::validate_transaction(source, tx, block_hash)
        }
    }

    impl apis::OffchainWorkerApi<Block> for Runtime {
        fn offchain_worker(header: &HeaderFor<Runtime>) {
            RuntimeExecutive::offchain_worker(header)
        }
    }

    impl apis::SessionKeys<Block> for Runtime {
        fn generate_session_keys(_owner: Vec<u8>, _seed: Option<Vec<u8>>) -> apis::OpaqueGeneratedSessionKeys {
            apis::OpaqueGeneratedSessionKeys { keys: Default::default(), proof: Default::default() }
        }

        fn decode_session_keys(
            _encoded: Vec<u8>,
        ) -> Option<Vec<(Vec<u8>, apis::KeyTypeId)>> {
            Default::default()
        }
    }

    impl apis::AccountNonceApi<Block, interface::AccountId, interface::Nonce> for Runtime {
        fn account_nonce(account: interface::AccountId) -> interface::Nonce {
            System::account_nonce(account)
        }
    }

    impl pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<Block, interface::Balance> for Runtime {
        fn query_info(uxt: ExtrinsicFor<Runtime>, len: u32) -> RuntimeDispatchInfo<interface::Balance> {
            TransactionPayment::query_info(uxt, len)
        }
        fn query_fee_details(uxt: ExtrinsicFor<Runtime>, len: u32) -> FeeDetails<interface::Balance> {
            TransactionPayment::query_fee_details(uxt, len)
        }
        fn query_weight_to_fee(weight: Weight) -> interface::Balance {
            TransactionPayment::weight_to_fee(weight)
        }
        fn query_length_to_fee(length: u32) -> interface::Balance {
            TransactionPayment::length_to_fee(length)
        }
    }

    impl apis::GenesisBuilder<Block> for Runtime {
        fn build_state(config: Vec<u8>) -> sp_genesis_builder::Result {
            build_state::<RuntimeGenesisConfig>(config)
        }

        fn get_preset(id: &Option<PresetId>) -> Option<Vec<u8>> {
            get_preset::<RuntimeGenesisConfig>(id, self::genesis_config_presets::get_preset)
        }

        fn preset_names() -> Vec<PresetId> {
            self::genesis_config_presets::preset_names()
        }
    }
}
