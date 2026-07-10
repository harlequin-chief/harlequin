//! Service: builds and runs the node (client, network, tx pool, RPC, block authoring).
//! Adapted from the polkadot-sdk minimal template (manual/instant seal). Woven-Trust replaces the
//! authoring engine later (SPEC §2). `harlequin_runtime` is our runtime.

use crate::cli::Consensus;
use futures::FutureExt;
use harlequin_runtime::{interface::OpaqueBlock as Block, RuntimeApi};
use polkadot_sdk::{
    sc_client_api::backend::Backend,
    sc_executor::WasmExecutor,
    sc_service::{error::Error as ServiceError, Configuration, TaskManager},
    sc_telemetry::{Telemetry, TelemetryWorker},
    sc_transaction_pool_api::OffchainTransactionPoolFactory,
    *,
};
use sp_runtime::traits::Block as BlockT;
use std::sync::Arc;

type HostFunctions = sp_io::SubstrateHostFunctions;

pub(crate) type FullClient =
    sc_service::TFullClient<Block, RuntimeApi, WasmExecutor<HostFunctions>>;

type FullBackend = sc_service::TFullBackend<Block>;
type FullSelectChain = sc_consensus::LongestChain<FullBackend, Block>;

pub type Service = sc_service::PartialComponents<
    FullClient,
    FullBackend,
    FullSelectChain,
    sc_consensus::DefaultImportQueue<Block>,
    sc_transaction_pool::TransactionPoolHandle<Block, FullClient>,
    Option<Telemetry>,
>;

pub fn new_partial(config: &Configuration) -> Result<Service, ServiceError> {
    let telemetry = config
        .telemetry_endpoints
        .clone()
        .filter(|x| !x.is_empty())
        .map(|endpoints| -> Result<_, sc_telemetry::Error> {
            let worker = TelemetryWorker::new(16)?;
            let telemetry = worker.handle().new_telemetry(endpoints);
            Ok((worker, telemetry))
        })
        .transpose()?;

    let executor = sc_service::new_wasm_executor(&config.executor);

    let (client, backend, keystore_container, task_manager) =
        sc_service::new_full_parts::<Block, RuntimeApi, _>(
            config,
            telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
            executor,
            Default::default(),
        )?;
    let client = Arc::new(client);

    let telemetry = telemetry.map(|(worker, telemetry)| {
        task_manager.spawn_handle().spawn("telemetry", None, worker.run());
        telemetry
    });

    let select_chain = sc_consensus::LongestChain::new(backend.clone());

    let transaction_pool = Arc::from(
        sc_transaction_pool::Builder::new(
            task_manager.spawn_essential_handle(),
            client.clone(),
            config.role.is_authority().into(),
        )
        .with_options(config.transaction_pool.clone())
        .with_prometheus(config.prometheus_registry())
        .build(),
    );

    // Import queue with Woven-Trust finality verification (step 3b): blocks/justifications carrying a
    // `WTC1` finality proof are verified (≥alpha signed committee votes) before finality is accepted —
    // a syncing node trusts the proof, not the peer. Harmless passthrough for the seal-only modes.
    let import_queue = crate::finality::build_import_queue(
        client.clone(),
        &task_manager.spawn_essential_handle(),
        config.prometheus_registry(),
    );

    Ok(sc_service::PartialComponents {
        client,
        backend,
        task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: telemetry,
    })
}

pub fn new_full<Network: sc_network::NetworkBackend<Block, <Block as BlockT>::Hash>>(
    config: Configuration,
    consensus: Consensus,
    vote_as: Option<String>,
) -> Result<TaskManager, ServiceError> {
    let sc_service::PartialComponents {
        client,
        backend,
        mut task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other: mut telemetry,
    } = new_partial(&config)?;

    let mut net_config = sc_network::config::FullNetworkConfiguration::<
        Block,
        <Block as BlockT>::Hash,
        Network,
    >::new(
        &config.network,
        config.prometheus_config.as_ref().map(|cfg| cfg.registry.clone()),
    );
    let metrics = Network::register_notification_metrics(
        config.prometheus_config.as_ref().map(|cfg| &cfg.registry),
    );

    // Woven-Trust finality gadget: register its gossip notification protocol BEFORE the network starts
    // (same recipe as `sc_consensus_grandpa::grandpa_peers_set_config`). Keep the service to drive the
    // gadget once the network is up. Only the real engine carries finality votes.
    let finality_notif = if matches!(
        consensus,
        Consensus::WovenTrust(_) | Consensus::WovenTrustVoteOnly(_)
    ) {
        let (cfg, service) = crate::finality::protocol_config::<Network>(
            metrics.clone(),
            net_config.peer_store_handle(),
        );
        net_config.add_notification_protocol(cfg);
        Some(service)
    } else {
        None
    };

    // Step 3c — finality-PROOF distribution protocol: registered on EVERY chain-following node (voters AND
    // followers) so verified finality proofs reach non-voting / catching-up nodes. Register before the
    // network starts, like the vote protocol above.
    let proof_notif = if matches!(
        consensus,
        Consensus::WovenTrust(_) | Consensus::WovenTrustVoteOnly(_) | Consensus::Follower
    ) {
        let (cfg, service) = crate::finality::proof_protocol_config::<Network>(
            metrics.clone(),
            net_config.peer_store_handle(),
        );
        net_config.add_notification_protocol(cfg);
        Some(service)
    } else {
        None
    };

    let (network, system_rpc_tx, tx_handler_controller, sync_service) =
        sc_service::build_network(sc_service::BuildNetworkParams {
            config: &config,
            net_config,
            client: client.clone(),
            transaction_pool: transaction_pool.clone(),
            spawn_handle: task_manager.spawn_handle(),
            spawn_essential_handle: task_manager.spawn_essential_handle(),
            import_queue,
            block_announce_validator_builder: None,
            warp_sync_config: None,
            block_relay: None,
            metrics,
        })?;

    // Clones for the finality gadget + proof distributor (the originals are moved into `spawn_tasks` below).
    let gossip_network = network.clone();
    let gossip_sync = sync_service.clone();
    let proof_network = network.clone();
    let proof_sync = sync_service.clone();

    if config.offchain_worker.enabled {
        let offchain_workers =
            sc_offchain::OffchainWorkers::new(sc_offchain::OffchainWorkerOptions {
                runtime_api_provider: client.clone(),
                is_validator: config.role.is_authority(),
                keystore: Some(keystore_container.keystore()),
                offchain_db: backend.offchain_storage(),
                transaction_pool: Some(OffchainTransactionPoolFactory::new(
                    transaction_pool.clone(),
                )),
                network_provider: Arc::new(network.clone()),
                enable_http_requests: true,
                custom_extensions: |_| vec![],
            })?;
        task_manager.spawn_handle().spawn(
            "offchain-workers-runner",
            "offchain-worker",
            offchain_workers.run(client.clone(), task_manager.spawn_handle()).boxed(),
        );
    }

    let rpc_extensions_builder = {
        let client = client.clone();
        let pool = transaction_pool.clone();

        Box::new(move |_| {
            let deps = crate::rpc::FullDeps { client: client.clone(), pool: pool.clone() };
            crate::rpc::create_full(deps).map_err(Into::into)
        })
    };

    let prometheus_registry = config.prometheus_registry().cloned();

    let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
        network,
        client: client.clone(),
        keystore: keystore_container.keystore(),
        task_manager: &mut task_manager,
        transaction_pool: transaction_pool.clone(),
        rpc_builder: rpc_extensions_builder,
        backend,
        system_rpc_tx,
        tx_handler_controller,
        sync_service,
        config,
        telemetry: telemetry.as_mut(),
        tracing_execute_block: None,
    })?;

    // Drive the Woven-Trust finality gadget: the committee runs the Snowball vote over the candidate
    // blocks and finalises by quorum (replacing the trivial finality of the authoring loop). Two
    // essential tasks: the worker (votes + finalises) and the gossip-engine driver.
    let finality_block_time = match &consensus {
        Consensus::WovenTrust(t) | Consensus::WovenTrustVoteOnly(t) => Some(*t),
        _ => None,
    };
    // This node's committee identity (sr25519 public of `--vote-as`) — used by the slot-leader
    // election (only the leader authors) AND declared as the author in the participation inherent
    // (F2 piece 6c). Computed once, BEFORE `finality::start` moves `vote_as`.
    let my_pub: Option<[u8; 32]> = {
        use sp_core::Pair as _;
        vote_as
            .as_deref()
            .and_then(|s| sp_core::sr25519::Pair::from_string(s, None).ok())
            .map(|p| p.public().0)
    };
    // P-1/#838: keep the `--vote-as` KEYPAIR (not just the public) so the participation inherent can SIGN
    // each block's authorship proof (`author_message`). Cloned into each authoring path's inherent provider;
    // a keyless node signs nothing and the pallet credits it as nobody.
    let my_vote_pair: Option<sp_core::sr25519::Pair> = {
        use sp_core::Pair as _;
        vote_as
            .as_deref()
            .and_then(|s| sp_core::sr25519::Pair::from_string(s, None).ok())
    };
    if let (Some(notification_service), Some(block_time)) = (finality_notif, finality_block_time) {
        let (worker, driver) = crate::finality::start(
            client.clone(),
            gossip_network,
            gossip_sync,
            notification_service,
            vote_as,
            block_time,
            prometheus_registry.as_ref(),
        );
        task_manager
            .spawn_essential_handle()
            .spawn("woven-trust-finality", None, worker);
        task_manager
            .spawn_essential_handle()
            .spawn("woven-trust-finality-gossip", None, driver);
    }

    // Step 3c — finality-proof distributor: runs on EVERY chain-following node (voters re-broadcast their
    // latest proof; followers receive, verify and finalise from it). Followers have no `<ms>` of their own,
    // so they re-broadcast/poll on a default cadence.
    let proof_round_ms = match &consensus {
        Consensus::WovenTrust(t) | Consensus::WovenTrustVoteOnly(t) => Some(*t),
        Consensus::Follower => Some(12000),
        _ => None,
    };
    if let (Some(notification_service), Some(round_ms)) = (proof_notif, proof_round_ms) {
        let (worker, driver) = crate::finality::start_proof_distribution(
            client.clone(),
            proof_network,
            proof_sync,
            notification_service,
            round_ms,
            prometheus_registry.as_ref(),
        );
        task_manager
            .spawn_essential_handle()
            .spawn("woven-trust-proof-dist", None, worker);
        task_manager
            .spawn_essential_handle()
            .spawn("woven-trust-proof-gossip", None, driver);
    }

    let proposer = sc_basic_authorship::ProposerFactory::new(
        task_manager.spawn_handle(),
        client.clone(),
        transaction_pool.clone(),
        prometheus_registry.as_ref(),
        telemetry.as_ref().map(|x| x.handle()),
    );

    match consensus {
        Consensus::InstantSeal => {
            let inherent_client = client.clone();
            let inherent_vote_pair = my_vote_pair.clone();
            let params = sc_consensus_manual_seal::InstantSealParams {
                block_import: client.clone(),
                env: proposer,
                client,
                pool: transaction_pool,
                select_chain,
                consensus_data_provider: None,
                create_inherent_data_providers: move |parent, ()| {
                    let client = inherent_client.clone();
                    let vote_pair = inherent_vote_pair.clone();
                    async move {
                        // MANDATORY participation inherent (F2 piece 6): every block carries its record.
                        let participation =
                            crate::participation_inherent::provider_for(&client, parent, vote_pair.as_ref());
                        Ok((sp_timestamp::InherentDataProvider::from_system_time(), participation))
                    }
                },
            };

            let authorship_future = sc_consensus_manual_seal::run_instant_seal(params);

            task_manager.spawn_essential_handle().spawn_blocking(
                "instant-seal",
                None,
                authorship_future,
            );
        },
        Consensus::ManualSeal(block_time) => {
            let (mut sink, commands_stream) = futures::channel::mpsc::channel(1024);
            task_manager.spawn_handle().spawn("block_authoring", None, async move {
                loop {
                    futures_timer::Delay::new(std::time::Duration::from_millis(block_time)).await;
                    // Don't panic in the authoring loop (audit): if the seal engine is gone
                    // (node shutting down) stop cleanly; if its channel is momentarily full, skip this slot.
                    if let Err(e) = sink.try_send(sc_consensus_manual_seal::EngineCommand::SealNewBlock {
                        create_empty: true,
                        finalize: true,
                        parent_hash: None,
                        sender: None,
                    }) {
                        if e.is_disconnected() {
                            break;
                        }
                    }
                }
            });

            let inherent_client = client.clone();
            let inherent_vote_pair = my_vote_pair.clone();
            let params = sc_consensus_manual_seal::ManualSealParams {
                block_import: client.clone(),
                env: proposer,
                client,
                pool: transaction_pool,
                select_chain,
                commands_stream: Box::pin(commands_stream),
                consensus_data_provider: None,
                create_inherent_data_providers: move |parent, ()| {
                    let client = inherent_client.clone();
                    let vote_pair = inherent_vote_pair.clone();
                    async move {
                        let participation =
                            crate::participation_inherent::provider_for(&client, parent, vote_pair.as_ref());
                        Ok((sp_timestamp::InherentDataProvider::from_system_time(), participation))
                    }
                },
            };
            let authorship_future = sc_consensus_manual_seal::run_manual_seal(params);

            task_manager.spawn_essential_handle().spawn_blocking(
                "manual-seal",
                None,
                authorship_future,
            );
        },
        Consensus::WovenTrust(block_time) => {
            use harlequin_consensus_api::HarlequinConsensusApi;
            use sp_api::ProvideRuntimeApi;
            use sp_blockchain::HeaderBackend;

            let (mut sink, commands_stream) = futures::channel::mpsc::channel(1024);
            let api_client = client.clone();
            // Step 2 — sortition off REAL on-chain reputation. Each slot the node reads the chain's
            // consensus reputation (`HarlequinConsensusApi`) and runs the reputation-weighted sortition;
            // it authors only when the sortition elects a committee for the slot. On a fresh chain with
            // no reputation yet it falls back to a lone dev identity so the devnet still produces blocks.
            // The per-author identity check (session keys) and the Snowball finality gadget are next.
            task_manager.spawn_handle().spawn("woven-trust-authoring", None, async move {
                use std::collections::BTreeMap;
                let station = String::from("station");
                // Authoring committee size (sortition τ). Production τ=60 (`PARAMETERS.md` ♠, = 3·k);
                // small on testnet. Selected by the `mainnet` feature (#30).
                #[cfg(feature = "mainnet")]
                let tau: u32 = 60;
                #[cfg(not(feature = "mainnet"))]
                let tau: u32 = 3;
                // This node's vote identity (computed once above), used to elect a single slot leader
                // so exactly ONE node authors per slot. A node with no vote key never leads (and so
                // never authors on a live chain) — it only follows.
                let mut last_slot: u64 = 0;
                loop {
                    futures_timer::Delay::new(std::time::Duration::from_millis(block_time)).await;
                    // WALL-CLOCK slot (Aura-style): every node derives the SAME slot number from real
                    // time, so the single leader (below) is globally agreed and exactly ONE block is
                    // produced per `block_time`. A per-node counter would let offset nodes each march
                    // their own slot stream and author in parallel — defeating single-leader + re-forking.
                    let slot = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0)
                        / block_time.max(1);
                    if slot == last_slot {
                        continue; // this slot already handled (poll/clock jitter) — never author it twice
                    }
                    last_slot = slot;

                    // Read the chain's consensus reputation at the current best block.
                    let at = api_client.info().best_hash;
                    let onchain = api_client
                        .runtime_api()
                        .consensus_reputation(at)
                        .unwrap_or_default();

                    let mut reputation: BTreeMap<String, i128> = BTreeMap::new();
                    let mut keys: BTreeMap<String, String> = BTreeMap::new();
                    let using_chain = !onchain.is_empty();
                    let seed;
                    if using_chain {
                        // §2.1 anti-grinding: once the commit–reveal beacon is populated, draw off it —
                        // candidate key = its committed secret (not `sk-{id}`), seed = beacon ⊕ slot (the
                        // slot keeps per-slot rotation; the beacon anchor makes it un-grindable). Before the
                        // pipeline fills (genesis/bootstrap) fall back to the reputation-only draw (no halt).
                        let committed = api_client
                            .runtime_api()
                            .beacon_committed_keys(at)
                            .unwrap_or_default();
                        if committed.is_empty() {
                            for (acc, rep) in &onchain {
                                let id = hex32(acc);
                                keys.insert(id.clone(), format!("sk-{id}"));
                                reputation.insert(id, *rep);
                            }
                            seed = format!("slot{slot}");
                        } else {
                            let kmap: BTreeMap<[u8; 32], String> = committed.into_iter().collect();
                            for (acc, rep) in &onchain {
                                if let Some(k) = kmap.get(acc) {
                                    let id = hex32(acc);
                                    keys.insert(id.clone(), k.clone());
                                    reputation.insert(id, *rep);
                                }
                            }
                            let beacon =
                                api_client.runtime_api().beacon_seed(at).unwrap_or_default();
                            seed = format!("{}-slot{slot}", consensus_core::beacon::hex(&beacon));
                        }
                    } else {
                        // Bootstrap fallback: a lone dev node so a fresh `--dev` chain still seals.
                        reputation.insert(station.clone(), 1_000_000_000);
                        keys.insert(station.clone(), String::from("sk-station-dev"));
                        seed = format!("slot{slot}");
                    }

                    let committee =
                        consensus_core::elect_committee_fp(&reputation, &keys, &seed, tau);
                    // SINGLE LEADER per slot. Of the elected committee, the member minimising
                    // `H(seed | id)` is the slot leader and the ONLY one that authors. Every node derives
                    // the same committee + leader from the AGREED seed (beacon ⊕ slot, or `slot{n}` before
                    // the beacon fills), so exactly one block is produced per slot — no per-slot forks.
                    //   Liveness: the seed rotates every slot, so the leader rotates; a down leader only
                    //   skips ITS slots and the chain produces on every slot whose leader is up. No single
                    //   node owns all slots, so offline nodes cannot freeze production. (Finality then
                    //   tolerates up to ⌊(1-0.70)·k⌋ faults via the alpha quorum: at k=5, alpha=4 → 1 down
                    //   still finalises, 2 down pauses finality safely and resumes when a node returns.)
                    //   Safety: the leader is a deterministic function of agreed state only, identical on
                    //   every node — no ambiguity over who authors.
                    let elected = if using_chain {
                        let leader = committee
                            .keys()
                            .min_by_key(|id| sp_core::blake2_256(format!("{seed}|{id}").as_bytes()));
                        // This node's committee identity: a session key delegates for a founder account
                        // (genesis voteKeys); fall back to the raw key if there is no delegation.
                        let my_id = my_pub.map(|pk| {
                            let rev = api_client.runtime_api().vote_keys(at).unwrap_or_default();
                            rev.into_iter()
                                .find(|(_, sess)| *sess == pk)
                                .map(|(acc, _)| hex32(&acc))
                                .unwrap_or_else(|| hex32(&pk))
                        });
                        match (leader, my_id.as_deref()) {
                            (Some(l), Some(me)) => l.as_str() == me,
                            _ => false,
                        }
                    } else {
                        committee.contains_key(&station)
                    };
                    if !elected {
                        continue; // not elected this slot -> do not author
                    }
                    if let Err(e) =
                        sink.try_send(sc_consensus_manual_seal::EngineCommand::SealNewBlock {
                            create_empty: true,
                            // Finality is owned by the Woven-Trust gadget now, not the authoring loop.
                            finalize: false,
                            parent_hash: None,
                            sender: None,
                        })
                    {
                        if e.is_disconnected() {
                            break;
                        }
                    }
                }
            });

            let inherent_client = client.clone();
            let inherent_vote_pair = my_vote_pair.clone();
            let params = sc_consensus_manual_seal::ManualSealParams {
                block_import: client.clone(),
                env: proposer,
                client,
                pool: transaction_pool,
                select_chain,
                commands_stream: Box::pin(commands_stream),
                consensus_data_provider: None,
                create_inherent_data_providers: move |parent, ()| {
                    let client = inherent_client.clone();
                    let vote_pair = inherent_vote_pair.clone();
                    async move {
                        // F2 piece 6c: the author's record — identity + the canonical finality votes
                        // since the on-chain cursor (never gossip; the runtime re-verifies anyway).
                        let participation =
                            crate::participation_inherent::provider_for(&client, parent, vote_pair.as_ref());
                        Ok((sp_timestamp::InherentDataProvider::from_system_time(), participation))
                    }
                },
            };
            let authorship_future = sc_consensus_manual_seal::run_manual_seal(params);

            task_manager.spawn_essential_handle().spawn_blocking(
                "woven-trust",
                None,
                authorship_future,
            );
        },
        Consensus::WovenTrustVoteOnly(_) => {
            // No authoring: this node follows the chain (sync) and only runs the finality gadget
            // (spawned above). Lets a devnet add finality voters without each one forking by authoring.
        },
        Consensus::Follower => {
            // No authoring AND no finality gadget (neither registered above nor spawned). This node only
            // syncs blocks; it accepts finality EXCLUSIVELY via the import queue's verified justification
            // import (step 3b). It finalises a block only when a peer serves a sound signed-vote proof.
        },
    }

    Ok(task_manager)
}

/// Lowercase hex of a 32-byte account id — a stable per-account string id for the sortition map.
pub(crate) fn hex32(b: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for &byte in b {
        s.push(HEX[(byte >> 4) as usize] as char);
        s.push(HEX[(byte & 0x0f) as usize] as char);
    }
    s
}
