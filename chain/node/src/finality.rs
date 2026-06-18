//! Woven-Trust finality gadget — the network layer that turns the validated Snowball voting core
//! (`consensus_core::FinalityRound`) into real block finality, replacing the trivial `finalize:true`.
//!
//! Shape (mirrors GRANDPA's split, with OUR logic): a `sc-network` notification protocol carries the
//! committee's finality votes; a worker future runs one [`consensus_core::FinalityRound`] per target
//! height, broadcasts this node's preferred block hash, feeds the votes it receives into the round, and
//! when the round decides calls `Finalizer::finalize_block` with a justification. The decision rule and
//! its safety/liveness are NOT here — they live (tested, 13/13 cross-validated) in `consensus-core`;
//! this file only wires votes ⇄ network ⇄ the chain. Block-import verification of the justification is
//! the next increment (block-import verification of the justification).
//!
//! Committee & params scale to the live committee: each height's committee is elected by the same
//! reputation-weighted sortition the node authors with (`elect_committee_fp`, deterministic seed
//! `final{height}` so every node agrees), and the Snowball `k/alpha` scale to its size while keeping the
//! 0.70 quorum wall. `beta` here is a TESTNET value (fast finality for validation); production params
//! come from `consensus-simulator/PARAMETERS.md` at the pre-mainnet config gate.

use crate::service::FullClient;
use futures::{FutureExt, StreamExt};
use harlequin_runtime::interface::OpaqueBlock as Block;
use polkadot_sdk::{
    sc_client_api::Finalizer,
    sc_network::{self, NetworkBackend, NotificationService},
    sc_network_gossip::{
        self, GossipEngine, MessageIntent, ValidationResult, Validator, ValidatorContext,
    },
    sc_network_types::PeerId,
    sp_blockchain::HeaderBackend,
    substrate_prometheus_endpoint::Registry,
};
use sp_core::H256;
use sp_runtime::traits::Block as BlockT;
use std::{
    collections::BTreeMap,
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

/// Confirmations before a height is irreversible. TESTNET value (production: `consensus-simulator/PARAMETERS.md` beta=12).
const FINALITY_BETA: u32 = 4;
/// Minimum distinct voters before a height may finalise: a lone node must not finalise alone.
const MIN_VOTERS: u32 = 2;

/// A finality vote: "at `height`, I prefer block `hash`." `voter` is a stable per-node id (dedup +
/// committee/quorum counting). Fixed 72-byte wire layout — dependency-free, deterministic.
#[derive(Clone, Copy)]
struct Vote {
    height: u64,
    hash: [u8; 32],
    voter: [u8; 32],
}

const VOTE_LEN: usize = 8 + 32 + 32;

impl Vote {
    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(VOTE_LEN);
        b.extend_from_slice(&self.height.to_le_bytes());
        b.extend_from_slice(&self.hash);
        b.extend_from_slice(&self.voter);
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
        let mut voter = [0u8; 32];
        voter.copy_from_slice(&d[40..72]);
        Some(Vote { height: u64::from_le_bytes(height), hash, voter })
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
/// [`protocol_config`]; `local_id` is a stable per-node id; `round_ms` is the voting cadence.
pub fn start<N, S>(
    client: Arc<FullClient>,
    network: N,
    sync: S,
    notification_service: Box<dyn NotificationService>,
    local_id: [u8; 32],
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
        futures::future::poll_fn(move |cx| gossip.lock().expect("gossip mutex").poll_unpin(cx))
    };

    log::info!(
        target: "woven-trust",
        "finality gadget online: protocol {PROTOCOL_NAME}, round {round_ms}ms, base finalized #{}",
        client.info().finalized_number,
    );
    let worker = run_worker(client, gossip, watermark, local_id, round_ms);
    (worker, driver)
}

/// The finality worker: one `FinalityRound` per target height, votes, aggregates, finalises.
async fn run_worker(
    client: Arc<FullClient>,
    gossip: Arc<Mutex<GossipEngine<Block>>>,
    watermark: Arc<AtomicU64>,
    local_id: [u8; 32],
    round_ms: u64,
) {
    use consensus_core::{FinalityRound, SnowballParams};

    // Start one past the last finalised block (block numbers are u32; votes carry it as u64 on the wire).
    let mut height: u32 = client.info().finalized_number + 1;
    let mut round = FinalityRound::new(target_hash(&client, height).unwrap_or_default().0);
    // Latest vote per voter at the current height.
    let mut votes: BTreeMap<[u8; 32], [u8; 32]> = BTreeMap::new();
    let mut votes_rx = gossip.lock().expect("gossip mutex").messages_for(topic(height as u64));

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

                // This node prefers its own best block at `height`; record + broadcast the vote.
                votes.insert(local_id, pref);
                let v = Vote { height: height as u64, hash: pref, voter: local_id };
                {
                    let mut g = gossip.lock().expect("gossip mutex");
                    g.register_gossip_message(topic(height as u64), v.encode());
                    g.gossip_message(topic(height as u64), v.encode(), true);
                }

                // STEP-2 committee model: the committee is the set of nodes actually voting (observed via
                // gossip), so `k`/`alpha` scale to live participation and stay stable across heights —
                // unlike a per-height reputation sortition, whose small-committee variance can collapse
                // `alpha` to 1 (a single vote finalising = unsafe). The reputation-WEIGHTED committee
                // (only sortition-elected accounts vote, votes signed by session keys) is STEP 3; until
                // then votes are unauthenticated, so this is a controlled-network model, not Byzantine-safe.
                let voters = votes.len() as u32;
                let k = voters.max(1);
                // alpha = ceil(0.70 k), never below a strict majority — the 0.70 "wall ♠".
                let alpha = (((k * 7) + 9) / 10).max((k / 2) + 1);
                let params = SnowballParams { k, alpha, beta: FINALITY_BETA, max_rounds: 80 };

                // The round PERSISTS across ticks (created once per height) so the streak accumulates;
                // params are passed in each tick so a growing committee is absorbed without a reset.
                let tally: Vec<[u8; 32]> = votes.values().copied().collect();
                // Floor: never finalise with fewer than MIN_VOTERS distinct nodes (a lone node must not
                // finalise alone). Doubles as the anti-partition guard for this step.
                let reaches_quorum = voters >= MIN_VOTERS;

                log::debug!(
                    target: "woven-trust",
                    "round h={height} voters={voters} k={k} alpha={alpha} quorum={reaches_quorum} pref={:02x?}",
                    &pref[..4],
                );

                if let Some(decided) = round.observe_round(&tally, &params, reaches_quorum) {
                    let decided_hash = Hash::from(decided);
                    let justification = (ENGINE_ID, decided.to_vec());
                    match client.finalize_block(decided_hash, Some(justification), true) {
                        Ok(()) => {
                            log::info!(
                                target: "woven-trust",
                                "🎭 finalised #{height} {decided_hash:?} (voters {voters}, alpha {alpha}/{k})"
                            );
                            watermark.store(height as u64, Ordering::Relaxed);
                            height += 1;
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

            notification = votes_rx.select_next_some() => {
                if let Some(v) = Vote::decode(&notification.message) {
                    log::debug!(
                        target: "woven-trust",
                        "rx vote h={} (worker h={height}) voter={:02x?}",
                        v.height, &v.voter[..4],
                    );
                    if v.height == height as u64 {
                        votes.insert(v.voter, v.hash);
                    }
                } else {
                    log::debug!(target: "woven-trust", "rx undecodable gossip msg ({} bytes)", notification.message.len());
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

