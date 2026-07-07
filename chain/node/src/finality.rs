//! Woven-Trust finality gadget — the network layer that turns the validated Snowball voting core
//! (`consensus_core::FinalityRound`) into real block finality, replacing the trivial `finalize:true`.
//!
//! Shape (mirrors GRANDPA's split, with OUR logic): a `sc-network` notification protocol carries the
//! committee's finality votes; a worker future runs one [`consensus_core::FinalityRound`] per target
//! height, broadcasts this node's preferred block hash, feeds the votes it receives into the round, and
//! when the round decides calls `Finalizer::finalize_block` with a justification. The decision rule and
//! its safety/liveness are NOT here — they live (tested, 13/13 cross-validated) in `consensus-core`;
//! this file only wires votes ⇄ network ⇄ the chain. Block-import verification of the justification is
//! the next increment (see `INTEGRACION-NODO-WOVEN-TRUST.md` step 3).
//!
//! Committee & params scale to the live committee: each height's committee is elected by the same
//! reputation-weighted sortition the node authors with (`elect_committee_fp`, deterministic seed
//! `final{height}` so every node agrees), and the Snowball `k/alpha` scale to its size while keeping the
//! 0.70 quorum wall. `beta` here is a TESTNET value (fast finality for validation); production params
//! come from `PARAMETERS.md` at the pre-mainnet config gate (#25).

use crate::service::FullClient;
use futures::{FutureExt, StreamExt};
use harlequin_runtime::interface::OpaqueBlock as Block;
use polkadot_sdk::{
    sc_client_api::{BlockBackend, Finalizer},
    sc_consensus::{
        import_queue::{BasicQueue, BoxBlockImport, BoxJustificationImport, Verifier},
        BlockCheckParams, BlockImport, BlockImportParams, ForkChoiceStrategy, ImportResult,
        JustificationImport,
    },
    sc_network::{self, NetworkBackend, NotificationService},
    sc_network_gossip::{
        self, GossipEngine, MessageIntent, ValidationResult, Validator, ValidatorContext,
    },
    sc_network_types::PeerId,
    sp_blockchain::HeaderBackend,
    sp_consensus::Error as ConsensusError,
    substrate_prometheus_endpoint::Registry,
};
use sp_api::ProvideRuntimeApi;
use sp_core::{sr25519, Pair, H256};
use sp_runtime::{
    traits::{Block as BlockT, Header as HeaderT, NumberFor},
    Justification,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

type Hash = <Block as BlockT>::Hash;

/// Notification protocol name. Versioned so a future wire change is a new protocol, not a silent break.
pub const PROTOCOL_NAME: &str = "/harlequin/snowball-finality/1";

/// Consensus engine id stamped on the justifications this gadget produces.
pub const ENGINE_ID: sp_runtime::ConsensusEngineId = *b"WTC1";

// --- Production cadence (#30, network-modelled) vs fast testnet values. Selected by the
// --- `mainnet` cargo feature. Mainnet: β=12, STEP=10 (≈60s), τ=60, epoch=600 (≈1h committee rotation).
/// Confirmations before a height is irreversible. β=12 in production (`PARAMETERS.md` ♥ "the twelve":
/// `Q1^12` is astronomically safe); a small fast value on testnet.
#[cfg(feature = "mainnet")]
const FINALITY_BETA: u32 = 12;
#[cfg(not(feature = "mainnet"))]
const FINALITY_BETA: u32 = 4;
/// How far ahead of the last finalised block the worker aims each finalisation. Finalising a block
/// finalises ALL its ancestors, so targeting `finalized + STEP` finalises a whole range at once — keeping
/// finality from lagging block production unboundedly. Must be ≥ the blocks produced during one
/// finalisation (~`beta`). Production = 10 blocks (≈60s at 6s/block, a clean 1-minute cadence; lag bounded
/// to ~`STEP`+`beta`, time-to-finality ~1–2 min).
// Production STEP=15: with single-leader authoring (1 block / 6s ≈ 10/min) a finalisation takes
// ~beta·round ≈ 72s, during which ~12 blocks are produced — so STEP must exceed 12 to keep finality
// AHEAD of production (15 → ~12.5/min finalised > ~10/min produced). The gap then stabilises ~STEP
// behind the tip instead of growing without bound (validated: STEP=10 lagged ~10/min vs 8.3/min).
#[cfg(feature = "mainnet")]
const FINALITY_STEP: u32 = 15;
#[cfg(not(feature = "mainnet"))]
const FINALITY_STEP: u32 = 8;
/// Self-heal recovery burial (iron test, the author four-eyes Q1 + the reviewer sizing): the recovery
/// anchor `H_r` is read at best (non-final, reorg-able). Re-anchoring the finalization of a halt band to an
/// `H_r` that could still reorg to a version with a different `cleared_at` would be a safety break. So
/// `H_r` is only used once it is buried by ≥ this many blocks OVER it (best_number − H_r ≥ RECOVERY_BURIAL):
/// an attacker who wants to reorg `hash(H_r)` must out-run the honest chain over exactly those blocks.
/// This path fires right after someone held > 1/3, so the context is ADVERSARIAL, not benign — MAINNET
/// uses 32 (≈2× step): with the share provably diluted < 1/3 at H_r, reversion probability ≈ (1/2)^32 ≈
/// 1e-9, a cryptographic margin. The cost is only a few blocks' extra delay before finality crosses the
/// band — cheap against a finality split. TESTNET uses 15 so the devnet self-heal test validates quickly.
#[cfg(feature = "mainnet")]
const RECOVERY_BURIAL: u32 = 32;
#[cfg(not(feature = "mainnet"))]
const RECOVERY_BURIAL: u32 = 15;
/// Under-quorum observability (F3 wedge): first WARN after this many consecutive round
/// ticks below the 60% vote quorum at one height, then repeat every `QUORUM_WARN_EVERY_TICKS`. The
/// stall itself is deliberate (safety over liveness) — these only make it loud and diagnosable
/// (votes seen / committee expected), never change behaviour.
const QUORUM_WARN_AFTER_TICKS: u32 = 10;
const QUORUM_WARN_EVERY_TICKS: u32 = 60;
/// Expected committee seats elected from reputation (sortition `tau`). Production τ=60 (`PARAMETERS.md` ♠
/// the rotating jury; = 3·k, honest supermajority w.h.p. below the threshold — the liveness knob); a small
/// value over the dev cohort on testnet.
#[cfg(feature = "mainnet")]
const COMMITTEE_TAU: u32 = 60;
#[cfg(not(feature = "mainnet"))]
const COMMITTEE_TAU: u32 = 4;
/// Blocks per finality epoch — the committee is re-drawn each epoch (Art. VI — no one keeps power), seed
/// bound to the epoch number so the jury rotates. Production = 300 blocks ≈ **1 hour** at 12s/block (D1 12s re-derivation) (aura
/// 60↔60: 60 seats re-drawn every 60 minutes; well above time-to-finality ~1–2 min and well below the
/// daily reputation recompute = 24 committees/day over the same reputation, fresh beacon). Fast on testnet.
#[cfg(feature = "mainnet")]
const EPOCH_LENGTH: u32 = 300;
#[cfg(not(feature = "mainnet"))]
const EPOCH_LENGTH: u32 = 10;
/// Base seed for the finality-committee sortition; the epoch number is appended so the jury rotates.
/// Single definition lives in consensus-core (the pallets pass the SAME prefix to the SAME election).
const COMMITTEE_SEED: &str = consensus_core::FINALITY_COMMITTEE_SEED;

/// The finality epoch a block height belongs to. Deterministic from the height, so every node agrees on
/// which committee secures which block.
fn epoch_of(height: u32) -> u32 {
    height / EPOCH_LENGTH
}

/// A finality vote: "at `height`, I prefer block `hash`." `voter` is a stable per-node id (dedup +
/// committee/quorum counting). Fixed 72-byte wire layout — dependency-free, deterministic.
/// A finality vote: "at `height`, I prefer block `hash`", signed by `signer` (an sr25519 public key =
/// the voter's reputation account). The signature authenticates the (height, hash) pair so only real
/// committee members can move finality — STEP 3 (Byzantine-safety) over the step-2 unauthenticated votes.
#[derive(Clone, Copy)]
struct Vote {
    height: u64,
    hash: [u8; 32],
    signer: [u8; 32],
    sig: [u8; 64],
}

const VOTE_LEN: usize = 8 + 32 + 32 + 64;

/// The bytes a vote signs over: `height || hash`. Signer and verifier rebuild it identically.
fn sign_message(height: u64, hash: &[u8; 32]) -> Vec<u8> {
    let mut m = Vec::with_capacity(40);
    m.extend_from_slice(&height.to_le_bytes());
    m.extend_from_slice(hash);
    m
}

impl Vote {
    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(VOTE_LEN);
        b.extend_from_slice(&self.height.to_le_bytes());
        b.extend_from_slice(&self.hash);
        b.extend_from_slice(&self.signer);
        b.extend_from_slice(&self.sig);
        b
    }

    fn decode(d: &[u8]) -> Option<Vote> {
        if d.len() != VOTE_LEN {
            return None;
        }
        let mut height = [0u8; 8];
        height.copy_from_slice(&d[0..8]);
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&d[8..40]);
        let mut signer = [0u8; 32];
        signer.copy_from_slice(&d[40..72]);
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&d[72..136]);
        Some(Vote { height: u64::from_le_bytes(height), hash, signer, sig })
    }

    /// Verify the sr25519 signature binds `signer` to this (height, hash).
    fn verify_sig(&self) -> bool {
        let msg = sign_message(self.height, &self.hash);
        let public = sp_core::sr25519::Public::from_raw(self.signer);
        let signature = sp_core::sr25519::Signature::from_raw(self.sig);
        sp_core::sr25519::Pair::verify(&signature, &msg, &public)
    }
}

/// The gossip topic for a height. Reversible (`from_low_u64_be`/`to_low_u64_be`) so the validator can
/// expire votes for already-finalised heights without keeping a side table.
pub fn topic(height: u64) -> Hash {
    H256::from_low_u64_be(height)
}

/// Gossip validator: decodes votes, tags each with its height topic, and expires votes at or below the
/// finalised watermark so old rounds are garbage-collected.
struct GossipValidator {
    watermark: Arc<AtomicU64>,
}

impl Validator<Block> for GossipValidator {
    fn validate(
        &self,
        _ctx: &mut dyn ValidatorContext<Block>,
        _sender: &PeerId,
        data: &[u8],
    ) -> ValidationResult<Hash> {
        match Vote::decode(data) {
            Some(v) if v.height > self.watermark.load(Ordering::Relaxed) => {
                ValidationResult::ProcessAndKeep(topic(v.height))
            }
            _ => ValidationResult::Discard,
        }
    }

    fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Hash, &[u8]) -> bool + 'a> {
        let watermark = self.watermark.clone();
        Box::new(move |topic, _data| topic.to_low_u64_be() <= watermark.load(Ordering::Relaxed))
    }

    fn message_allowed<'a>(
        &'a self,
    ) -> Box<dyn FnMut(&PeerId, MessageIntent, &Hash, &[u8]) -> bool + 'a> {
        let watermark = self.watermark.clone();
        Box::new(move |_who, _intent, topic, _data| {
            topic.to_low_u64_be() > watermark.load(Ordering::Relaxed)
        })
    }
}

/// Build the notification-protocol config + service for the finality gadget. Add the config to the
/// network before `build_network`; pass the service into [`start`]. Same recipe as
/// `sc_consensus_grandpa::grandpa_peers_set_config`.
pub fn protocol_config<N: NetworkBackend<Block, Hash>>(
    metrics: sc_network::service::NotificationMetrics,
    peer_store_handle: Arc<dyn sc_network::peer_store::PeerStoreProvider>,
) -> (N::NotificationProtocolConfig, Box<dyn NotificationService>) {
    N::notification_config(
        PROTOCOL_NAME.into(),
        Vec::new(),
        1024 * 1024,
        None,
        sc_network::config::SetConfig {
            in_peers: 25,
            out_peers: 25,
            reserved_nodes: Vec::new(),
            non_reserved_mode: sc_network::config::NonReservedPeerMode::Accept,
        },
        metrics,
        peer_store_handle,
    )
}

/// Wire the gadget and return its driving future + the gossip driver future. Spawn both as essential
/// tasks. `network`/`sync` are the services from `build_network`; `notification_service` is from
/// [`protocol_config`]; `vote_as` is the sr25519 secret URI this node signs votes with (or `None`).
pub fn start<N, S>(
    client: Arc<FullClient>,
    network: N,
    sync: S,
    notification_service: Box<dyn NotificationService>,
    vote_as: Option<String>,
    round_ms: u64,
    registry: Option<&Registry>,
) -> (impl std::future::Future<Output = ()>, impl std::future::Future<Output = ()>)
where
    N: sc_network_gossip::Network<Block> + Clone + Send + 'static,
    S: sc_network_gossip::Syncing<Block> + Clone + Send + 'static,
{
    let watermark = Arc::new(AtomicU64::new(client.info().finalized_number as u64));
    let validator = Arc::new(GossipValidator { watermark: watermark.clone() });
    let gossip = GossipEngine::new(
        network,
        sync,
        notification_service,
        PROTOCOL_NAME,
        validator,
        registry,
    );
    let gossip = Arc::new(Mutex::new(gossip));

    // Driver: GRANDPA-style — the engine future must be polled to move notifications; the worker holds
    // the same `Arc<Mutex<>>` to send/subscribe. They take turns on the mutex.
    let driver = {
        let gossip = gossip.clone();
        futures::future::poll_fn(move |cx| {
            let r = gossip.lock().expect("gossip mutex").poll_unpin(cx);
            if r.is_ready() {
                log::warn!(target: "woven-trust", "vote gossip engine terminated (network stream closed)");
            }
            r
        })
    };

    // Derive this node's finality-vote key from the secret URI (e.g. `//Alice`). A node without one
    // follows the chain and tallies others' votes but cannot itself vote.
    let vote_pair = vote_as.and_then(|s| match sr25519::Pair::from_string(&s, None) {
        Ok(p) => Some(p),
        Err(_) => {
            log::warn!(target: "woven-trust", "invalid --vote-as secret URI; this node will not vote");
            None
        }
    });
    log::info!(
        target: "woven-trust",
        "finality gadget online: protocol {PROTOCOL_NAME}, round {round_ms}ms, base finalized #{}, voting={}",
        client.info().finalized_number,
        vote_pair.is_some(),
    );
    let worker = run_worker(client, gossip, watermark, vote_pair, round_ms);
    (worker, driver)
}

/// The finality worker: one `FinalityRound` per target height, votes, aggregates, finalises.
async fn run_worker(
    client: Arc<FullClient>,
    gossip: Arc<Mutex<GossipEngine<Block>>>,
    watermark: Arc<AtomicU64>,
    vote_pair: Option<sr25519::Pair>,
    round_ms: u64,
) {
    use consensus_core::{FinalityRound, SnowballParams};

    // This node's vote identity (sr25519 account), if it has a key.
    let vote_signer: Option<[u8; 32]> = vote_pair.as_ref().map(|p| p.public().0);
    // Aim `STEP` ahead of the last finalised block (a height every node agrees on). Finalising it
    // finalises the whole range below, so finality keeps pace with production instead of crawling by one.
    // Block numbers are u32; votes carry the height as u64 on the wire.
    let mut height: u32 = client.info().finalized_number + FINALITY_STEP;
    let mut round = FinalityRound::new(target_hash(&client, height).unwrap_or_default().0);
    // Latest signed vote per COMMITTEE MEMBER (account) at the current height — only valid, in-committee
    // votes. The FULL `Vote` (signer + signature) is kept so the finality justification can carry the
    // signed-vote proof that any peer verifies on import (step 3b).
    let mut votes: BTreeMap<[u8; 32], Vote> = BTreeMap::new();
    // Finality committee + session-key delegation for the current height's epoch; refreshed each tick.
    // Both are read at the SAME epoch-pinned block (`at`) so membership and signer→account resolution
    // never diverge.
    let mut at = epoch_pinned_at(&client, height);
    let mut committee: BTreeSet<[u8; 32]> = committee_for_height(&client, height, at);
    let mut vote_keys_rev: BTreeMap<[u8; 32], [u8; 32]> = vote_keys_rev_for_height(&client, at);
    let mut votes_rx = gossip.lock().expect("gossip mutex").messages_for(topic(height as u64));
    // Consecutive round ticks the current height has sat below the 60% vote quorum (see the guard in
    // the tick branch). Reset on quorum or when the target height moves.
    let mut below_quorum_ticks: u32 = 0;

    let mut timer = futures_timer::Delay::new(std::time::Duration::from_millis(round_ms)).fuse();

    loop {
        futures::select! {
            _ = timer => {
                timer = futures_timer::Delay::new(std::time::Duration::from_millis(round_ms)).fuse();

                // No block at this height yet (chain hasn't produced it) -> nothing to finalise.
                let (pref, have) = match target_hash(&client, height) {
                    Some(h) => h,
                    None => continue,
                };
                if !have {
                    continue;
                }

                // Refresh the committee for THIS height's epoch (sortition over reputation at the epoch
                // start). No cohort yet (fresh chain) -> empty committee -> no finality until it exists.
                at = epoch_pinned_at(&client, height);
                committee = committee_for_height(&client, height, at);
                vote_keys_rev = vote_keys_rev_for_height(&client, at);
                // I1 fix (, epoch-boundary STALE-COMMITTEE race — root cause of the permanent
                // finality split): re-validate the accumulated votes against the CURRENT committee every
                // tick. When `height` advanced across an epoch boundary on the previous finalisation, the
                // committee it carried was the PREVIOUS epoch's; a vote for the new height admitted by the
                // receive branch in that ≤round window (against that stale committee) would persist here,
                // be tallied, and be packed into the proof — a signature that every other node (computing
                // the correct committee for that height) rejects as out-of-committee, so the proof fails
                // to verify anywhere and finality wedges permanently. Pruning to the current committee
                // guarantees the tally AND the emitted proof only ever contain in-committee votes.
                votes.retain(|acc, _| committee.contains(acc));
                let committee_size = committee.len() as u32;
                if committee_size == 0 {
                    continue;
                }

                // Sign + broadcast this node's vote (if it holds a key). Every node gossips its signed
                // vote so peers can verify it; only a signer that is a committee member is tallied.
                if let (Some(pair), Some(signer)) = (&vote_pair, vote_signer) {
                    let msg = sign_message(height as u64, &pref);
                    let v = Vote { height: height as u64, hash: pref, signer, sig: pair.sign(&msg).0 };
                    {
                        let mut g = gossip.lock().expect("gossip mutex");
                        g.register_gossip_message(topic(height as u64), v.encode());
                        g.gossip_message(topic(height as u64), v.encode(), true);
                    }
                    // This node signs with its HOT session key; resolve it to the cold committee
                    // account it votes for, and tally under the account (one slot per member).
                    let account = resolve_voter(&signer, &vote_keys_rev);
                    if committee.contains(&account) {
                        votes.insert(account, v);
                    }
                }

                // STEP 3 committee model: only votes signed by ELECTED committee members count, so an
                // unbacked (sybil) node cannot move finality. `k`/`alpha` scale to the committee size,
                // keeping the 0.70 "wall ♠".
                let k = committee_size;
                let alpha = (((k * 7) + 9) / 10).max((k / 2) + 1);
                let params = SnowballParams { k, alpha, beta: FINALITY_BETA, max_rounds: 80 };

                // The round PERSISTS across ticks (created once per height) so the streak accumulates;
                // params are passed in each tick so committee changes are absorbed without a reset.
                let tally: Vec<[u8; 32]> = votes.values().map(|v| v.hash).collect();
                // Anti-partition guard: only finalise while ≥60% of the committee is seen voting.
                let reaches_quorum = (votes.len() as u32) * 10 >= committee_size * 6;
                // Under-quorum is a DELIBERATE stall (safety over liveness) but used to be silent —
                // an epoch whose committee cannot muster 60% live votes wedges finality at this height
                // with no trace (F3 wedge). Make the wait loud and diagnosable.
                if !reaches_quorum {
                    below_quorum_ticks = below_quorum_ticks.saturating_add(1);
                    if below_quorum_ticks == QUORUM_WARN_AFTER_TICKS
                        || below_quorum_ticks % QUORUM_WARN_EVERY_TICKS == 0
                    {
                        log::warn!(
                            target: "woven-trust",
                            "⏳ finality below quorum at #{height}: {}/{} committee votes (need ≥60%) for {} rounds — waiting, not finalising (safety over liveness)",
                            votes.len(), committee_size, below_quorum_ticks
                        );
                    }
                } else {
                    below_quorum_ticks = 0;
                }

                if let Some(decided) = round.observe_round(&tally, &params, reaches_quorum) {
                    let decided_hash = Hash::from(decided);
                    // Pack the signed votes that back this decision into the justification: this is the
                    // PROOF a peer verifies on import (≥alpha committee signatures over height||hash).
                    let proof = encode_proof(height as u64, &decided, &votes);
                    let justification = (ENGINE_ID, proof);
                    match client.finalize_block(decided_hash, Some(justification), true) {
                        Ok(()) => {
                            log::info!(
                                target: "woven-trust",
                                "🎭 finalised #{height} {decided_hash:?} (committee {committee_size}, alpha {alpha}/{k})"
                            );
                            watermark.store(height as u64, Ordering::Relaxed);
                            // Aim the next chunk: STEP past the new finalised tip (which is now `height`,
                            // since finalising it finalised every ancestor). Keeps finality near the head.
                            height = client.info().finalized_number + FINALITY_STEP;
                            // I1 fix: refresh the committee/vote-key view for the NEW height
                            // IMMEDIATELY on advance, so the vote-receive branch below validates incoming
                            // votes against the correct committee even before the next timer tick. This
                            // closes the epoch-boundary window where the receive path would otherwise admit
                            // an out-of-committee vote against the just-superseded committee. (The prune at
                            // the top of each tick is the belt; this is the suspenders.)
                            at = epoch_pinned_at(&client, height);
                            committee = committee_for_height(&client, height, at);
                            vote_keys_rev = vote_keys_rev_for_height(&client, at);
                            below_quorum_ticks = 0;
                            votes.clear();
                            round = FinalityRound::new(
                                target_hash(&client, height).unwrap_or_default().0,
                            );
                            votes_rx = gossip
                                .lock()
                                .expect("gossip mutex")
                                .messages_for(topic(height as u64));
                        }
                        Err(e) => {
                            log::warn!(target: "woven-trust", "finalise #{height} failed: {e}");
                        }
                    }
                }
            }

            // `.next` (not `select_next_some`): if the gossip receiver terminates (the engine's
            // notification stream closed — a startup race on constrained hardware), `select_next_some`
            // would panic the whole node ("SelectNextSome polled after terminated"). Handle `None`
            // gracefully by re-subscribing so the worker survives instead of crashing.
            maybe_notification = votes_rx.next() => {
                let notification = match maybe_notification {
                    Some(n) => n,
                    None => {
                        log::warn!(target: "woven-trust", "vote stream terminated at h={height}; resubscribing");
                        votes_rx = gossip.lock().expect("gossip mutex").messages_for(topic(height as u64));
                        continue;
                    }
                };
                if let Some(v) = Vote::decode(&notification.message) {
                    // Count a vote only if (1) it is for the current height, (2) its sr25519 signature
                    // is valid, and (3) the signer is an elected committee member. Anything else is
                    // dropped — this is what makes finality Byzantine-safe (sybils cannot vote).
                    // The signer may be a hot session key — resolve to its cold committee account
                    // before the membership check, and dedup the tally by account.
                    let account = resolve_voter(&v.signer, &vote_keys_rev);
                    if v.height == height as u64 && v.verify_sig() && committee.contains(&account) {
                        votes.insert(account, v);
                    }
                }
            }
        }
    }
}

/// The node's preferred (canonical best-chain) block hash at `height`, plus whether such a block
/// exists yet. `([0;32], false)` collapses to a harmless default the round never finalises against.
fn target_hash(client: &Arc<FullClient>, height: u32) -> Option<([u8; 32], bool)> {
    if client.info().best_number < height {
        return Some(([0u8; 32], false));
    }
    match client.hash(height) {
        Ok(Some(h)) => Some((h.0, true)),
        _ => Some(([0u8; 32], false)),
    }
}

/// The finality committee securing a block at `height`: the reputation accounts the sortition elects for
/// that block's EPOCH. The election seed is bound to the epoch (`COMMITTEE_SEED-{epoch}`) so the jury
/// **rotates** each epoch (Art. VI — no one keeps power, step 3d), and the reputation snapshot is read at
/// the epoch's first block so every node — voter, importer, follower — agrees on the same committee for a
/// given block regardless of when it evaluates. Empty on a fresh chain with no consensus reputation yet.
/// The block at which an epoch's on-chain state is read for `height`: the epoch's first block (genesis
/// for epoch 0), falling back to the current best block if that block isn't on this node yet. Computed
/// ONCE per height and passed to BOTH [`committee_for_height`] and [`vote_keys_rev_for_height`] so the
/// committee and the session→account map are read at the IDENTICAL state — a divergence here (e.g. one
/// reads the pinned block, the other falls back) would resolve votes against a different account set
/// than the committee and split finality. Single source of `at` = no such divergence.
fn epoch_pinned_at(client: &Arc<FullClient>, height: u32) -> Hash {
    use harlequin_consensus_api::HarlequinConsensusApi;
    let epoch_start = epoch_of(height) * EPOCH_LENGTH;
    let normal = match client.hash(epoch_start) {
        Ok(Some(h)) => h,
        _ => return client.info().best_hash, // epoch start not on chain yet -> best-effort
    };
    // SELF-HEAL (iron test): if this height's epoch-start state has the guard HALTED, the band's
    // committee is frozen empty and finality would deadlock (committee_for_height returns empty forever, and
    // the target only advances STEP from the stuck point → never crosses the band). Re-anchor the WHOLE read
    // (committee AND vote_keys — they share this single `at`, the reviewer's fork catch) to the recovery height
    // H_r: the block where the share diluted back < 1/3, whose reputation is HEALTHY. Finality then crosses
    // the band validated by a sane committee, never finalising under the compromised one. The runtime
    // resolves the band that covers `height` deterministically (range-aware, multi-halt-safe).
    let best = client.info();
    if client.runtime_api().entrenchment_halted(normal).unwrap_or(false) {
        if let Ok(Some(h_r)) =
            client.runtime_api().entrenchment_recovery_for(best.best_hash, height)
        {
            // Q1 burial guard (the author): only anchor to an H_r that is beyond realistic reorg, so a reorg of
            // H_r to a version with a different `cleared_at` cannot retroactively invalidate a finalisation.
            // While H_r is too fresh we keep the empty committee (finality waits); production buries it fast.
            if h_r >= epoch_start && best.best_number.saturating_sub(h_r) >= RECOVERY_BURIAL {
                if let Ok(Some(hr_hash)) = client.hash(h_r) {
                    return hr_hash;
                }
            }
        }
    }
    normal
}

fn committee_for_height(client: &Arc<FullClient>, height: u32, at: Hash) -> BTreeSet<[u8; 32]> {
    use harlequin_consensus_api::HarlequinConsensusApi;
    let epoch = epoch_of(height);
    // `at` is the epoch-pinned block (see `epoch_pinned_at`), shared with the vote-key map read so both
    // see the same state. Pinning makes the committee a deterministic function of (epoch).
    //
    // SINGLE-SOURCE (F2 piece 6): this fn now only FEEDS on-chain state into
    // `consensus_core::elect_finality_committee` — the one election the pallets call too, so node and
    // runtime can never disagree about who the committee is. The semantics live there (byte-exact
    // extraction of the draw that used to be inline here): entrenchment halt → empty committee
    // (finality freezes, self-heals via `epoch_pinned_at`); beacon populated → §2.1 anti-grinding
    // draw off committed keys; else the genesis/bootstrap fallback draw.
    // I1 hygiene: a runtime-API error here used to default SILENTLY. A silent empty
    // `consensus_reputation` collapses the draw to an EMPTY committee (finality stalls with no trace); a
    // silent default on the others skews the draw. None should ever be invisible — log and let the caller
    // see the degraded input (empty committee ⇒ the tick simply waits, never finalises on a guess).
    let onchain = client.runtime_api().consensus_reputation(at).unwrap_or_else(|e| {
        log::warn!(target: "woven-trust", "committee@{at:?}: consensus_reputation API error: {e:?} — treating as EMPTY (no finality this tick)");
        Default::default()
    });
    let halted = client.runtime_api().entrenchment_halted(at).unwrap_or_else(|e| {
        log::warn!(target: "woven-trust", "committee@{at:?}: entrenchment_halted API error: {e:?} — treating as not-halted");
        false
    });
    let committed = client.runtime_api().beacon_committed_keys(at).unwrap_or_else(|e| {
        log::warn!(target: "woven-trust", "committee@{at:?}: beacon_committed_keys API error: {e:?} — treating as empty");
        Default::default()
    });
    let beacon_seed = client.runtime_api().beacon_seed(at).unwrap_or_else(|e| {
        log::warn!(target: "woven-trust", "committee@{at:?}: beacon_seed API error: {e:?} — treating as zero");
        Default::default()
    });
    consensus_core::elect_finality_committee(
        &onchain,
        &committed,
        &beacon_seed,
        epoch,
        COMMITTEE_TAU,
        COMMITTEE_SEED,
        halted,
    )
}

/// Reverse finality-vote delegation: **hot session pubkey → cold committee account**, read from
/// on-chain state at the SAME epoch-pinned block as [`committee_for_height`]. Reading it from chain
/// state (never local node config) is what makes every node — voter, importer, follower — resolve a
/// vote's signer → its committee account IDENTICALLY; a divergence here would split finality. Empty →
/// every vote resolves to its own signer (direct/legacy signing, dev path).
fn vote_keys_rev_for_height(client: &Arc<FullClient>, at: Hash) -> BTreeMap<[u8; 32], [u8; 32]> {
    use harlequin_consensus_api::HarlequinConsensusApi;
    // `at` is the SAME epoch-pinned block passed to `committee_for_height` (see `epoch_pinned_at`), so
    // the session→account map is read at the identical state as the committee — never divergently.
    client
        .runtime_api()
        .vote_keys(at)
        .unwrap_or_default()
        .into_iter()
        .map(|(account, session)| (session, account)) // index by the hot key that actually signs
        .collect()
}

/// Resolve a vote's `signer` (which may be a hot **session key**) to the COLD committee **account**
/// it votes for. A signer with no delegation resolves to itself (direct signing). Committee membership
/// is ALWAYS checked against the returned account, so a founder's sovereign key never has to sign — its
/// genesis-delegated session key does, and the vote still counts for (and is deduped under) the account.
#[inline]
fn resolve_voter(signer: &[u8; 32], rev: &BTreeMap<[u8; 32], [u8; 32]>) -> [u8; 32] {
    rev.get(signer).copied().unwrap_or(*signer)
}

// ───────────────────────── STEP 3b: verifiable finality justifications ─────────────────────────
//
// Until now finality was applied locally (each node ran its round and called `finalize_block`) with a
// justification that carried only the decided hash — unverifiable. Step 3b makes finality stand on its
// own: the justification is a PROOF (the signed committee votes), and the import pipeline VERIFIES that
// proof before accepting finality from anyone. A peer cannot make us finalise a block it did not earn.

/// Justification proof header: `height (u64 LE) || hash (32) || count (u16 LE)`. Followed by `count`
/// records of `signer (32) || sig (64)`. Dependency-free, deterministic wire layout.
const PROOF_HEADER: usize = 8 + 32 + 2;
/// One signed vote inside the proof: sr25519 public key (account) + signature.
const PROOF_VOTE: usize = 32 + 64;

/// Encode the finality proof for a decided `(height, hash)`: every retained committee vote whose hash
/// matches the decision, as `signer||sig`. The verifier rebuilds `height||hash` and checks each signature.
fn encode_proof(height: u64, hash: &[u8; 32], votes: &BTreeMap<[u8; 32], Vote>) -> Vec<u8> {
    let selected: Vec<&Vote> =
        votes.values().filter(|v| v.height == height && &v.hash == hash).collect();
    let count = selected.len().min(u16::MAX as usize);
    let mut b = Vec::with_capacity(PROOF_HEADER + count * PROOF_VOTE);
    b.extend_from_slice(&height.to_le_bytes());
    b.extend_from_slice(hash);
    b.extend_from_slice(&(count as u16).to_le_bytes());
    for v in selected.iter().take(count) {
        b.extend_from_slice(&v.signer);
        b.extend_from_slice(&v.sig);
    }
    b
}

/// Verify a finality proof: the block at `(number, hash)` was finalised by **≥alpha distinct elected
/// committee members**, each signing `height||hash` with their sr25519 account key. `committee` is the
/// elected finality committee at that height. `Ok` iff the proof is sound — the SAME `k/alpha` wall
/// (0.70 quorum) the worker uses, so a proof that passes here is exactly one the committee could produce.
fn verify_proof(
    hash: &[u8; 32],
    number: u64,
    proof: &[u8],
    committee: &BTreeSet<[u8; 32]>,
    vote_keys_rev: &BTreeMap<[u8; 32], [u8; 32]>,
) -> Result<(), &'static str> {
    if proof.len() < PROOF_HEADER {
        return Err("proof too short");
    }
    let mut h = [0u8; 8];
    h.copy_from_slice(&proof[0..8]);
    let p_height = u64::from_le_bytes(h);
    let mut p_hash = [0u8; 32];
    p_hash.copy_from_slice(&proof[8..40]);
    let mut c = [0u8; 2];
    c.copy_from_slice(&proof[40..42]);
    let count = u16::from_le_bytes(c) as usize;

    if p_height != number {
        return Err("height mismatch");
    }
    if &p_hash != hash {
        return Err("hash mismatch");
    }
    if proof.len() != PROOF_HEADER + count * PROOF_VOTE {
        return Err("proof length mismatch");
    }

    let k = committee.len() as u32;
    if k == 0 {
        return Err("empty committee");
    }
    // ceil(0.70·k), floored at a strict majority — identical to the worker's `alpha`.
    let alpha = (((k * 7) + 9) / 10).max((k / 2) + 1);

    let msg = sign_message(p_height, &p_hash);
    let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
    for i in 0..count {
        let off = PROOF_HEADER + i * PROOF_VOTE;
        let mut signer = [0u8; 32];
        signer.copy_from_slice(&proof[off..off + 32]);
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&proof[off + 32..off + PROOF_VOTE]);
        // The proof carries the HOT session key that actually signed. Resolve it to the cold
        // committee account (identical resolution to the worker's tally — same on every node) and
        // count only valid signatures from distinct elected committee members.
        let account = resolve_voter(&signer, vote_keys_rev);
        if !committee.contains(&account) {
            continue;
        }
        // The signature is verified against the SIGNER (the session pubkey that made it), while
        // membership and the distinct-member count are keyed by the resolved account.
        let public = sr25519::Public::from_raw(signer);
        let signature = sr25519::Signature::from_raw(sig);
        if !sr25519::Pair::verify(&signature, &msg, &public) {
            continue;
        }
        seen.insert(account);
    }
    if (seen.len() as u32) >= alpha {
        Ok(())
    } else {
        Err("insufficient committee signatures")
    }
}

/// Block-import wrapper: verifies any Woven-Trust finality justification ATTACHED to an imported block
/// before delegating to the inner import. A peer cannot make us accept finality it did not earn — the
/// attached proof must carry ≥alpha signed committee votes. Blocks without our justification (plain
/// sync, other engines) pass straight through.
pub struct WovenTrustBlockImport<I> {
    inner: I,
    client: Arc<FullClient>,
}

impl<I> WovenTrustBlockImport<I> {
    pub fn new(inner: I, client: Arc<FullClient>) -> Self {
        Self { inner, client }
    }
}

#[async_trait::async_trait]
impl<I> BlockImport<Block> for WovenTrustBlockImport<I>
where
    I: BlockImport<Block, Error = ConsensusError> + Send + Sync,
{
    type Error = ConsensusError;

    async fn check_block(
        &self,
        block: BlockCheckParams<Block>,
    ) -> Result<ImportResult, Self::Error> {
        self.inner.check_block(block).await
    }

    async fn import_block(
        &self,
        block: BlockImportParams<Block>,
    ) -> Result<ImportResult, Self::Error> {
        if let Some(justifications) = &block.justifications {
            if let Some(proof) = justifications.get(ENGINE_ID) {
                let hash = block.post_hash();
                let number = (*block.header.number()) as u64;
                let at = epoch_pinned_at(&self.client, number as u32);
                let committee = committee_for_height(&self.client, number as u32, at);
                let vote_keys_rev = vote_keys_rev_for_height(&self.client, at);
                verify_proof(&hash.0, number, proof, &committee, &vote_keys_rev).map_err(|e| {
                    ConsensusError::ClientImport(format!(
                        "woven-trust finality proof rejected at #{number}: {e}"
                    ))
                })?;
            }
        }
        self.inner.import_block(block).await
    }
}

/// Justification import: when the network delivers a finality justification for a block (sync / catch-up),
/// VERIFY its signed-vote proof and only then finalise. This is how a freshly-syncing node accepts
/// finality from peers WITHOUT trusting them — the proof stands on its own.
pub struct WovenTrustJustificationImport {
    client: Arc<FullClient>,
}

impl WovenTrustJustificationImport {
    pub fn new(client: Arc<FullClient>) -> Self {
        Self { client }
    }
}

#[async_trait::async_trait]
impl JustificationImport<Block> for WovenTrustJustificationImport {
    type Error = ConsensusError;

    async fn on_start(&mut self) -> Vec<(Hash, NumberFor<Block>)> {
        Vec::new()
    }

    async fn import_justification(
        &mut self,
        hash: Hash,
        number: NumberFor<Block>,
        justification: Justification,
    ) -> Result<(), Self::Error> {
        let (engine, proof) = justification;
        if engine != ENGINE_ID {
            return Ok(()); // not our finality — ignore
        }
        let at = epoch_pinned_at(&self.client, number as u32);
        let committee = committee_for_height(&self.client, number as u32, at);
        let vote_keys_rev = vote_keys_rev_for_height(&self.client, at);
        verify_proof(&hash.0, number as u64, &proof, &committee, &vote_keys_rev).map_err(|e| {
            ConsensusError::ClientImport(format!(
                "woven-trust justification rejected at #{number}: {e}"
            ))
        })?;
        self.client.finalize_block(hash, Some((ENGINE_ID, proof)), true).map_err(|e| {
            ConsensusError::ClientImport(format!("finalise on justification import: {e}"))
        })?;
        Ok(())
    }
}

/// Import-queue verifier: like manual-seal's, it does NOT auto-finalise on import and picks the longest
/// chain. Finality is owned by the gadget and gated by justification verification, never by this verifier.
struct WovenTrustImportVerifier;

#[async_trait::async_trait]
impl Verifier<Block> for WovenTrustImportVerifier {
    async fn verify(
        &self,
        mut block: BlockImportParams<Block>,
    ) -> Result<BlockImportParams<Block>, String> {
        block.finalized = false;
        block.fork_choice = Some(ForkChoiceStrategy::LongestChain);
        Ok(block)
    }
}

/// Build the import queue with Woven-Trust finality verification wired in: imported blocks and
/// justifications carrying a `WTC1` finality proof are verified (≥alpha committee signatures) before
/// finality is accepted. Drop-in replacement for `sc_consensus_manual_seal::import_queue`.
pub fn build_import_queue(
    client: Arc<FullClient>,
    spawner: &impl sp_core::traits::SpawnEssentialNamed,
    registry: Option<&Registry>,
) -> BasicQueue<Block> {
    let block_import: BoxBlockImport<Block> =
        Box::new(WovenTrustBlockImport::new(client.clone(), client.clone()));
    let justification_import: BoxJustificationImport<Block> =
        Box::new(WovenTrustJustificationImport::new(client));
    BasicQueue::new(
        WovenTrustImportVerifier,
        block_import,
        Some(justification_import),
        spawner,
        registry,
    )
}

// ─────────────────────── STEP 3c: finality-proof distribution (gossip) ───────────────────────
//
// 3b made finality VERIFIABLE on import; 3c makes the proof ARRIVE. A second gossip protocol carries the
// finality proofs themselves so a node that does NOT run the voting gadget (a `follower`) — or one catching
// up — receives the signed-vote proof, verifies it (same `verify_proof`), and finalises. Decoupled design:
// every node periodically re-broadcasts the proof of ITS OWN latest finalised block (read back from the
// client's stored justification); any node that has the matching block but hasn't finalised it imports the
// proof, verifies, and finalises. No trust in the sender — the proof stands on its own.

/// Notification protocol for finality-proof distribution. Separate from the vote protocol so followers
/// (which never vote) can still subscribe and receive proofs. Versioned.
pub const PROOF_PROTOCOL_NAME: &str = "/harlequin/finality-proof/1";

/// Single fixed gossip topic for proofs: re-broadcasts of the latest proof land on the same topic, and a
/// newly-connected peer is handed the registered latest. Per-height expiry is by decoding the message.
fn proof_topic() -> Hash {
    H256::from_low_u64_be(u64::MAX) // distinct from any vote topic (which uses the height)
}

/// Read `(height, hash)` from a proof's header without verifying it (cheap pre-filter).
fn proof_header(proof: &[u8]) -> Option<(u64, [u8; 32])> {
    if proof.len() < PROOF_HEADER {
        return None;
    }
    let mut h = [0u8; 8];
    h.copy_from_slice(&proof[0..8]);
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&proof[8..40]);
    Some((u64::from_le_bytes(h), hash))
}

/// Gossip validator for proofs: keep proofs above the local finalised watermark, expire the rest.
struct ProofGossipValidator {
    watermark: Arc<AtomicU64>,
}

impl Validator<Block> for ProofGossipValidator {
    fn validate(
        &self,
        _ctx: &mut dyn ValidatorContext<Block>,
        _sender: &PeerId,
        data: &[u8],
    ) -> ValidationResult<Hash> {
        match proof_header(data) {
            Some((height, _)) if height > self.watermark.load(Ordering::Relaxed) => {
                ValidationResult::ProcessAndKeep(proof_topic())
            }
            _ => ValidationResult::Discard,
        }
    }

    fn message_expired<'a>(&'a self) -> Box<dyn FnMut(Hash, &[u8]) -> bool + 'a> {
        let watermark = self.watermark.clone();
        Box::new(move |_topic, data| match proof_header(data) {
            Some((height, _)) => height <= watermark.load(Ordering::Relaxed),
            None => true,
        })
    }

    fn message_allowed<'a>(
        &'a self,
    ) -> Box<dyn FnMut(&PeerId, MessageIntent, &Hash, &[u8]) -> bool + 'a> {
        let watermark = self.watermark.clone();
        Box::new(move |_who, _intent, _topic, data| match proof_header(data) {
            Some((height, _)) => height > watermark.load(Ordering::Relaxed),
            None => false,
        })
    }
}

/// Build the notification-protocol config + service for finality-proof distribution. Register on EVERY
/// node that follows the chain (incl. followers) so proofs reach non-voting nodes.
pub fn proof_protocol_config<N: NetworkBackend<Block, Hash>>(
    metrics: sc_network::service::NotificationMetrics,
    peer_store_handle: Arc<dyn sc_network::peer_store::PeerStoreProvider>,
) -> (N::NotificationProtocolConfig, Box<dyn NotificationService>) {
    N::notification_config(
        PROOF_PROTOCOL_NAME.into(),
        Vec::new(),
        1024 * 1024,
        None,
        sc_network::config::SetConfig {
            in_peers: 25,
            out_peers: 25,
            reserved_nodes: Vec::new(),
            non_reserved_mode: sc_network::config::NonReservedPeerMode::Accept,
        },
        metrics,
        peer_store_handle,
    )
}

/// Wire the finality-proof distributor and return its worker + gossip-driver futures (spawn both as
/// essential tasks, like the vote gadget). Runs on EVERY chain-following node, voter or follower.
pub fn start_proof_distribution<N, S>(
    client: Arc<FullClient>,
    network: N,
    sync: S,
    notification_service: Box<dyn NotificationService>,
    round_ms: u64,
    registry: Option<&Registry>,
) -> (impl std::future::Future<Output = ()>, impl std::future::Future<Output = ()>)
where
    N: sc_network_gossip::Network<Block> + Clone + Send + 'static,
    S: sc_network_gossip::Syncing<Block> + Clone + Send + 'static,
{
    let watermark = Arc::new(AtomicU64::new(client.info().finalized_number as u64));
    let validator = Arc::new(ProofGossipValidator { watermark: watermark.clone() });
    let gossip = GossipEngine::new(
        network,
        sync,
        notification_service,
        PROOF_PROTOCOL_NAME,
        validator,
        registry,
    );
    let gossip = Arc::new(Mutex::new(gossip));

    let driver = {
        let gossip = gossip.clone();
        futures::future::poll_fn(move |cx| {
            let r = gossip.lock().expect("proof gossip mutex").poll_unpin(cx);
            if r.is_ready() {
                log::warn!(target: "woven-trust", "proof gossip engine terminated (network stream closed)");
            }
            r
        })
    };
    log::info!(
        target: "woven-trust",
        "finality-proof distributor online: protocol {PROOF_PROTOCOL_NAME}, base finalized #{}",
        client.info().finalized_number,
    );
    let worker = run_proof_worker(client, gossip, watermark, round_ms);
    (worker, driver)
}

/// The proof distributor: each tick re-broadcasts our latest finalised proof, and finalises any cached
/// received proof whose block we now have. Caches the highest received proof so a block/proof timing gap
/// (proof seen before the block synced) resolves on a later tick.
async fn run_proof_worker(
    client: Arc<FullClient>,
    gossip: Arc<Mutex<GossipEngine<Block>>>,
    watermark: Arc<AtomicU64>,
    round_ms: u64,
) {
    let mut proofs_rx = gossip.lock().expect("proof gossip mutex").messages_for(proof_topic());
    let mut timer = futures_timer::Delay::new(std::time::Duration::from_millis(round_ms)).fuse();
    // Highest received-but-not-yet-applied proof: (height, hash, bytes).
    let mut pending: Option<(u64, [u8; 32], Vec<u8>)> = None;

    loop {
        futures::select! {
            _ = timer => {
                timer = futures_timer::Delay::new(std::time::Duration::from_millis(round_ms)).fuse();

                // PUBLISH: re-broadcast the proof of our own latest finalised block (if any) so peers /
                // newly-synced followers can catch up. Read it back from the client's stored justification.
                let info = client.info();
                if info.finalized_number > 0 {
                    if let Ok(Some(justs)) = client.justifications(info.finalized_hash) {
                        if let Some(proof) = justs.into_justification(ENGINE_ID) {
                            let mut g = gossip.lock().expect("proof gossip mutex");
                            g.register_gossip_message(proof_topic(), proof.clone());
                            g.gossip_message(proof_topic(), proof, false);
                        }
                    }
                }

                // APPLY: try to finalise the cached proof now that we may have synced its block.
                if let Some((height, hash, proof)) = pending.clone() {
                    if try_apply_proof(&client, &watermark, height, &hash, &proof) {
                        pending = None;
                    } else if height <= client.info().finalized_number as u64 {
                        pending = None; // already final by other means -> drop
                    }
                }
            }

            // `.next` (not `select_next_some`): survive gossip-receiver termination instead of
            // panicking the node (see the vote worker for the full rationale).
            maybe_notification = proofs_rx.next() => {
                let notification = match maybe_notification {
                    Some(n) => n,
                    None => {
                        log::warn!(target: "woven-trust", "proof stream terminated; resubscribing");
                        proofs_rx = gossip.lock().expect("proof gossip mutex").messages_for(proof_topic());
                        continue;
                    }
                };
                if let Some((height, hash)) = proof_header(&notification.message) {
                    if height > watermark.load(Ordering::Relaxed) {
                        // Cache the highest proof; apply immediately if the block is already here.
                        let take = pending.as_ref().map(|(h, _, _)| height > *h).unwrap_or(true);
                        if take {
                            pending = Some((height, hash, notification.message.clone()));
                        }
                        if try_apply_proof(&client, &watermark, height, &hash, &notification.message) {
                            pending = None;
                        }
                    }
                }
            }
        }
    }
}

/// Verify a received proof against our own block at that height and finalise if sound. Returns true iff
/// the block was finalised by this call (or is already final). No trust in the sender — `verify_proof`
/// requires ≥alpha signed committee votes binding exactly this `(height, hash)`.
fn try_apply_proof(
    client: &Arc<FullClient>,
    watermark: &Arc<AtomicU64>,
    height: u64,
    hash: &[u8; 32],
    proof: &[u8],
) -> bool {
    if height <= client.info().finalized_number as u64 {
        watermark.store(client.info().finalized_number as u64, Ordering::Relaxed);
        return true;
    }
    // We must have the block at this height on our chain, with exactly this hash.
    match client.hash(height as u32) {
        Ok(Some(local)) if local.0 == *hash => {}
        Ok(Some(local)) => {
            // I1: a silent early-return here wedged n2 invisibly. A hash divergence at a
            // finalised height is a real fork signal — never swallow it.
            log::warn!(
                target: "woven-trust",
                "import proof #{height}: local block {:?} != proof hash {:?} — not applying (fork?)",
                local, Hash::from(*hash)
            );
            return false;
        }
        _ => {
            // The block at this height is not on our chain yet (still syncing). Not an error: the retry
            // loop re-applies once the block arrives. Logged so the wait is visible, not silent (I1).
            log::debug!(
                target: "woven-trust",
                "import proof #{height}: block not on chain yet — will retry when it arrives"
            );
            return false;
        }
    }
    let at = epoch_pinned_at(client, height as u32);
    let committee = committee_for_height(client, height as u32, at);
    let vote_keys_rev = vote_keys_rev_for_height(client, at);
    if let Err(e) = verify_proof(hash, height, proof, &committee, &vote_keys_rev) {
        // I1 root-cause probe: this early-return was SILENT and is where n2's wedge lived —
        // it received a valid proof(#40) but never finalised, with no trace. Dump the EXACT committee this
        // node computed AND the proof's signers (resolved to accounts, marked in/OUT of that committee),
        // so the divergent member is visible on sight — settles whether the input (consensus_reputation
        // at the epoch-pinned block) diverges or the vote-key resolution does.
        let committee_hex: Vec<String> = committee.iter().map(consensus_core::beacon::hex).collect();
        let mut signer_hex: Vec<String> = Vec::new();
        if proof.len() >= PROOF_HEADER {
            let mut c = [0u8; 2];
            c.copy_from_slice(&proof[40..42]);
            let count = u16::from_le_bytes(c) as usize;
            for i in 0..count {
                let off = PROOF_HEADER + i * PROOF_VOTE;
                if off + 32 <= proof.len() {
                    let mut s = [0u8; 32];
                    s.copy_from_slice(&proof[off..off + 32]);
                    let acc = resolve_voter(&s, &vote_keys_rev);
                    let mark = if committee.contains(&acc) { "in" } else { "OUT" };
                    signer_hex.push(format!("{}({mark})", consensus_core::beacon::hex(&acc)));
                }
            }
        }
        log::warn!(
            target: "woven-trust",
            "import proof #{height} REJECTED: {e} | committee(size {}): {:?} | proof signers: {:?}",
            committee.len(), committee_hex, signer_hex
        );
        return false;
    }
    let bh = Hash::from(*hash);
    match client.finalize_block(bh, Some((ENGINE_ID, proof.to_vec())), true) {
        Ok(()) => {
            watermark.store(height, Ordering::Relaxed);
            log::info!(
                target: "woven-trust",
                "🎭 finalised #{height} {bh:?} via imported proof (committee {})",
                committee.len()
            );
            true
        }
        Err(e) => {
            log::debug!(target: "woven-trust", "finalise via proof #{height} failed: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a committee of `n` sr25519 founders (`//F0`..) and return their pairs + the account set.
    fn founders(n: usize) -> (Vec<sr25519::Pair>, BTreeSet<[u8; 32]>) {
        let pairs: Vec<sr25519::Pair> = (0..n)
            .map(|i| sr25519::Pair::from_string(&format!("//F{i}"), None).unwrap())
            .collect();
        let set = pairs.iter().map(|p| p.public().0).collect();
        (pairs, set)
    }

    /// A signed vote by `pair` for `(height, hash)`, as the worker would build it.
    fn vote(pair: &sr25519::Pair, height: u64, hash: [u8; 32]) -> Vote {
        let msg = sign_message(height, &hash);
        Vote { height, hash, signer: pair.public().0, sig: pair.sign(&msg).0 }
    }

    fn votes_map(votes: Vec<Vote>) -> BTreeMap<[u8; 32], Vote> {
        votes.into_iter().map(|v| (v.signer, v)).collect()
    }

    #[test]
    fn epoch_of_partitions_heights() {
        // Heights group into fixed-length epochs; the boundary is deterministic.
        assert_eq!(epoch_of(0), 0);
        assert_eq!(epoch_of(EPOCH_LENGTH - 1), 0);
        assert_eq!(epoch_of(EPOCH_LENGTH), 1);
        assert_eq!(epoch_of(2 * EPOCH_LENGTH + 3), 2);
    }

    #[test]
    fn committee_rotates_each_epoch() {
        // A cohort large enough that the reputation-weighted sortition draws a real subset.
        let mut reputation: BTreeMap<String, i128> = BTreeMap::new();
        let mut keys: BTreeMap<String, String> = BTreeMap::new();
        for i in 0..24 {
            let id = format!("acc{i:02}");
            reputation.insert(id.clone(), 1_000);
            keys.insert(id.clone(), format!("sk-{id}"));
        }
        // Elect the committee per epoch exactly as `committee_for_height` does: seed bound to the epoch.
        let committee = |epoch: u32| -> BTreeSet<String> {
            let seed = format!("{COMMITTEE_SEED}-{epoch}");
            consensus_core::elect_committee_fp(&reputation, &keys, &seed, COMMITTEE_TAU)
                .into_keys()
                .collect()
        };
        let epochs: Vec<BTreeSet<String>> = (0..6).map(committee).collect();
        // Each epoch elects a non-empty STRICT subset of the cohort.
        for c in &epochs {
            assert!(!c.is_empty() && c.len() < 24);
        }
        // The jury ROTATES (Art. VI): membership is not frozen across epochs.
        assert!(epochs.windows(2).any(|w| w[0] != w[1]), "committee must rotate across epochs");
    }

    #[test]
    fn quorum_proof_verifies() {
        let (pairs, committee) = founders(6); // k=6 -> alpha=5
        let hash = [7u8; 32];
        // 5 of 6 founders sign -> meets alpha.
        let m = votes_map(pairs.iter().take(5).map(|p| vote(p, 3, hash)).collect());
        let proof = encode_proof(3, &hash, &m);
        assert!(verify_proof(&hash, 3, &proof, &committee, &BTreeMap::new()).is_ok());
    }

    #[test]
    fn below_alpha_rejected() {
        let (pairs, committee) = founders(6); // alpha=5
        let hash = [7u8; 32];
        let m = votes_map(pairs.iter().take(4).map(|p| vote(p, 3, hash)).collect()); // only 4
        let proof = encode_proof(3, &hash, &m);
        assert_eq!(verify_proof(&hash, 3, &proof, &committee, &BTreeMap::new()), Err("insufficient committee signatures"));
    }

    #[test]
    fn outsider_signatures_dont_count() {
        let (pairs, committee) = founders(6); // alpha=5
        let hash = [7u8; 32];
        // 4 real founders + 3 outsiders (not in committee) -> still only 4 valid -> rejected.
        let outsiders: Vec<sr25519::Pair> = (0..3)
            .map(|i| sr25519::Pair::from_string(&format!("//Evil{i}"), None).unwrap())
            .collect();
        let mut all: Vec<Vote> = pairs.iter().take(4).map(|p| vote(p, 3, hash)).collect();
        all.extend(outsiders.iter().map(|p| vote(p, 3, hash)));
        let proof = encode_proof(3, &hash, &votes_map(all));
        assert!(verify_proof(&hash, 3, &proof, &committee, &BTreeMap::new()).is_err());
    }

    #[test]
    fn wrong_hash_or_height_rejected() {
        let (pairs, committee) = founders(6);
        let hash = [7u8; 32];
        let m = votes_map(pairs.iter().take(5).map(|p| vote(p, 3, hash)).collect());
        let proof = encode_proof(3, &hash, &m);
        // Same proof, different block hash / height -> rejected (binding check).
        assert_eq!(verify_proof(&[9u8; 32], 3, &proof, &committee, &BTreeMap::new()), Err("hash mismatch"));
        assert_eq!(verify_proof(&hash, 4, &proof, &committee, &BTreeMap::new()), Err("height mismatch"));
    }

    #[test]
    fn tampered_signature_rejected() {
        let (pairs, committee) = founders(6); // alpha=5
        let hash = [7u8; 32];
        let mut votes: Vec<Vote> = pairs.iter().take(5).map(|p| vote(p, 3, hash)).collect();
        votes[0].sig[0] ^= 0xff; // corrupt one signature -> 4 valid left -> below alpha
        let proof = encode_proof(3, &hash, &votes_map(votes));
        assert!(verify_proof(&hash, 3, &proof, &committee, &BTreeMap::new()).is_err());
    }

    #[test]
    fn empty_committee_never_finalises() {
        let (pairs, _) = founders(3);
        let hash = [7u8; 32];
        let m = votes_map(pairs.iter().map(|p| vote(p, 1, hash)).collect());
        let proof = encode_proof(1, &hash, &m);
        assert_eq!(verify_proof(&hash, 1, &proof, &BTreeSet::new(), &BTreeMap::new()), Err("empty committee"));
    }

    /// Session-key delegation: the committee is the COLD founder accounts, but the votes are signed by
    /// distinct HOT session keys mapped to those founders. A proof of ≥alpha session signatures
    /// finalises FOR the cold founders — the sovereign keys never sign. This is the core of the
    /// cold-sovereign / hot-session model.
    #[test]
    fn session_keys_finalise_for_cold_founders() {
        let (cold, committee) = founders(6); // k=6 -> alpha=5; these accounts NEVER sign
        let hash = [7u8; 32];
        let sessions: Vec<sr25519::Pair> = (0..6)
            .map(|i| sr25519::Pair::from_string(&format!("//S{i}"), None).unwrap())
            .collect();
        // What the runtime API yields, reversed by the node: hot session pubkey -> cold account.
        let rev: BTreeMap<[u8; 32], [u8; 32]> =
            sessions.iter().zip(cold.iter()).map(|(s, f)| (s.public().0, f.public().0)).collect();
        // 5 of 6 session keys sign -> alpha met, each resolves to a distinct cold founder.
        let m = votes_map(sessions.iter().take(5).map(|p| vote(p, 3, hash)).collect());
        let proof = encode_proof(3, &hash, &m);
        assert!(
            verify_proof(&hash, 3, &proof, &committee, &rev).is_ok(),
            "delegated session keys finalise for the cold founders without the sovereign keys signing"
        );
    }

    /// A session key with NO genesis delegation resolves to itself, is not in the committee, and so
    /// cannot move finality — an impostor cannot mint authority by inventing a hot key.
    #[test]
    fn undelegated_session_key_is_impostor() {
        let (cold, committee) = founders(6); // alpha=5
        let hash = [7u8; 32];
        let sessions: Vec<sr25519::Pair> = (0..6)
            .map(|i| sr25519::Pair::from_string(&format!("//S{i}"), None).unwrap())
            .collect();
        // Only 4 founders delegated; the other 2 session keys are not in the map (impostors).
        let rev: BTreeMap<[u8; 32], [u8; 32]> =
            sessions.iter().take(4).zip(cold.iter()).map(|(s, f)| (s.public().0, f.public().0)).collect();
        // All 6 session keys sign, but only 4 resolve to committee accounts -> below alpha=5 -> rejected.
        let m = votes_map(sessions.iter().map(|p| vote(p, 3, hash)).collect());
        let proof = encode_proof(3, &hash, &m);
        assert!(
            verify_proof(&hash, 3, &proof, &committee, &rev).is_err(),
            "session keys with no genesis delegation cannot move finality"
        );
    }
}

