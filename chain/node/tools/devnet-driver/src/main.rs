//! Iron test driver for the entrenchment guard (Art. VI wiring) on a LIVE devnet.
//!
//! Run against a dev-chain node with RPC open (e.g. devtest/hlq-6node.sh n1 on:19971):
//!   cargo run -p devnet-driver -- --rpc http://127.0.0.1:19971
//!
//! Phases (each one asserts; any failure exits non-zero):
//!   A. honest baseline — 6 equal founders, shares ~1/6: guard must stay quiet, finality advances.
//!   B. entrenchment — sudo-seeded evidence pushes one founder above 1/3 of consensus reputation;
//!      after REQUIRED_EPOCHS sustained the guard must HALT finality (blocks keep being produced,
//!      finalisation stops). Reversible, no punishment: exactly the contract.
//!   C. dilution — evidence for the others brings the share back under 1/3: counter resets,
//!      finality resumes on its own (self-healing, no human hand).
//!
//! The driver submits REAL signed extrinsics over RPC (same path a citizen would use) and reads the
//! guard's storage + the finalized head. No test hooks, no shortcuts: hierro.

use clap::Parser;
use codec::{Decode, Encode};
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::rpc_params;
use polkadot_sdk::{frame_system, pallet_sudo, pallet_transaction_payment};
use sp_core::crypto::{AccountId32, Ss58Codec};
use sp_core::{sr25519, twox_128, Pair, H256};
use sp_runtime::generic::Era;
use sp_runtime::{MultiAddress, MultiSignature};

use harlequin_runtime::{Runtime, RuntimeCall};
use pallet_reputation::Suit;

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

const SUITS: [Suit; 4] = [Suit::Commerce, Suit::Technical, Suit::Judicial, Suit::Governance];
const THRESHOLD_NUM: f64 = 1.0 / 3.0;
const REQUIRED_EPOCHS: u32 = 7;

#[derive(Parser)]
struct Args {
    /// HTTP RPC endpoint of a devnet node (the sudo/author node).
    #[arg(long, default_value = "http://127.0.0.1:19971")]
    rpc: String,
    /// Seconds per epoch to size timeouts (testnet: EpochLength 10 blocks x 6s = 60s).
    #[arg(long, default_value_t = 60u64)]
    epoch_secs: u64,
    /// `iron` = the full A/B/C entrenchment run. `seed` = ONLY sudo-seed `--target` with `--amount`
    /// evidence per suit and exit — the long-range (M2) harness uses it to fabricate reputation
    /// history on an isolated rival chain.
    #[arg(long, default_value = "iron")]
    mode: String,
    /// Seed target (dev URI) for `--mode seed`.
    #[arg(long, default_value = "//Ferdie")]
    target: String,
    /// Evidence per suit for `--mode seed`.
    #[arg(long, default_value_t = 100_000_000u128)]
    amount: u128,
    /// `--mode emission`: a KNOWN service-provider account (dev URI) whose HLQ balance must GROW across the
    /// era boundary — the direct proof a node was PAID the split (not just that the pool minted). The devnet
    /// authoring nodes vote-as these founders, so the leader's cold account receives the reward.
    #[arg(long, default_value = "//Alice")]
    provider: String,
}

struct Chain {
    c: HttpClient,
    genesis: H256,
    spec_version: u32,
    tx_version: u32,
}

impl Chain {
    async fn connect(url: &str) -> Self {
        let c = HttpClientBuilder::default().build(url).expect("rpc url");
        let genesis: H256 = rpc(&c, "chain_getBlockHash", rpc_params![0u32]).await;
        let v: serde_json::Value = rpc(&c, "state_getRuntimeVersion", rpc_params![]).await;
        Chain {
            c,
            genesis,
            spec_version: v["specVersion"].as_u64().unwrap() as u32,
            tx_version: v["transactionVersion"].as_u64().unwrap() as u32,
        }
    }

    async fn storage(&self, pallet: &str, item: &str) -> Option<Vec<u8>> {
        let mut key = twox_128(pallet.as_bytes()).to_vec();
        key.extend(twox_128(item.as_bytes()));
        let hex: Option<String> =
            rpc(&self.c, "state_getStorage", rpc_params![format!("0x{}", hex_str(&key))]).await;
        hex.map(|h| hex_bytes(&h))
    }

    async fn u32_storage(&self, pallet: &str, item: &str) -> u32 {
        self.storage(pallet, item)
            .await
            .map(|b| u32::decode(&mut &b[..]).expect("u32"))
            .unwrap_or(0)
    }

    async fn u64_storage(&self, pallet: &str, item: &str) -> u64 {
        self.storage(pallet, item)
            .await
            .map(|b| u64::decode(&mut &b[..]).expect("u64"))
            .unwrap_or(0)
    }

    async fn bool_storage(&self, pallet: &str, item: &str) -> bool {
        self.storage(pallet, item)
            .await
            .map(|b| bool::decode(&mut &b[..]).expect("bool"))
            .unwrap_or(false)
    }

    /// Read `Tokens::Minted[coin]` (StorageMap, Blake2_128Concat) — cumulative emission of a coin.
    /// `coin_variant` = SCALE index of `Coin` (0 = Hlq, 1 = Sov). Absent key → 0 (ValueQuery).
    async fn minted(&self, coin_variant: u8) -> u128 {
        let mut key = twox_128("Tokens".as_bytes()).to_vec();
        key.extend(twox_128("Minted".as_bytes()));
        // Blake2_128Concat(coin.encode): the enum encodes as its 1-byte variant index.
        let enc = [coin_variant];
        key.extend(sp_core::blake2_128(&enc));
        key.extend_from_slice(&enc);
        let hex: Option<String> =
            rpc(&self.c, "state_getStorage", rpc_params![format!("0x{}", hex_str(&key))]).await;
        hex.map(|h| u128::decode(&mut &hex_bytes(&h)[..]).expect("u128")).unwrap_or(0)
    }

    /// Read `Tokens::Accounts[account].hlq` (StorageMap, Blake2_128Concat; value = `Balances { hlq, sov }`,
    /// hlq first). The DIRECT proof a provider was paid: its HLQ balance. Absent key → 0.
    async fn balance_hlq(&self, account: &[u8; 32]) -> u128 {
        let mut key = twox_128("Tokens".as_bytes()).to_vec();
        key.extend(twox_128("Accounts".as_bytes()));
        key.extend(sp_core::blake2_128(account));
        key.extend_from_slice(account);
        let hex: Option<String> =
            rpc(&self.c, "state_getStorage", rpc_params![format!("0x{}", hex_str(&key))]).await;
        // Balances encodes hlq (u128) then sov (u128); decode the leading u128 = hlq.
        hex.map(|h| u128::decode(&mut &hex_bytes(&h)[..]).expect("hlq u128")).unwrap_or(0)
    }

    async fn best_number(&self) -> u64 {
        let h: serde_json::Value = rpc(&self.c, "chain_getHeader", rpc_params![]).await;
        u64::from_str_radix(h["number"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
    }

    async fn finalized_number(&self) -> u64 {
        let hash: H256 = rpc(&self.c, "chain_getFinalizedHead", rpc_params![]).await;
        let h: serde_json::Value = rpc(&self.c, "chain_getHeader", rpc_params![hash]).await;
        u64::from_str_radix(h["number"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap()
    }

    /// max single-entity share of consensus reputation, via the same runtime API the committee uses.
    async fn max_share(&self) -> f64 {
        let raw: String = rpc(
            &self.c,
            "state_call",
            rpc_params!["HarlequinConsensusApi_consensus_reputation", "0x"],
        )
        .await;
        let bytes = hex_bytes(&raw);
        let reps = <Vec<([u8; 32], i128)>>::decode(&mut &bytes[..]).expect("reps decode");
        let total: i128 = reps.iter().map(|(_, r)| *r).sum();
        if total <= 0 {
            return 0.0;
        }
        let max = reps.iter().map(|(_, r)| *r).max().unwrap_or(0);
        max as f64 / total as f64
    }

    async fn submit_as(&self, signer: &sr25519::Pair, call: RuntimeCall) -> H256 {
        let who: AccountId32 = signer.public().into();
        let nonce: u32 = rpc(
            &self.c,
            "system_accountNextIndex",
            rpc_params![who.to_ss58check()],
        )
        .await;
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
        // Signing payload = (call, extension, implicit). implicit needs chain storage, so a client
        // supplies the values by hand: spec/tx version (CheckSpecVersion/CheckTxVersion), genesis hash
        // (CheckGenesis), era anchor hash (CheckEra, = genesis for immortal); the rest are .
        let implicit: ((), (), u32, u32, H256, H256, (), (), (), ()) = (
            (),
            (),
            self.spec_version,
            self.tx_version,
            self.genesis,
            self.genesis,
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
            MultiAddress::Id(who),
            MultiSignature::Sr25519(sig),
            ext,
        );
        rpc(
            &self.c,
            "author_submitExtrinsic",
            rpc_params![format!("0x{}", hex_str(&xt.encode()))],
        )
        .await
    }
}

async fn rpc<T: serde::de::DeserializeOwned>(
    c: &HttpClient,
    method: &str,
    params: jsonrpsee::core::params::ArrayParams,
) -> T {
    c.request(method, params).await.unwrap_or_else(|e| panic!("rpc {method}: {e}"))
}

fn hex_str(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn hex_bytes(s: &str) -> Vec<u8> {
    let s = s.trim_start_matches("0x");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// sudo(submit_evidence(who, suit, amount)) for every suit.
fn seed_evidence_calls(who: &AccountId32, amount: u128) -> Vec<RuntimeCall> {
    SUITS
        .iter()
        .map(|s| {
            RuntimeCall::Sudo(pallet_sudo::Call::sudo {
                call: Box::new(RuntimeCall::Reputation(pallet_reputation::Call::submit_evidence {
                    who: who.clone(),
                    suit: *s,
                    amount,
                })),
            })
        })
        .collect()
}

async fn wait_epoch_change(chain: &Chain, epoch_secs: u64) -> u64 {
    let start = chain.u64_storage("Reputation", "Epoch").await;
    for _ in 0..(epoch_secs * 3) {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let now = chain.u64_storage("Reputation", "Epoch").await;
        if now != start {
            return now;
        }
    }
    panic!("epoch never advanced past {start} (waited {}s)", epoch_secs * 3);
}

struct Report {
    pass: u32,
    fail: u32,
}
impl Report {
    fn check(&mut self, name: &str, ok: bool, detail: String) {
        if ok {
            self.pass += 1;
            println!("  PASS  {name}  ({detail})");
        } else {
            self.fail += 1;
            println!("  FAIL  {name}  ({detail})");
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();
    let chain = Chain::connect(&args.rpc).await;
    let alice = sr25519::Pair::from_string("//Alice", None).unwrap();
    let dave: AccountId32 =
        sr25519::Pair::from_string("//Dave", None).unwrap().public().into();
    let others = ["//Alice", "//Bob", "//Charlie", "//Eve", "//Ferdie"]
        .map(|s| -> AccountId32 { sr25519::Pair::from_string(s, None).unwrap().public().into() });
    let mut r = Report { pass: 0, fail: 0 };

    if args.mode == "emission" {
        // P-2/#838 station-band: the disinflationary emission mints from the non-forgeable participation
        // feed (author + finality) with NO Root/Sudo — the mint that was DEAD on a king-less mainnet.
        // Evidence: Tokens::Epoch advances (run_epoch fired in on_finalize at the era boundary) AND
        // Tokens::Minted[Hlq] grows. Minted only grows when the reward vector is non-empty (audit 🔴#2:
        // no receivers → nothing minted), so Minted>0 ALSO proves nodes were credited.
        println!("== EMISSION TEST (P-2/#838): king-less mint from the participation feed ==");
        println!("rpc={} genesis={:?} spec={}", args.rpc, chain.genesis, chain.spec_version);
        let provider: [u8; 32] =
            sr25519::Pair::from_string(&args.provider, None).unwrap().public().0;
        let e0 = chain.u64_storage("Tokens", "Epoch").await;
        let hlq0 = chain.minted(0).await;
        let sov0 = chain.minted(1).await;
        let bal0 = chain.balance_hlq(&provider).await;
        println!(
            "[start]  Tokens.Epoch={e0}  Minted.Hlq={hlq0}  Minted.Sov={sov0}  provider({}).Hlq={bal0}",
            args.provider
        );
        // Wait for an era boundary: Tokens::Epoch to bump (advances once per EraLength in on_finalize).
        let mut e1 = e0;
        for _ in 0..(args.epoch_secs * 6) {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            e1 = chain.u64_storage("Tokens", "Epoch").await;
            if e1 > e0 {
                break;
            }
        }
        let hlq1 = chain.minted(0).await;
        let sov1 = chain.minted(1).await;
        let bal1 = chain.balance_hlq(&provider).await;
        println!(
            "[boundary]  Tokens.Epoch={e1}  Minted.Hlq={hlq1}  Minted.Sov={sov1}  provider({}).Hlq={bal1}",
            args.provider
        );
        r.check("emission.era-advanced", e1 > e0, format!("Tokens.Epoch {e0} -> {e1}"));
        r.check(
            "emission.hlq-minted-no-root",
            hlq1 > hlq0,
            format!("Minted.Hlq {hlq0} -> {hlq1} (mint from feed, no Sudo)"),
        );
        // DIRECT proof the node was PAID (not inferred from Minted): the provider's HLQ balance grew by the
        // split. This is the exact #838 failure a pool-mint alone would hide (mint-to-nowhere).
        r.check(
            "emission.provider-paid",
            bal1 > bal0,
            format!("provider {} HLQ {bal0} -> {bal1} (received the split)", args.provider),
        );
        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
    }

    if args.mode == "seed" {
        // M2 long-range harness: fabricate reputation history on THIS (rival, isolated) chain.
        let target: AccountId32 =
            sr25519::Pair::from_string(&args.target, None).unwrap().public().into();
        for call in seed_evidence_calls(&target, args.amount) {
            chain.submit_as(&alice, call).await;
        }
        println!("SEEDED {} amount={} per suit (sudo) genesis={:?}", args.target, args.amount, chain.genesis);
        return;
    }

    println!("== IRON TEST: entrenchment guard (Art. VI) ==");
    println!("rpc={} genesis={:?} spec={}", args.rpc, chain.genesis, chain.spec_version);

    // ── Phase A: honest baseline ─────────────────────────────────────────────
    println!("\n[A] honest baseline (6 equal founders)");
    let e = wait_epoch_change(&chain, args.epoch_secs).await; // ensure ≥1 recompute ran
    let share = chain.max_share().await;
    let halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
    let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
    r.check("A.share-under-third", share < THRESHOLD_NUM, format!("share={share:.4} epoch={e}"));
    r.check("A.not-halted", !halted && counter == 0, format!("halted={halted} counter={counter}"));
    let f0 = chain.finalized_number().await;
    wait_epoch_change(&chain, args.epoch_secs).await;
    let f1 = chain.finalized_number().await;
    r.check("A.finality-advances", f1 > f0, format!("finalized {f0} -> {f1}"));

    // ── Phase B: entrench Dave above 1/3 ─────────────────────────────────────
    println!("\n[B] entrenchment: sudo-seed Dave to ~48% and hold {REQUIRED_EPOCHS} epochs");
    for call in seed_evidence_calls(&dave, 3_600) {
        chain.submit_as(&alice, call).await;
    }
    // walk epochs; counter must climb once share > 1/3, halt at REQUIRED_EPOCHS.
    let mut halted_at: Option<u32> = None;
    for _ in 0..(REQUIRED_EPOCHS + 3) {
        let e = wait_epoch_change(&chain, args.epoch_secs).await;
        let share = chain.max_share().await;
        let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
        let halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
        println!("  epoch={e} share={share:.4} counter={counter} halted={halted}");
        if halted {
            halted_at = Some(counter);
            break;
        }
    }
    r.check(
        "B.halts-after-required",
        halted_at.map(|c| c >= REQUIRED_EPOCHS).unwrap_or(false),
        format!("halted_at_counter={halted_at:?}"),
    );
    // Finality must PLATEAU while production continues. NOTE the committee-epoch pinning lag: the guard
    // seats an empty committee only for heights whose EPOCH-START state already has halted=true, so
    // in-flight PRE-halt heights keep finalising (you never retroactively un-finalise — Art. IX). So we
    // don't assert an instant freeze; we poll until `finalized` stops climbing (two equal samples across
    // a full epoch), then confirm it stays put while `best` keeps rising.
    let mut plateau = chain.finalized_number().await;
    let mut stalled = false;
    for _ in 0..6 {
        wait_epoch_change(&chain, args.epoch_secs).await;
        let now = chain.finalized_number().await;
        if now == plateau {
            stalled = true;
            break;
        }
        plateau = now;
    }
    let bh = chain.best_number().await;
    wait_epoch_change(&chain, args.epoch_secs).await;
    let fh2 = chain.finalized_number().await;
    let bh2 = chain.best_number().await;
    r.check(
        "B.finality-plateaued",
        stalled && fh2 == plateau,
        format!("finalized plateau={plateau} still={fh2}"),
    );
    r.check("B.production-continues", bh2 > bh, format!("best {bh} -> {bh2}"));

    // ── Phase C: dilution → self-heal ────────────────────────────────────────
    println!("\n[C] dilution: seed the other 5 founders, share must fall, finality must resume");
    for acc in &others {
        for call in seed_evidence_calls(acc, 4_000) {
            chain.submit_as(&alice, call).await;
        }
    }
    let mut resumed = false;
    let mut share_after = 1.0;
    for _ in 0..5 {
        wait_epoch_change(&chain, args.epoch_secs).await;
        share_after = chain.max_share().await;
        let halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
        let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
        println!("  share={share_after:.4} counter={counter} halted={halted}");
        if !halted && counter == 0 {
            resumed = true;
            break;
        }
    }
    r.check("C.share-diluted", share_after < THRESHOLD_NUM, format!("share={share_after:.4}"));
    r.check("C.counter-reset-unhalted", resumed, "halted=false counter=0".into());
    // Finality must CROSS the frozen band and resume. Poll several epochs: the burial-K guard delays the
    // re-anchor until H_r is buried ≥ FINALITY_STEP, so recovery is not instant on the dilution epoch.
    let f_before = chain.finalized_number().await;
    let mut f_after = f_before;
    for _ in 0..6 {
        wait_epoch_change(&chain, args.epoch_secs).await;
        f_after = chain.finalized_number().await;
        if f_after > f_before {
            break;
        }
    }
    r.check("C.finality-resumed", f_after > f_before, format!("finalized {f_before} -> {f_after}"));

    println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
    std::process::exit(if r.fail == 0 { 0 } else { 1 });
}
