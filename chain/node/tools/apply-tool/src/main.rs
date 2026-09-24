//! apply-tool — MOMENT 2 of the Key ceremony: assembles and signs
//! `MultisigUpgrade::apply_upgrade(code)` fully OFFLINE (no RPC), with an ephemeral mask.
//!
//! The call is permissionless once the objection window has elapsed (`ensure_signed` only,
//! pallet lib.rs: "window elapses → anyone applies") and `MultisigUpgrade(..)` is
//! feeless-eligible, so the ephemeral mask needs no balance, holds no power, and is worthless
//! after use. Assembly mirrors devnet-driver's `submit_as` byte for byte (the logic that sealed
//! the 3/3 propose+approves on the live chain); submission happens separately over curl.

use clap::Parser;
use codec::Encode;
use harlequin_runtime::{Runtime, RuntimeCall};
use polkadot_sdk::{frame_system, pallet_transaction_payment};
use sp_core::crypto::{AccountId32, Ss58Codec};
use sp_core::{sr25519, Pair, H256};
use sp_runtime::generic::Era;
use sp_runtime::{MultiAddress, MultiSignature};

/// Must mirror the runtime's (private) TxExtension tuple exactly — the encoding is what matters.
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

type UncheckedXt = sp_runtime::generic::UncheckedExtrinsic<
    MultiAddress<AccountId32, ()>,
    RuntimeCall,
    MultiSignature,
    TxExtension,
>;

#[derive(Parser)]
struct Args {
    /// Path to the runtime wasm blob (compact.compressed).
    #[arg(long)]
    wasm: String,
    /// Ephemeral signing seed, 32 bytes hex (no 0x). Throwaway — discard after use.
    #[arg(long)]
    seed_hex: String,
    /// Announced code_hash on-chain (PendingUpgrade). Refuses to sign on mismatch.
    #[arg(long)]
    expect_hash: String,
    /// Live chain genesis hash (hex, 0x optional).
    #[arg(long)]
    genesis: String,
    #[arg(long, default_value_t = 1)]
    spec_version: u32,
    #[arg(long, default_value_t = 1)]
    tx_version: u32,
    /// Nonce of the signer (0 for a newborn mask — cold-start lane).
    #[arg(long, default_value_t = 0)]
    nonce: u32,
    /// Output file for the signed extrinsic hex.
    #[arg(long)]
    out: String,
}

fn hexb(s: &str) -> Vec<u8> {
    let s = s.trim().trim_start_matches("0x");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
        .collect()
}

fn main() {
    let a = Args::parse();

    let code = std::fs::read(&a.wasm).expect("read wasm");
    let hash = sp_core::blake2_256(&code);
    let expect = hexb(&a.expect_hash);
    assert_eq!(
        hash.to_vec(),
        expect,
        "code_hash MISMATCH — wrong wasm, refusing to sign"
    );

    let seed: [u8; 32] = hexb(&a.seed_hex).try_into().expect("seed must be 32 bytes");
    let signer = sr25519::Pair::from_seed(&seed);
    let who: AccountId32 = signer.public().into();

    let call = RuntimeCall::MultisigUpgrade(pallet_multisig_upgrade::Call::apply_upgrade {
        code: code.clone(),
    });
    let enc = call.encode();
    // The live chain sealed the propose under prefix 0x0e00 (pallet 14, call 0); apply is call 3.
    assert_eq!(enc[0], 0x0e, "pallet index drift vs live chain");
    assert_eq!(enc[1], 0x03, "call index drift (apply_upgrade = 3)");

    let genesis = H256::from_slice(&hexb(&a.genesis));
    let ext: TxExtension = (
        frame_system::AuthorizeCall::<Runtime>::new(),
        frame_system::CheckNonZeroSender::<Runtime>::new(),
        frame_system::CheckSpecVersion::<Runtime>::new(),
        frame_system::CheckTxVersion::<Runtime>::new(),
        frame_system::CheckGenesis::<Runtime>::new(),
        frame_system::CheckEra::<Runtime>::from(Era::immortal()),
        frame_system::CheckNonce::<Runtime>::from(a.nonce),
        frame_system::CheckWeight::<Runtime>::new(),
        pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0),
        frame_system::WeightReclaim::<Runtime>::new(),
    );
    let implicit: ((), (), u32, u32, H256, H256, (), (), (), ()) = (
        (),
        (),
        a.spec_version,
        a.tx_version,
        genesis,
        genesis,
        (),
        (),
        (),
        (),
    );
    let raw = (&call, &ext, &implicit).encode();
    let sig = if raw.len() > 256 {
        signer.sign(&sp_core::blake2_256(&raw)[..])
    } else {
        signer.sign(&raw)
    };
    let xt = UncheckedXt::new_signed(
        call,
        MultiAddress::Id(who.clone()),
        MultiSignature::Sr25519(sig),
        ext,
    );
    let bytes = xt.encode();
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    std::fs::write(&a.out, format!("0x{}\n", hex)).expect("write out");

    println!("signer (ephemeral): {}", who.to_ss58check());
    println!("wasm: {} bytes, blake2_256 = 0x{}", code.len(), hash.iter().map(|b| format!("{:02x}", b)).collect::<String>());
    println!("xt: {} bytes -> {}", bytes.len(), a.out);
}
