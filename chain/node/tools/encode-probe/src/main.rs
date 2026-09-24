//! Prints, for fixed inputs, exactly what the runtime types produce for a `set_vote_key` claim:
//! the call bytes, the signing payload and the final extrinsic. The browser encoder in `web/bind.js`
//! is hand-written (no CDN libraries allowed), so it is diffed against this instead of trusted.
use codec::Encode;
use harlequin_runtime::{Runtime, RuntimeCall};
use polkadot_sdk::{frame_system, pallet_transaction_payment};
use sp_core::crypto::AccountId32;
use sp_core::H256;
use sp_runtime::generic::Era;
use sp_runtime::{MultiAddress, MultiSignature};

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

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

fn main() {
    let sk_pub = [0x11u8; 32];
    let pop_sig = [0x22u8; 64];
    let account = AccountId32::new([0x33u8; 32]);
    let genesis = H256::from([0x44u8; 32]);
    let nonce: u32 = 7;
    let spec_version: u32 = 3;
    let tx_version: u32 = 1;
    let sig = [0x55u8; 64];

    let call = RuntimeCall::Reputation(pallet_reputation::Call::set_vote_key { sk_pub, pop_sig });
    println!("call    0x{}", hex(&call.encode()));

    let ext: TxExtension = (
        frame_system::AuthorizeCall::<Runtime>::new(),
        frame_system::CheckNonZeroSender::<Runtime>::new(),
        frame_system::CheckSpecVersion::<Runtime>::new(),
        frame_system::CheckTxVersion::<Runtime>::new(),
        frame_system::CheckGenesis::<Runtime>::new(),
        frame_system::CheckEra::<Runtime>::from(Era::immortal()),
        frame_system::CheckNonce::<Runtime>::from(nonce),
        frame_system::CheckWeight::<Runtime>::new(),
        pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(0),
        frame_system::WeightReclaim::<Runtime>::new(),
    );
    println!("ext     0x{}", hex(&ext.encode()));

    let implicit: ((), (), u32, u32, H256, H256, (), (), (), ()) =
        ((), (), spec_version, tx_version, genesis, genesis, (), (), (), ());
    let payload = (&call, &ext, &implicit).encode();
    println!("payload 0x{}", hex(&payload));
    println!("payload_len {}", payload.len());

    // Mortal era ground truth: the browser must reproduce BOTH the 2-byte encoding and the birth
    // block it is anchored to, or the signature commits to the wrong block hash.
    for (period, current) in [(64u64, 100u64), (64, 83_456), (256, 83_456), (64, 128)] {
        let era = Era::mortal(period, current);
        println!(
            "era p={} c={} enc=0x{} birth={}",
            period,
            current,
            hex(&era.encode()),
            era.birth(current)
        );
    }

    let xt = UncheckedXt::new_signed(
        call,
        MultiAddress::Id(account),
        MultiSignature::Sr25519(sig.into()),
        ext,
    );
    println!("xt      0x{}", hex(&xt.encode()));
}
