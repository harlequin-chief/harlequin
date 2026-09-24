//! Iron test driver for the entrenchment guard (Art. VI wiring) on a LIVE devnet.
//!
//! Run against a dev-chain node with RPC open (e.g. devtest/hlq-6node.sh n1 on :19971):
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
    /// `--mode cluster` only: submit every seed/vouch batch in REVERSED order. The C' determinism
    /// check runs the scenario twice (plain, then --shuffle) on fresh chains and diffs the final
    /// STATE line — order-independent keying must make them byte-identical.
    #[arg(long, default_value_t = false)]
    shuffle: bool,
    /// `--mode ceremony` / `--mode apply`: path to the candidate runtime blob.
    #[arg(long, default_value = "")]
    wasm: String,
    /// FLAG-1 declaration: the caps must match THE CHAIN's, not this binary's. The driver links the
    /// runtime WITHOUT the `mainnet` feature (it needs Sudo, which mainnet drops), so its compiled-in
    /// caps are the testnet ones — declaring those against a mainnet chain is rejected, and silently.
    /// Defaults here are the MAINNET caps (540M / 54M x 10^12); override for a testnet-flavoured chain.
    #[arg(long, default_value_t = 540_000_000u128 * 1_000_000_000_000u128)]
    cap_hlq: u128,
    #[arg(long, default_value_t = 54_000_000u128 * 1_000_000_000_000u128)]
    cap_sov: u128,
    /// `--mode heal918` only: comma-separated peer RPC endpoints. The determinism check asserts every
    /// one of them crosses the healed band on its own AND finalises the same canonical hash — including
    /// a peer the runner restarted mid-halt (a lagging `best` must not change the skip target).
    #[arg(long, default_value = "")]
    peers: String,
    /// `--mode encode-propose`: if set (>=0), also emit the UNSIGNED SIGNING PAYLOAD (call + tx-extension
    /// + implicit, with this nonce) that the offline signer signs verbatim. Which call: `--which`.
    #[arg(long, default_value_t = -1)]
    nonce: i64,
    /// `--mode encode-propose`: which call the signing payload wraps — `propose` or `approve`.
    #[arg(long, default_value = "propose")]
    which: String,
    /// `--mode encode-propose` with `--nonce` and `--sig`: verify this sr25519 signature over the
    /// rebuilt payload against `--pubkey` BEFORE anyone submits it. 0x-hex, 64 bytes.
    #[arg(long, default_value = "")]
    sig: String,
    /// `--mode encode-propose`: signer public key (0x-hex, 32 bytes) to verify `--sig` against.
    #[arg(long, default_value = "")]
    pubkey: String,
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

    /// Count the entries of a storage MAP (prefix = twox128(pallet) ++ twox128(item)).
    async fn map_count(&self, pallet: &str, item: &str) -> usize {
        let mut prefix = twox_128(pallet.as_bytes()).to_vec();
        prefix.extend(twox_128(item.as_bytes()));
        let keys: Vec<String> = rpc(
            &self.c,
            "state_getKeys",
            rpc_params![format!("0x{}", hex_str(&prefix))],
        )
        .await;
        keys.len()
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
        // Blake2_128Concat(coin.encode()): the enum encodes as its 1-byte variant index.
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

    /// Like [`Self::submit_as`] but SURFACES the node's rejection instead of panicking. The newcomer
    /// scenario (G6) exists precisely to observe a refusal — a helper that panics on error can only
    /// test happy paths, which is how the "newcomer cannot pay to register their node" hole survived
    /// this long.
    async fn submit_as_try(
        &self,
        signer: &sr25519::Pair,
        call: RuntimeCall,
    ) -> Result<H256, String> {
        let who: AccountId32 = signer.public().into();
        let nonce: u32 = self
            .c
            .request("system_accountNextIndex", rpc_params![who.to_ss58check()])
            .await
            .map_err(|e| format!("nonce: {e}"))?;
        let xt = self.sign_xt(signer, call, nonce);
        self.c
            .request(
                "author_submitExtrinsic",
                rpc_params![format!("0x{}", hex_str(&xt))],
            )
            .await
            .map_err(|e| e.to_string())
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
        // Signing payload = (call, extension, implicit). implicit() needs chain storage, so a client
        // supplies the values by hand: spec/tx version (CheckSpecVersion/CheckTxVersion), genesis hash
        // (CheckGenesis), era anchor hash (CheckEra, = genesis for immortal); the rest are ().
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

    /// Sign an extrinsic without submitting it — same envelope as [`Self::submit_as`], factored out so
    /// the fallible path ([`Self::submit_as_try`]) builds a byte-identical transaction.
    fn sign_xt(&self, signer: &sr25519::Pair, call: RuntimeCall, nonce: u32) -> Vec<u8> {
        let who: AccountId32 = signer.public().into();
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
        UncheckedXt::new_signed(
            call,
            MultiAddress::Id(who),
            MultiSignature::Sr25519(sig),
            ext,
        )
        .encode()
    }
}

impl Chain {
    /// All `Reputation::ClusterGuard` events in the last `depth` blocks (the guard emits on the
    /// epoch-boundary block, which `wait_epoch_change` may already have walked past). Oldest first.
    async fn recent_cluster_events(&self, depth: u64) -> Vec<(i128, bool)> {
        let mut key = twox_128("System".as_bytes()).to_vec();
        key.extend(twox_128("Events".as_bytes()));
        let key_hex = format!("0x{}", hex_str(&key));
        let best = self.best_number().await;
        let mut out = Vec::new();
        for n in best.saturating_sub(depth)..=best {
            let hash: Option<H256> = rpc(&self.c, "chain_getBlockHash", rpc_params![n]).await;
            let Some(hash) = hash else { continue };
            let raw: Option<String> =
                rpc(&self.c, "state_getStorage", rpc_params![key_hex.clone(), hash]).await;
            let Some(raw) = raw else { continue };
            let bytes = hex_bytes(&raw);
            type Records = Vec<frame_system::EventRecord<harlequin_runtime::RuntimeEvent, H256>>;
            let Ok(records) = Records::decode(&mut &bytes[..]) else { continue };
            for rec in records {
                if let harlequin_runtime::RuntimeEvent::Reputation(
                    pallet_reputation::Event::ClusterGuard { cluster_share_fp, armed },
                ) = rec.event
                {
                    out.push((cluster_share_fp, armed));
                }
            }
        }
        out
    }
}

impl Chain {
    /// Read a `Blake2_128Concat` StorageMap entry: `twox128(pallet)‖twox128(item)‖blake2_128(k)‖k`.
    async fn map_raw(&self, pallet: &str, item: &str, key_enc: &[u8]) -> Option<Vec<u8>> {
        let mut k = twox_128(pallet.as_bytes()).to_vec();
        k.extend(twox_128(item.as_bytes()));
        k.extend(sp_core::blake2_128(key_enc));
        k.extend_from_slice(key_enc);
        let hex: Option<String> =
            rpc(&self.c, "state_getStorage", rpc_params![format!("0x{}", hex_str(&k))]).await;
        hex.map(|h| hex_bytes(&h))
    }
    /// `Reputation::VoteKeys[account]` → the delegated hot session pubkey, if any.
    async fn vote_key_of(&self, acc: &AccountId32) -> Option<[u8; 32]> {
        self.map_raw("Reputation", "VoteKeys", acc.as_ref())
            .await
            .map(|b| <[u8; 32]>::decode(&mut &b[..]).expect("vote key 32B"))
    }
    /// `Reputation::VoteKeyOwner[sk_pub]` → the cold account it is delegated for, if any.
    async fn vote_key_owner(&self, sk_pub: &[u8; 32]) -> Option<AccountId32> {
        self.map_raw("Reputation", "VoteKeyOwner", sk_pub)
            .await
            .map(|b| AccountId32::decode(&mut &b[..]).expect("owner acc"))
    }
}

/// vouch(target, suit=Commerce, weight=1) signed by `who` — the citizen path, no sudo.
fn vouch_call(target: &AccountId32) -> RuntimeCall {
    RuntimeCall::Reputation(pallet_reputation::Call::vouch {
        target: target.clone(),
        suit: Suit::Commerce,
        weight: 1,
    })
}


/// Current best block number (used by the ceremony to report where the objection window starts).

/// The citizen actions a newcomer would attempt, in order. Kept in one place so part A and the
/// fee-gate table can never drift apart.
fn alloc_steps(_peer: &sr25519::Pair) -> Vec<(&'static str, RuntimeCall)> {
    let body = sp_core::blake2_256(b"harlequin g7: a stranger tries to speak");
    let detail = sp_core::blake2_256(b"harlequin g7: a stranger tries to sell");
    vec![
        ("register a name", RuntimeCall::Directory(pallet_directory::Call::register {})),
        ("speak in the forum", RuntimeCall::Forum(pallet_forum::Call::post { body, parent: None })),
        (
            "publish an offer",
            RuntimeCall::Market(pallet_market::Call::publish {
                detail,
                category: b"general".to_vec().try_into().expect("category fits"),
            }),
        ),
    ]
}

/// The three numbers that say whether anyone actually LIVES in the society: masks with a name, things
/// said, things offered. Read from chain state, never from logs.
async fn society_counts(c: &Chain) -> (u64, u64, u64) {
    (
        c.u64_storage("Directory", "MaskCount").await,
        c.u64_storage("Forum", "NextId").await,
        c.u64_storage("Market", "NextId").await,
    )
}

async fn head_number_of(c: &Chain) -> u64 {
    let h: serde_json::Value = rpc(&c.c, "chain_getHeader", rpc_params![]).await;
    let n = h["number"].as_str().unwrap_or("0x0").trim_start_matches("0x");
    u64::from_str_radix(n, 16).unwrap_or(0)
}

/// Sleep until the chain has advanced `n` blocks (the lab runs sub-second blocks, so this is quick).
async fn wait_blocks(c: &Chain, n: u64) {
    let start = head_number_of(c).await;
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        if head_number_of(c).await >= start + n {
            return;
        }
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

    if args.mode == "encode-propose" {
        // LIVE CEREMONY (): emit ONLY the unsigned `propose_upgrade{code_hash, declared}`
        // call hex, built against the LIVE chain's sealed truth. Does NOT sign and does NOT submit —
        // the founder mask signs it in an offline signer, the reviewer assembles + submits. Same declaration
        // composition as `--mode ceremony`, so a candidate that this prints is exactly one FLAG-1 accepts.
        let code = std::fs::read(&args.wasm).unwrap_or_else(|e| panic!("--wasm: {e}"));
        let code_hash = sp_core::blake2_256(&code);
        let manifesto_hash: [u8; 32] = chain
            .storage("Manifesto", "ManifestoHash")
            .await
            .map(|b| <[u8; 32]>::decode(&mut &b[..]).expect("manifesto hash"))
            .expect("no sealed manifesto on this chain — nothing can be upgraded");
        let declared = pallet_multisig_upgrade::InvariantDeclaration {
            manifesto_hash,
            cap_hlq: args.cap_hlq,
            cap_sov: args.cap_sov,
            entrenchment_threshold_fp: pallet_reputation::pallet::ENTRENCHMENT_THRESHOLD_FP,
            entrenchment_required_epochs: pallet_reputation::pallet::ENTRENCHMENT_REQUIRED_EPOCHS,
            preserves_validator: true,
        };
        let propose = RuntimeCall::MultisigUpgrade(
            pallet_multisig_upgrade::Call::propose_upgrade { code_hash, declared },
        );
        let approve = RuntimeCall::MultisigUpgrade(
            pallet_multisig_upgrade::Call::approve_upgrade { code_hash },
        );
        println!("live spec_version = {} tx_version = {}", chain.spec_version, chain.tx_version);
        println!("genesis = 0x{}", hex_str(chain.genesis.as_ref()));
        println!("manifesto_hash = 0x{}", hex_str(&manifesto_hash));
        println!("code_hash = 0x{}", hex_str(&code_hash));
        println!("declared.cap_hlq = {}", args.cap_hlq);
        println!("declared.cap_sov = {}", args.cap_sov);
        println!(
            "declared.entrenchment_threshold_fp = {}",
            pallet_reputation::pallet::ENTRENCHMENT_THRESHOLD_FP
        );
        println!(
            "declared.entrenchment_required_epochs = {}",
            pallet_reputation::pallet::ENTRENCHMENT_REQUIRED_EPOCHS
        );
        println!("PROPOSE_CALL_HEX = 0x{}", hex_str(&propose.encode()));
        println!("APPROVE_CALL_HEX = 0x{}", hex_str(&approve.encode()));

        if args.nonce >= 0 {
            // The exact signing payload the live chain verifies: `(call, ext, implicit).encode()`.
            // Same construction as `sign_xt`, but we print it instead of signing — the founder's
            // offline signer signs these bytes verbatim (blake2_256 first iff len > 256).
            let call = if args.which == "approve" { approve } else { propose };
            let nonce = args.nonce as u32;
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
            let implicit: ((), (), u32, u32, H256, H256, (), (), (), ()) = (
                (),
                (),
                chain.spec_version,
                chain.tx_version,
                chain.genesis,
                chain.genesis,
                (),
                (),
                (),
                (),
            );
            let raw = (&call, &ext, &implicit).encode();
            println!("PAYLOAD which={} nonce={} len={}", args.which, nonce, raw.len());
            println!("SIGN_OVER = {}", if raw.len() > 256 { "blake2_256(payload)" } else { "payload verbatim" });
            println!("PAYLOAD_HEX = 0x{}", hex_str(&raw));

            if !args.sig.is_empty() && !args.pubkey.is_empty() {
                // Verify the offline signature BEFORE the reviewer submits — a bad sig caught here costs
                // nothing; caught on chain wastes a submit and confuses the signer. sr25519 signs the
                // payload verbatim iff len <= 256, else blake2_256(payload) (sp_core convention).
                let sig_bytes = hex_bytes(&args.sig);
                let pk_bytes = hex_bytes(&args.pubkey);
                let mut sb = [0u8; 64];
                sb.copy_from_slice(&sig_bytes);
                let sig = sr25519::Signature::from(sb);
                let pk = sr25519::Public::from(<[u8; 32]>::try_from(&pk_bytes[..]).expect("32-byte pubkey"));
                let ok = if raw.len() > 256 {
                    sr25519::Pair::verify(&sig, sp_core::blake2_256(&raw), &pk)
                } else {
                    sr25519::Pair::verify(&sig, &raw[..], &pk)
                };
                println!("SIG_VALID = {}", ok);
                if ok {
                    // Assemble the signed extrinsic (external signature) ready for author_submitExtrinsic.
                    let xt = UncheckedXt::new_signed(
                        call,
                        MultiAddress::Id(AccountId32::from(
                            <[u8; 32]>::try_from(&pk_bytes[..]).unwrap(),
                        )),
                        MultiSignature::Sr25519(sig),
                        ext,
                    );
                    println!("EXTRINSIC_HEX = 0x{}", hex_str(&xt.encode()));
                }
                std::process::exit(if ok { 0 } else { 2 });
            }
        }
        return;
    }

    if args.mode == "beacon" {
        // Bootstrap the FINALITY COMMITTEE by driving the beacon commit-reveal pipeline for the founders.
        // Plain validator nodes do NOT emit commit/reveal, so on a fresh devnet Active stays empty and
        // finality never starts. Pipeline: commit -> (roll) Awaiting -> reveal -> Staged -> (roll) Active.
        // Committing every epoch keeps Active populated so finality runs; then the transition question
        // (old fa79dda9 node vs new #837 vote_keys layout) becomes observable at the boundary.
        println!("== BEACON BOOTSTRAP: drive commit-reveal so the finality committee (Active) populates ==");
        println!("rpc={} genesis={:?} spec={}", args.rpc, chain.genesis, chain.spec_version);
        let founders: Vec<(&str, sr25519::Pair)> = ["//Alice", "//Bob", "//Charlie", "//Dave"]
            .iter()
            .map(|s| (*s, sr25519::Pair::from_string(s, None).unwrap()))
            .collect();
        let mut prev: Vec<Option<Vec<u8>>> = vec![None; founders.len()];
        let rounds: u64 = 9;
        for round in 0..rounds {
            for (i, (name, pair)) in founders.iter().enumerate() {
                // reveal the previous round's commitment (now promoted Pending->Awaiting by the roll)
                if let Some(sec) = prev[i].clone() {
                    let call =
                        RuntimeCall::BeaconPallet(pallet_beacon::Call::reveal { secret: sec });
                    let _ = chain.submit_as(pair, call).await;
                }
                // commit a fresh secret for this round
                let secret = format!("{name}-beacon-{round}").into_bytes();
                let commitment = consensus_core::beacon::commit(&secret);
                let call =
                    RuntimeCall::BeaconPallet(pallet_beacon::Call::commit { commitment });
                chain.submit_as(pair, call).await;
                prev[i] = Some(secret);
            }
            let active = chain.map_count("BeaconPallet", "Active").await;
            let fin = chain.finalized_number().await;
            let ep = chain.u64_storage("Reputation", "Epoch").await;
            println!("[round {round}] epoch={ep} Active(committee)={active} finalized=#{fin}");
            wait_epoch_change(&chain, args.epoch_secs).await;
        }
        let active = chain.map_count("BeaconPallet", "Active").await;
        let fin = chain.finalized_number().await;
        println!("[final] Active(committee)={active} finalized=#{fin}");
        r.check(
            "beacon.committee-populated",
            active >= 4,
            format!("BeaconPallet.Active={active} (expect the 4 founders)"),
        );
        r.check(
            "beacon.finalizing",
            fin > 0,
            format!("finalized=#{fin} (>0 means the finality committee is live)"),
        );
        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
    }

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

    if args.mode == "heal918" {
        // #918 iron: after an entrenchment halt heals, finality must RESUME BY ITSELF — the gadget
        // SKIPS its target past the frozen band and finalises it transitively (direction b, see
        // DESIGN-918-SELFHEAL-FIX.md). Two full halt→heal cycles run on ONE continuous chain, so the
        // second resume must skip past a chain that already carries a healed band (multi-band case).
        // `--peers` adds the determinism leg: every listed node must cross the band by itself and
        // finalise the SAME canonical hash — the runner restarts one node mid-halt so a lagging
        // `best` is part of the scenario.
        println!("== IRON #918: finality self-heal after entrenchment halt (direction b) ==");
        println!("rpc={} genesis={:?} spec={}", args.rpc, chain.genesis, chain.spec_version);
        let mut peers: Vec<Chain> = Vec::new();
        for u in args.peers.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            peers.push(Chain::connect(u).await);
        }
        let mut amt: u128 = 4_000;
        for cycle in 1..=2u32 {
            // ── entrench //Dave over 1/3 and hold until the guard halts. Seeds double each epoch
            // while the counter is still 0 (cycle 2 must out-grow everything the heal seeded).
            println!("\n[#918 c{cycle}] entrench //Dave over 1/3 → guard must halt");
            let mut halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
            let mut halted_ok = false;
            for _ in 0..(REQUIRED_EPOCHS + 10) {
                if halted {
                    halted_ok = true;
                    break;
                }
                let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
                if counter == 0 {
                    for call in seed_evidence_calls(&dave, amt) {
                        chain.submit_as(&alice, call).await;
                    }
                    amt = amt.saturating_mul(2);
                }
                let e = wait_epoch_change(&chain, args.epoch_secs).await;
                halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
                let share = chain.max_share().await;
                println!("  epoch={e} share={share:.4} counter={counter} halted={halted}");
            }
            r.check(&format!("918.c{cycle}.halts"), halted_ok, format!("halted={halted}"));

            // ── finality must plateau while production continues (in-flight pre-halt heights may
            // still finalise — never retroactively un-finalised, Art. IX — so poll to the plateau).
            let mut plateau = chain.finalized_number().await;
            let mut stalled = false;
            for _ in 0..8 {
                wait_epoch_change(&chain, args.epoch_secs).await;
                let now = chain.finalized_number().await;
                if now == plateau {
                    stalled = true;
                    break;
                }
                plateau = now;
            }
            let b0 = chain.best_number().await;
            wait_epoch_change(&chain, args.epoch_secs).await;
            let still = chain.finalized_number().await;
            r.check(
                &format!("918.c{cycle}.finality-plateaued"),
                stalled && still == plateau,
                format!("plateau={plateau} still={still}"),
            );
            let b1 = chain.best_number().await;
            r.check(&format!("918.c{cycle}.production-continues"), b1 > b0, format!("best {b0} -> {b1}"));

            // ── dilute: the other founders out-earn Dave → guard self-heals.
            println!("[#918 c{cycle}] dilute → self-heal → finality must resume past the band");
            let mut healed = false;
            for _ in 0..8u32 {
                for a in &others {
                    for call in seed_evidence_calls(a, amt) {
                        chain.submit_as(&alice, call).await;
                    }
                }
                amt = amt.saturating_mul(2);
                let e = wait_epoch_change(&chain, args.epoch_secs).await;
                let h = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
                let c = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
                println!("  epoch={e} halted={h} counter={c}");
                if !h && c == 0 {
                    healed = true;
                    break;
                }
            }
            r.check(&format!("918.c{cycle}.self-heals"), healed, "halted=false counter=0".into());

            // ── THE check that failed on: finalized must CROSS the frozen band. The skip
            // only accepts a healthy boundary buried RECOVERY_BURIAL under best, so poll wide.
            let mut f_after = chain.finalized_number().await;
            for _ in 0..10 {
                wait_epoch_change(&chain, args.epoch_secs).await;
                f_after = chain.finalized_number().await;
                if f_after > plateau {
                    break;
                }
            }
            r.check(
                &format!("918.c{cycle}.finality-resumed"),
                f_after > plateau,
                format!("finalized {plateau} -> {f_after}"),
            );

            // ── determinism: every peer crosses the band on its own and finalises the SAME canonical
            // hash right past the plateau — the band all of them transitively finalised is identical.
            if !peers.is_empty() {
                let probe = plateau + 1;
                let want: Option<H256> =
                    rpc(&chain.c, "chain_getBlockHash", rpc_params![probe]).await;
                for (i, p) in peers.iter().enumerate() {
                    let mut fp = p.finalized_number().await;
                    for _ in 0..10 {
                        if fp > plateau {
                            break;
                        }
                        wait_epoch_change(&chain, args.epoch_secs).await;
                        fp = p.finalized_number().await;
                    }
                    r.check(&format!("918.c{cycle}.peer{i}-resumed"), fp > plateau, format!("peer finalized {fp}"));
                    let got: Option<H256> = rpc(&p.c, "chain_getBlockHash", rpc_params![probe]).await;
                    r.check(
                        &format!("918.c{cycle}.peer{i}-same-band"),
                        want.is_some() && got == want,
                        format!("hash@{probe} {got:?}"),
                    );
                }
            }
        }
        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
    }

    if args.mode == "ceremony" || args.mode == "apply" {
        // UPGRADE BOUNDARY for the gate v2 scenarios (the author). G1, G3 and G4 test how a node
        // behaves ACROSS a runtime upgrade; without a real boundary in the lab they are theatre. This
        // performs the real ceremony — propose, approve to threshold, wait out the objection window,
        // apply — using the same extrinsics the live chain used, no test hooks.
        let code = std::fs::read(&args.wasm).unwrap_or_else(|e| panic!("--wasm: {e}"));
        let code_hash = sp_core::blake2_256(&code);
        println!("== CEREMONY (mode={}) ==", args.mode);
        println!("rpc={} spec={} blob={} bytes hash=0x{}", args.rpc, chain.spec_version, code.len(), hex_str(&code_hash));

        if args.mode == "ceremony" {
            // The declaration must equal the chain's sealed truth byte for byte, or propose fails closed.
            let manifesto_hash: [u8; 32] = chain
                .storage("Manifesto", "ManifestoHash")
                .await
                .map(|b| <[u8; 32]>::decode(&mut &b[..]).expect("manifesto hash"))
                .expect("no sealed manifesto on this chain — nothing can be upgraded");
            let declared = pallet_multisig_upgrade::InvariantDeclaration {
                manifesto_hash,
                cap_hlq: args.cap_hlq,
                cap_sov: args.cap_sov,
                entrenchment_threshold_fp: pallet_reputation::pallet::ENTRENCHMENT_THRESHOLD_FP,
                entrenchment_required_epochs: pallet_reputation::pallet::ENTRENCHMENT_REQUIRED_EPOCHS,
                preserves_validator: true,
            };
            // Dev seats: Alice..Eve, Eve is CUSTODY (may not propose or approve) — so Alice proposes and
            // Bob + Charlie approve, reaching the 3-of-5 upgrade threshold.
            let alice = sr25519::Pair::from_string("//Alice", None).unwrap();
            let bob = sr25519::Pair::from_string("//Bob", None).unwrap();
            let charlie = sr25519::Pair::from_string("//Charlie", None).unwrap();
            chain
                .submit_as(
                    &alice,
                    RuntimeCall::MultisigUpgrade(pallet_multisig_upgrade::Call::propose_upgrade {
                        code_hash,
                        declared,
                    }),
                )
                .await;
            wait_blocks(&chain, 4).await;
            for (who, name) in [(&bob, "Bob"), (&charlie, "Charlie")] {
                chain
                    .submit_as(
                        who,
                        RuntimeCall::MultisigUpgrade(pallet_multisig_upgrade::Call::approve_upgrade {
                            code_hash,
                        }),
                    )
                    .await;
                println!("approval submitted by {name}");
                wait_blocks(&chain, 4).await;
            }
            match chain.storage("MultisigUpgrade", "PendingUpgrade").await {
                Some(_) => println!("PENDING recorded on chain — proposal + approvals landed"),
                None => println!("NO PENDING UPGRADE — the proposal did not land, check the declaration"),
            }
            println!("now at block {}", head_number_of(&chain).await);
            println!("apply once the objection window has elapsed: --mode apply --wasm <same blob>");
        } else {
            // Permissionless and feeless: anyone may apply once the window has elapsed.
            let anyone = sr25519::Pair::from_string("//Dave", None).unwrap();
            chain
                .submit_as(
                    &anyone,
                    RuntimeCall::MultisigUpgrade(pallet_multisig_upgrade::Call::apply_upgrade { code }),
                )
                .await;
            wait_blocks(&chain, 6).await;
            let after = Chain::connect(&args.rpc).await;
            println!("runtime spec after apply = {}", after.spec_version);
            r.check(
                "apply.spec-bumped",
                after.spec_version > chain.spec_version,
                format!("{} -> {}", chain.spec_version, after.spec_version),
            );
            println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
            std::process::exit(if r.fail == 0 { 0 } else { 1 });
        }
        return;
    }

    if args.mode == "journey" {
        // THE WHOLE ROAD, ONE MASK (the reviewer). The earlier evidence used TWO different
        // strangers — one bound a node, another took a name and spoke. That is not what was promised:
        // the promise is that ONE person arriving with nothing can walk the entire path. Four acts:
        // take a name, bind your node so you can ever be paid, speak, and offer something.
        println!("== THE STRANGER'S WHOLE ROAD — one mask, four acts ==");
        let mask = sr25519::Pair::from_seed(&[0xc3; 32]);
        let acc: AccountId32 = mask.public().into();
        let hot = sr25519::Pair::from_seed(&[0xc4; 32]);
        println!("mask = {}  (node key 0x{})", acc.to_ss58check(), hex_str(&hot.public().0));
        let bal = chain.balance_hlq(&mask.public().0).await;
        r.check("journey.starts-broke", bal == 0, format!("HLQ = {bal}"));

        const BIND_LABEL: &[u8] = b"hlq-node-bind-v1";
        let mut msg = BIND_LABEL.to_vec();
        msg.extend_from_slice(acc.as_ref());
        let pop = hot.sign(&msg).0;
        let body = sp_core::blake2_256(b"harlequin: a stranger says hello");
        let detail = sp_core::blake2_256(b"harlequin: a stranger offers something");

        let acts: Vec<(&str, RuntimeCall)> = vec![
            ("1. take a name", RuntimeCall::Directory(pallet_directory::Call::register {})),
            (
                "2. bind own node",
                RuntimeCall::Reputation(pallet_reputation::Call::set_vote_key {
                    sk_pub: hot.public().0,
                    pop_sig: pop,
                }),
            ),
            ("3. speak", RuntimeCall::Forum(pallet_forum::Call::post { body, parent: None })),
            (
                "4. offer",
                RuntimeCall::Market(pallet_market::Call::publish {
                    detail,
                    category: b"general".to_vec().try_into().expect("fits"),
                }),
            ),
        ];
        let mut done = 0;
        for (label, call) in acts {
            wait_blocks(&chain, 3).await; // the feeless lane also has a per-BLOCK ceiling
            match chain.submit_as_try(&mask, call).await {
                Ok(_) => { println!("  {label}: ACCEPTED"); done += 1; }
                Err(e) => {
                    let why = if e.contains("Inability to pay") { "no budget left / cannot pay" } else { "refused" };
                    println!("  {label}: DENIED ({why})");
                }
            }
        }
        wait_blocks(&chain, 4).await;

        // Verdict from chain state: does THIS mask hold a name AND its node binding?
        let mut kh = twox_128("Directory".as_bytes()).to_vec();
        kh.extend(twox_128("HandleOf".as_bytes()));
        kh.extend(sp_core::blake2_128(acc.as_ref()));
        kh.extend_from_slice(acc.as_ref());
        let handle: Option<String> =
            rpc(&chain.c, "state_getStorage", rpc_params![format!("0x{}", hex_str(&kh))]).await;
        let owner = chain.vote_key_owner(&hot.public().0).await;
        r.check("journey.has-name", handle.is_some(), format!("Directory::HandleOf = {handle:?}"));
        r.check(
            "journey.node-bound-to-SAME-mask",
            owner.as_ref() == Some(&acc),
            format!("VoteKeyOwner = {owner:?}"),
        );
        r.check("journey.all-four-in-one-day", done == 4, format!("{done}/4 acts accepted on day one"));
        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
    }

    if args.mode == "pay" {
        // One transfer, measured alone. The society run mixes fees and a payment in the same window, so
        // a balance delta there cannot say whether the payment landed — this isolates it.
        let a = sr25519::Pair::from_string("//Dave", None).unwrap();
        let b = sr25519::Pair::from_string("//Ferdie", None).unwrap();
        let a_acc: AccountId32 = a.public().into();
        let before = chain.balance_hlq(&a.public().0).await;
        let amount = 7_000_000_000u128; // distinctive figure: unmistakable in the delta
        let res = chain
            .submit_as_try(
                &b,
                RuntimeCall::Tokens(pallet_tokens::Call::transfer {
                    coin: pallet_tokens::Coin::Hlq,
                    to: a_acc,
                    amount,
                }),
            )
            .await;
        println!("submit: {res:?}");
        wait_blocks(&chain, 5).await;
        let after = chain.balance_hlq(&a.public().0).await;
        println!("receiver {before} -> {after} (delta {})", after as i128 - before as i128);
        r.check(
            "pay.landed",
            after == before + amount,
            format!("expected +{amount}, got {}", after as i128 - before as i128),
        );
        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
    }

    if args.mode == "society" {
        // G7 — THE TWO STRANGERS (the reviewer). The forum, the market and the directory are all
        // LIVE in the runtime and have never been used once — 0 masks, 0 posts, 0 offers in 50k blocks
        // of the real chain. This walks the whole citizen journey twice: first with masks that arrive
        // broke (which is who the project is for), then with funded ones, so the verdict separates
        // "the mechanism is broken" from "the mechanism works but the door charges".
        println!("== G7: two strangers — register, speak, reply, offer, pay ==");
        println!("rpc={} spec={}", args.rpc, chain.spec_version);

        // Society counters, read straight from chain state — the verdict is state, not logs.
        let (m0, p0, o0) = society_counts(&chain).await;
        println!("BEFORE  masks={m0} posts={p0} offers={o0}");

        // ---- PART A: the strangers as they really arrive — with nothing ----
        let a_broke = sr25519::Pair::from_seed(&[0xb1; 32]);
        let b_broke = sr25519::Pair::from_seed(&[0xb2; 32]);
        let steps: Vec<(&str, RuntimeCall)> = alloc_steps(&b_broke);
        println!("\n-- part A: broke masks (0 HLQ) --");
        let mut denied: Vec<&str> = Vec::new();
        for (label, call) in steps {
            // Space the attempts out: the feeless lane has a per-BLOCK weight ceiling as well as a
            // per-mask budget, so three calls crammed into one block can be refused by the ceiling even
            // when the mask has budget left. Testing them back-to-back would blame the wrong limit.
            wait_blocks(&chain, 3).await;
            match chain.submit_as_try(&a_broke, call).await {
                Ok(_) => println!("  {label}: ACCEPTED"),
                Err(e) => {
                    let short = if e.contains("Inability to pay") { "cannot pay the fee" } else { "refused" };
                    println!("  {label}: DENIED ({short})");
                    denied.push(label);
                }
            }
        }
        r.check(
            "society.broke-can-live",
            denied.is_empty(),
            format!("denied to a penniless mask: {denied:?}"),
        );

        // ---- PART B: same journey, funded masks — does the machinery work AT ALL? ----
        println!("\n-- part B: funded masks (dev founders) --");
        let a = sr25519::Pair::from_string("//Dave", None).unwrap();
        let b = sr25519::Pair::from_string("//Ferdie", None).unwrap();
        let a_acc: AccountId32 = a.public().into();
        let bal_a_before = chain.balance_hlq(&a.public().0).await;

        for (who, name) in [(&a, "A"), (&b, "B")] {
            match chain.submit_as_try(who, RuntimeCall::Directory(pallet_directory::Call::register {})).await {
                Ok(_) => println!("  {name} register: submitted"),
                Err(e) => println!("  {name} register: REFUSED {e}"),
            }
            wait_blocks(&chain, 3).await;
        }

        let body_a = sp_core::blake2_256(b"harlequin g7: first words in the forum");
        let _ = chain.submit_as_try(&a, RuntimeCall::Forum(pallet_forum::Call::post { body: body_a, parent: None })).await;
        wait_blocks(&chain, 3).await;
        let posts_now = chain.u64_storage("Forum", "NextId").await;
        // The reply hangs off the post just made (ids are sequential from 0).
        let parent_id = posts_now.saturating_sub(1);
        let body_b = sp_core::blake2_256(b"harlequin g7: a stranger answers");
        let _ = chain
            .submit_as_try(&b, RuntimeCall::Forum(pallet_forum::Call::post { body: body_b, parent: Some(parent_id) }))
            .await;
        wait_blocks(&chain, 3).await;

        let detail = sp_core::blake2_256(b"harlequin g7: something for sale");
        let category: Vec<u8> = b"general".to_vec();
        let _ = chain
            .submit_as_try(
                &a,
                RuntimeCall::Market(pallet_market::Call::publish {
                    detail,
                    category: category.try_into().expect("category fits"),
                }),
            )
            .await;
        wait_blocks(&chain, 3).await;

        // STEP 5 — HONEST LABEL: there is no buy/accept/close/escrow in the market pallet (only publish
        // and withdraw). The closest thing to "closing a deal" is a bare transfer with NO link to the
        // offer: a BLIND PAYMENT. Reported as such, never as a completed trade.
        let _ = chain
            .submit_as_try(
                &b,
                RuntimeCall::Tokens(pallet_tokens::Call::transfer {
                    coin: pallet_tokens::Coin::Hlq,
                    to: a_acc.clone(),
                    amount: 1_000_000_000u128,
                }),
            )
            .await;
        wait_blocks(&chain, 4).await;

        let (masks, posts, offers) = society_counts(&chain).await;
        let bal_a_after = chain.balance_hlq(&a.public().0).await;
        println!("\nAFTER   masks={masks} posts={posts} offers={offers}");
        println!("A balance {bal_a_before} -> {bal_a_after}");

        r.check("society.masks-registered", masks >= 2, format!("MaskCount={masks}"));
        r.check("society.thread-exists", posts >= 2, format!("Forum posts={posts}"));
        r.check("society.offer-published", offers >= 1, format!("Market offers={offers}"));
        r.check(
            "society.payment-landed",
            bal_a_after > bal_a_before,
            format!("blind payment, unlinked to the offer: {bal_a_before} -> {bal_a_after}"),
        );

        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
    }

    if args.mode == "newcomer" {
        // G6 — THE STRANGER'S JOURNEY (the author). Every iron run so far started from a
        // founder mask: already seeded, already bound, already funded. Nobody had ever walked the road
        // of someone arriving with empty hands, which is exactly where the door turned out to be shut.
        //
        // The scenario: a mask born one second ago (zero HLQ, zero reputation, known to no one) tries
        // to register its own node with a VALID possession proof. The only question asked here is
        // whether the chain lets it in.
        println!("== G6: newcomer journey (fresh mask, zero funds, registers its own node) ==");
        println!("rpc={} genesis={:?} spec={}", args.rpc, chain.genesis, chain.spec_version);
        const BIND_LABEL: &[u8] = b"hlq-node-bind-v1";

        // Deterministic "random" newcomer: a fixed seed keeps the run reproducible, and the account is
        // one no genesis ever heard of.
        let newcomer = sr25519::Pair::from_seed(&[0xa7; 32]);
        let newcomer_acc: AccountId32 = newcomer.public().into();
        let hot = sr25519::Pair::from_seed(&[0xa8; 32]);
        let hot_pub = hot.public().0;
        println!("newcomer mask = {}", newcomer_acc.to_ss58check());
        println!("newcomer node key = 0x{}", hex_str(&hot_pub));

        // It arrives with nothing — assert that, so a funded account can never fake a pass here.
        let bal = chain.balance_hlq(&newcomer.public().0).await;
        r.check("newcomer.starts-broke", bal == 0, format!("HLQ balance = {bal}"));

        // A correct proof of possession: the node key signs BIND_LABEL ‖ the mask's account.
        let mut msg = BIND_LABEL.to_vec();
        msg.extend_from_slice(newcomer_acc.as_ref());
        let pop = hot.sign(&msg).0;
        let call = RuntimeCall::Reputation(pallet_reputation::Call::set_vote_key {
            sk_pub: hot_pub,
            pop_sig: pop,
        });

        match chain.submit_as_try(&newcomer, call).await {
            Ok(h) => {
                println!("submitted: {h:?}");
                wait_epoch_change(&chain, args.epoch_secs).await;
                let owner = chain.vote_key_owner(&hot_pub).await;
                r.check(
                    "newcomer.can-register",
                    owner.as_ref() == Some(&newcomer_acc),
                    format!("VoteKeyOwner[node] = {owner:?}"),
                );
            }
            Err(e) => {
                // The refusal IS the finding. Name it plainly rather than burying it in a stack trace.
                println!("REFUSED at submission: {e}");
                let paywall = e.contains("Payment")
                    || e.contains("payment")
                    || e.contains("Inability to pay");
                r.check(
                    "newcomer.can-register",
                    false,
                    format!("door shut before the chain even saw it; paywall={paywall}: {e}"),
                );
            }
        }

        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
    }

    if args.mode == "bind" {
        // #837 iron: real set_vote_key on live iron, with AccountId32 (the 48-byte PoP layout) — the
        // path a citizen node takes to link itself to its mask. Exercises PoP verify, uniqueness,
        // rotation, foreign-key rejection, and clear, reading the on-chain maps after each step.
        println!("== IRON #837: node↔mask binding (set_vote_key + PoP) ==");
        println!("rpc={} genesis={:?} spec={}", args.rpc, chain.genesis, chain.spec_version);
        const BIND_LABEL: &[u8] = b"hlq-node-bind-v1";
        let pop_msg = |acc: &AccountId32| -> Vec<u8> {
            let mut m = BIND_LABEL.to_vec();
            m.extend_from_slice(acc.as_ref()); // AccountId32 = 32 raw bytes (the frozen 48B layout)
            m
        };
        let bind_call = |sk_pub: [u8; 32], pop_sig: [u8; 64]| -> RuntimeCall {
            RuntimeCall::Reputation(pallet_reputation::Call::set_vote_key { sk_pub, pop_sig })
        };
        let alice = sr25519::Pair::from_string("//Alice", None).unwrap();
        let bob = sr25519::Pair::from_string("//Bob", None).unwrap();
        let alice_acc: AccountId32 = alice.public().into();
        let bob_acc: AccountId32 = bob.public().into();
        // Triangulation vector (diff against the reviewer's substrate-interface + wallet-core): the 48-byte
        // PoP message for //Alice must be BIND_LABEL ‖ acc32, byte-for-byte identical everywhere.
        println!("POP-VECTOR //Alice acc32={}", hex_str(alice_acc.as_ref()));
        println!("POP-VECTOR //Alice msg48={}", hex_str(&pop_msg(&alice_acc)));
        // Two node hot keys (session keys), distinct from any founder's.
        let hot_a = sr25519::Pair::from_seed(&[0x51; 32]);
        let hot_b = sr25519::Pair::from_seed(&[0x52; 32]);
        let (ska, skb) = (hot_a.public().0, hot_b.public().0);

        // 1) Valid bind: Alice's mask signs the tx; hot_a proves possession over label‖alice_acc.
        let pop_a = hot_a.sign(&pop_msg(&alice_acc)).0;
        chain.submit_as(&alice, bind_call(ska, pop_a)).await;
        wait_epoch_change(&chain, args.epoch_secs).await;
        let vk = chain.vote_key_of(&alice_acc).await;
        let owner = chain.vote_key_owner(&ska).await;
        r.check("bind.stored", vk == Some(ska), format!("VoteKeys[Alice]={:?}", vk.map(|b| hex_str(&b))));
        r.check("bind.reverse", owner.as_ref() == Some(&alice_acc), format!("owner={owner:?}"));

        // 2) Bad PoP is refused: Bob tries to bind a fresh key with a garbage proof → not stored.
        let hot_c = sr25519::Pair::from_seed(&[0x53; 32]);
        chain.submit_as(&bob, bind_call(hot_c.public().0, [0u8; 64])).await;
        wait_epoch_change(&chain, args.epoch_secs).await;
        r.check(
            "bind.bad-pop-rejected",
            chain.vote_key_of(&bob_acc).await.is_none(),
            "garbage PoP left no binding".into(),
        );

        // 3) Foreign key taken: Bob HOLDS hot_a (signs a valid proof for his own account) but ska is
        //    already Alice's → SessionKeyInUse, Bob gets no binding.
        let pop_a_for_bob = hot_a.sign(&pop_msg(&bob_acc)).0;
        chain.submit_as(&bob, bind_call(ska, pop_a_for_bob)).await;
        wait_epoch_change(&chain, args.epoch_secs).await;
        r.check(
            "bind.foreign-key-rejected",
            chain.vote_key_of(&bob_acc).await.is_none() && chain.vote_key_owner(&ska).await.as_ref() == Some(&alice_acc),
            "ska still Alice's; Bob unbound".into(),
        );

        // 4) Rotation: Alice binds hot_b → old ska freed in the reverse map, skb now hers.
        let pop_b = hot_b.sign(&pop_msg(&alice_acc)).0;
        chain.submit_as(&alice, bind_call(skb, pop_b)).await;
        wait_epoch_change(&chain, args.epoch_secs).await;
        r.check(
            "bind.rotation",
            chain.vote_key_of(&alice_acc).await == Some(skb)
                && chain.vote_key_owner(&ska).await.is_none()
                && chain.vote_key_owner(&skb).await.as_ref() == Some(&alice_acc),
            "skb bound, ska released".into(),
        );

        // 5) Freed key reusable: Bob now binds the released ska (he holds hot_a).
        let pop_a_bob2 = hot_a.sign(&pop_msg(&bob_acc)).0;
        chain.submit_as(&bob, bind_call(ska, pop_a_bob2)).await;
        wait_epoch_change(&chain, args.epoch_secs).await;
        r.check(
            "bind.freed-key-reusable",
            chain.vote_key_owner(&ska).await.as_ref() == Some(&bob_acc),
            "ska now Bob's".into(),
        );

        // 6) Clear: Alice unbinds → both maps clear for her key.
        chain
            .submit_as(&alice, RuntimeCall::Reputation(pallet_reputation::Call::clear_vote_key {}))
            .await;
        wait_epoch_change(&chain, args.epoch_secs).await;
        r.check(
            "bind.clear",
            chain.vote_key_of(&alice_acc).await.is_none() && chain.vote_key_owner(&skb).await.is_none(),
            "Alice unbound, skb freed".into(),
        );

        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
    }

    if args.mode == "premeasure" {
        // #599 activation gate: READ-ONLY pre-measurement of the LIVE chain's cluster shares — the
        // exact math the wired guard will run, computed offline from public state. No extrinsics.
        // Run against a local tunnel to a live node's RPC (ssh -L 9944:127.0.0.1:9944 <node>).
        println!("== PREMEASURE #599 (read-only): per-suit cluster shares of consensus reputation ==");
        println!("rpc={} genesis={:?} spec={}", args.rpc, chain.genesis, chain.spec_version);
        let raw: String = rpc(
            &chain.c,
            "state_call",
            rpc_params!["HarlequinConsensusApi_consensus_reputation", "0x"],
        )
        .await;
        let bytes = hex_bytes(&raw);
        let reps_vec = <Vec<([u8; 32], i128)>>::decode(&mut &bytes[..]).expect("reps decode");
        let hexk = |a: &[u8; 32]| a.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let reps: std::collections::BTreeMap<String, i128> =
            reps_vec.iter().map(|(a, r)| (hexk(a), *r)).collect();
        let nodes: Vec<String> = reps.keys().cloned().collect();
        // Walk the whole Vouches map (state_getKeysPaged over its prefix; devnet/mainnet sets are tiny).
        let mut prefix = twox_128("Reputation".as_bytes()).to_vec();
        prefix.extend(twox_128("Vouches".as_bytes()));
        let prefix_hex = format!("0x{}", hex_str(&prefix));
        let keys: Vec<String> = rpc(
            &chain.c,
            "state_getKeysPaged",
            rpc_params![prefix_hex.clone(), 1000u32, Option::<String>::None],
        )
        .await;
        let mut graph = reputation_core::TrustGraph::new();
        let mut edge_count = 0u32;
        for k in &keys {
            let kb = hex_bytes(k);
            // key layout: prefix(32) + blake2_128(acc)(16) + acc(32)
            let acc: [u8; 32] = kb[kb.len() - 32..].try_into().unwrap();
            let val: Option<String> = rpc(&chain.c, "state_getStorage", rpc_params![k.clone()]).await;
            let Some(val) = val else { continue };
            let vb = hex_bytes(&val);
            let edges = <Vec<([u8; 32], Suit, u32)>>::decode(&mut &vb[..]).expect("vouches decode");
            for (target, suit, weight) in edges {
                graph.attest(&hexk(&acc), &hexk(&target), suit.dim_name(), weight as f64);
                edge_count += 1;
            }
        }
        println!("participants={} vouch_edges={}", nodes.len(), edge_count);
        const FP: i128 = 1_000_000_000; // reputation_core::FP_SCALE
        let single = {
            let v: Vec<i128> = reps.values().copied().collect();
            reputation_core::max_single_entity_share_fp(&v)
        };
        let mut worst: i128 = 0;
        let mut real_cluster = false;
        for suit in SUITS {
            let labels = graph.communities(suit.dim_name(), &nodes);
            let share = reputation_core::max_cluster_share_fp(&labels, &reps);
            let mut sizes: std::collections::BTreeMap<&String, u32> = Default::default();
            let mut biggest = 0u32;
            for (node, label) in labels.iter() {
                if reps.get(node).is_some_and(|r| *r > 0) {
                    let n = sizes.entry(label).or_insert(0);
                    *n += 1;
                    biggest = biggest.max(*n);
                }
            }
            if biggest >= 2 {
                real_cluster = true;
            }
            println!(
                "suit={:<12} cluster_share_fp={share} ({:.2}%) biggest_community={biggest}",
                suit.dim_name(),
                share as f64 * 100.0 / FP as f64
            );
            worst = worst.max(share);
        }
        let would_arm = real_cluster && worst > 0 && worst * 3 < FP;
        println!(
            "single_entity_fp={single} ({:.2}%)  worst_cluster_fp={worst} ({:.2}%)  real_cluster={real_cluster}",
            single as f64 * 100.0 / FP as f64,
            worst as f64 * 100.0 / FP as f64
        );
        println!(
            "VERDICT: on upgrade the leg would {} (arming needs a ≥2 community AND worst < 1/3); effective share while unarmed = single-entity only",
            if would_arm { "ARM immediately" } else { "stay OBSERVE-ONLY" }
        );
        return;
    }

    if args.mode == "cluster" {
        // #599 iron v4 (four-eyes R5): FOUNDERS-ONLY scenarios — every reputation-bearing account is
        // a VOTING devnet founder, because the committee fallback seats any rep>0 account with no
        // vote-key filter (sortition_fp.rs): fabricated non-voting rep >= 1/3 weight stalls finality
        // (that killed pass 1 at B'2). One persistent 3-ring D-E-F drives all phases; A-B-C dilute.
        // Arithmetic (the reviewer): ring<1/3 with 3 equal free accounts keeps every single <1/3 always.
        // A' late ring must never arm or halt; B' dilution arms, ring re-boost halts, re-dilution
        // self-heals; C' = the runner executes this twice (plain / --shuffle) on fresh chains and
        // diffs ROUNDS then STATE — keying must be order-independent.
        println!("== IRON TEST #599 v4: cluster-share guard (Art. VI anti-split), founders-only ==");
        println!(
            "rpc={} genesis={:?} spec={} shuffle={}",
            args.rpc, chain.genesis, chain.spec_version, args.shuffle
        );
        let founders = ["//Alice", "//Bob", "//Charlie", "//Dave", "//Eve", "//Ferdie"]
            .map(|s| sr25519::Pair::from_string(s, None).unwrap());
        let acc = |p: &sr25519::Pair| -> AccountId32 { p.public().into() };
        let ring3: Vec<&sr25519::Pair> = vec![&founders[3], &founders[4], &founders[5]]; // D,E,F
        let free3: Vec<&sr25519::Pair> = vec![&founders[0], &founders[1], &founders[2]]; // A,B,C
        // --shuffle reverses every batch: same set, different submission order.
        fn ordered<T>(mut v: Vec<T>, shuffle: bool) -> Vec<T> {
            if shuffle {
                v.reverse();
            }
            v
        }
        let mut events: Vec<(i128, bool)> = Vec::new();
        const FP_SCALE: i128 = 1_000_000_000; // mirrors reputation_core::FP_SCALE

        // ── A'1: baseline — founders with genesis reputation, NO vouch edges anywhere.
        println!("\n[A'] baseline: rep without a ring must not arm");
        let e = wait_epoch_change(&chain, args.epoch_secs).await;
        let armed = chain.bool_storage("Reputation", "ClusterGuardArmed").await;
        let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
        r.check("A'.baseline-unarmed", !armed && counter == 0, format!("epoch={e} armed={armed} counter={counter}"));

        // ── A'2: the founders form a ring LATE (the R1 sequence) — observe-only, never a halt.
        // D-E-F at ~50% of six equal founders: over threshold, real cluster, must stay inert.
        println!("[A'] late 3-ring D-E-F (Commerce) held 8 epochs — must stay unarmed, zero counter");
        let cycle: Vec<(usize, usize)> = (0..3).map(|i| (i, (i + 1) % 3)).collect();
        for (i, j) in ordered(cycle, args.shuffle) {
            let target = acc(ring3[j]);
            chain.submit_as(ring3[i], vouch_call(&target)).await;
        }
        let f0 = chain.finalized_number().await;
        for _ in 0..8u32 {
            let e = wait_epoch_change(&chain, args.epoch_secs).await;
            let armed = chain.bool_storage("Reputation", "ClusterGuardArmed").await;
            let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
            let halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
            println!("  epoch={e} armed={armed} counter={counter} halted={halted}");
            if armed || counter != 0 || halted {
                break;
            }
        }
        let armed = chain.bool_storage("Reputation", "ClusterGuardArmed").await;
        let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
        let halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
        let f1 = chain.finalized_number().await;
        r.check(
            "A'.late-ring-inert",
            !armed && counter == 0 && !halted,
            format!("armed={armed} counter={counter} halted={halted} (R1: no false halt)"),
        );
        r.check("A'.finality-alive", f1 > f0, format!("finalized {f0} -> {f1}"));
        events.extend(chain.recent_cluster_events(12).await);

        // ── B'1: dilute the ring under 1/3 with the three FREE founders (voters — finality lives).
        // Auto-calibrated: while over threshold the guard emits its observed share every epoch.
        println!("\n[B'] dilute D-E-F under 1/3 seeding A+B+C equally (auto-calibrated) → must ARM");
        let mut ext_amount: u128 = 1_000;
        let mut ext_rounds: u32 = 0;
        let mut armed = false;
        for round in 0..10u32 {
            armed = chain.bool_storage("Reputation", "ClusterGuardArmed").await;
            if armed {
                break;
            }
            let observed = chain
                .recent_cluster_events(12)
                .await
                .last()
                .map(|(s, _)| *s)
                .unwrap_or(i128::MAX);
            if observed != i128::MAX && observed.saturating_mul(3) < FP_SCALE {
                println!("  round={round} observed_share_fp={observed} under threshold → waiting for arm");
            } else {
                println!("  round={round} observed_share_fp={observed} over threshold → seeding free founders {ext_amount}/suit");
                for p in ordered(free3.clone(), args.shuffle) {
                    for call in seed_evidence_calls(&acc(p), ext_amount) {
                        chain.submit_as(&alice, call).await;
                    }
                }
                ext_amount = ext_amount.saturating_mul(2);
                ext_rounds += 1;
            }
            wait_epoch_change(&chain, args.epoch_secs).await;
        }
        r.check("B'.diluted-ring-arms", armed, "real bounded cluster (D-E-F < 1/3) present".into());
        events.extend(chain.recent_cluster_events(12).await);

        // ── B'2: the armed ring RE-CONCENTRATES: boost D-E-F equally until the CLUSTER passes 1/3
        // (each member stays well under 1/3 — single-entity is blind; only the cluster leg counts) →
        // halt after REQUIRED_EPOCHS sustained. All voters keep voting, so finality lives UNTIL the
        // guard itself freezes it — which is the assert.
        println!("[B'] re-boost D-E-F (auto-calibrated) until ring > 1/3 — must HALT at {REQUIRED_EPOCHS}");
        let mut syb_amount: u128 = 2_000;
        let mut syb_rounds: u32 = 0;
        let mut halted_at: Option<u32> = None;
        for _ in 0..(REQUIRED_EPOCHS + 8) {
            let e = wait_epoch_change(&chain, args.epoch_secs).await;
            let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
            let halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
            println!("  epoch={e} counter={counter} halted={halted} round={syb_rounds} boost={syb_amount}");
            if halted {
                halted_at = Some(counter);
                break;
            }
            if counter == 0 {
                for p in ordered(ring3.clone(), args.shuffle) {
                    for call in seed_evidence_calls(&acc(p), syb_amount) {
                        chain.submit_as(&alice, call).await;
                    }
                }
                syb_amount = syb_amount.saturating_mul(2);
                syb_rounds += 1;
            }
        }
        r.check(
            "B'.ring-halts-after-required",
            halted_at.map(|c| c >= REQUIRED_EPOCHS).unwrap_or(false),
            format!("halted_at_counter={halted_at:?}"),
        );
        let bh = chain.best_number().await;
        wait_epoch_change(&chain, args.epoch_secs).await;
        let bh2 = chain.best_number().await;
        r.check("B'.production-continues", bh2 > bh, format!("best {bh} -> {bh2}"));
        events.extend(chain.recent_cluster_events(12).await);

        // ── B'3: dilution — the free founders out-earn the ring again → self-heal + one-way arm +
        // finality actually RESUMES (burial-K delay tolerated, same as the single-entity iron).
        println!("[B'] re-dilution (auto-escalated A+B+C) — must self-heal, STAY armed, finality resumes");
        let mut heal_amount: u128 = syb_amount;
        let mut heal_rounds: u32 = 0;
        let mut healed = false;
        for _ in 0..8u32 {
            heal_rounds += 1;
            for p in ordered(free3.clone(), args.shuffle) {
                for call in seed_evidence_calls(&acc(p), heal_amount) {
                    chain.submit_as(&alice, call).await;
                }
            }
            heal_amount = heal_amount.saturating_mul(2);
            let e = wait_epoch_change(&chain, args.epoch_secs).await;
            let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
            let halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
            println!("  epoch={e} counter={counter} halted={halted} round={heal_rounds} heal_amount={heal_amount}");
            if !halted && counter == 0 {
                healed = true;
                break;
            }
        }
        let armed = chain.bool_storage("Reputation", "ClusterGuardArmed").await;
        r.check("B'.self-heals", healed, "halted=false counter=0".into());
        r.check("B'.arm-is-one-way", armed, format!("armed={armed}"));
        let fr0 = chain.finalized_number().await;
        let mut fr1 = fr0;
        for _ in 0..6u32 {
            wait_epoch_change(&chain, args.epoch_secs).await;
            fr1 = chain.finalized_number().await;
            if fr1 > fr0 {
                break;
            }
        }
        r.check("B'.finality-resumed", fr1 > fr0, format!("finalized {fr0} -> {fr1}"));
        events.extend(chain.recent_cluster_events(12).await);

        // ── STATE line for C' (runner diffs ROUNDS first, then STATE; see the C' race protocol).
        events.sort();
        events.dedup();
        let counter = chain.u32_storage("Reputation", "EntrenchmentCounter").await;
        let halted = chain.bool_storage("Reputation", "EntrenchmentHalted").await;
        println!("\nROUNDS ext={ext_rounds} syb={syb_rounds} heal={heal_rounds}");
        println!("STATE armed={armed} counter={counter} halted={halted} events={events:?}");
        println!("\n== RESULT: {} PASS / {} FAIL ==", r.pass, r.fail);
        std::process::exit(if r.fail == 0 { 0 } else { 1 });
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
