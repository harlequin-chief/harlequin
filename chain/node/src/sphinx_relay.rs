//! Messenger relay `/hlq/sphinx/1` — Corro phase M0 ().
//!
//! Every node can carry fixed-size onion packets for others, like a Tor relay, but with no exit to the
//! internet and no new port: it is one more notification protocol on the p2p connection the node already has.
//! The onion is `sphinx-packet` (Nym's audited implementation, pinned `=0.6.0`, same x25519/curve25519/digest
//! versions the node already links). Nothing here is home-made cryptography.
//!
//! What a relay learns: the previous hop (the peer that handed it the packet) and the next hop. Never the
//! origin, the destination or its position in the path. What it logs: "packet in / packet out, N bytes" and
//! nothing else — no peer ids, no addresses (zero persistence, M0 rule).
//!
//! Transport: REQUEST-RESPONSE, not a notification substream. The first version used a notification protocol and on
//! the test chain only 1-4 of the 5 connected neighbours ever opened it (the per-protocol peer set fills slowly and
//! `open_substream` is unimplemented in both backends). A request goes to any peer the node is already connected to,
//! like a datagram: `HELLO` returns the peer's relay key, `PACKET` is acknowledged at once and then processed.
//! Keys: an ephemeral x25519 secret per run. Node address on the onion = blake2-256 of the libp2p PeerId bytes.
//! M0 limitation, said out loud: keys travel only by HELLO between connected peers, so a path can only use relays the SENDER is
//! connected to, and each hop must also be connected to the next one (otherwise that relay drops the packet,
//! silently for the sender). Fine on the full-mesh test chain; a key directory on chain is for M1+.
//!
//! OFF by default (`--sphinx-relay`): production nodes do not open the protocol until it has passed the test
//! chain. `--sphinx-probe-secs N` makes the node send itself a probe through 3 random relays every N s, so
//! the test chain can show a packet crossing 3 hops (origin and final hop log the same probe tag).

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use futures::{future::BoxFuture, stream::FuturesUnordered, FutureExt, StreamExt};
use log::{debug, info, warn};
use polkadot_sdk::{
    sc_network::{
        self,
        request_responses::{IncomingRequest, OutgoingResponse},
        IfDisconnected,
        service::traits::NetworkService,
        NetworkBackend,
    },
    sc_network_sync::SyncingService,
    sc_network_types::PeerId,
};
use sphinx_packet::{
    header::delays::Delay,
    packet::{ProcessedPacket, ProcessedPacketData, SphinxPacket},
    route::{Destination, DestinationAddressBytes, Node, NodeAddressBytes},
};
use x25519_dalek::{PublicKey, StaticSecret};

use harlequin_runtime::interface::OpaqueBlock as Block;
use sp_runtime::traits::Block as BlockT;
type Hash = <Block as BlockT>::Hash;

/// What the command line asked for.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub enabled: bool,
    pub probe_secs: Option<u64>,
}

/// Protocol name. Versioned: a future change of packet format gets `/hlq/sphinx/2`.
pub const PROTOCOL_NAME: &str = "/hlq/sphinx/1";
/// HELLO response = format version byte + 32-byte x25519 public key.
const HANDSHAKE_VERSION: u8 = 1;
/// Request kinds (first byte).
const REQ_HELLO: u8 = 0;
const REQ_PACKET: u8 = 1;
/// A sphinx packet with the default 1024-byte payload is a little over 1.3 KB; leave room, refuse anything bigger.
const MAX_REQUEST: u64 = 4 * 1024;
const MAX_RESPONSE: u64 = 64;
/// Path length for probes (the M0 VERIFY asks for 3 relays).
const HOPS: usize = 3;
/// Replay cache: shared secrets of packets already processed. Bounded; oldest forgotten first.
const REPLAY_CAP: usize = 65_536;
/// How often the relay re-reads who it is connected to and greets the newcomers.
const REFRESH: Duration = Duration::from_secs(20);

const TARGET: &str = "hlq-sphinx";

/// Address of a node on the onion: blake2-256 of its PeerId bytes (32 bytes = `NODE_ADDRESS_LENGTH`).
fn node_address(peer: &PeerId) -> [u8; 32] {
    sp_core::hashing::blake2_256(&peer.to_bytes())
}

fn handshake_bytes(public: &PublicKey) -> Vec<u8> {
    let mut v = Vec::with_capacity(33);
    v.push(HANDSHAKE_VERSION);
    v.extend_from_slice(public.as_bytes());
    v
}

fn parse_handshake(h: &[u8]) -> Option<PublicKey> {
    if h.len() != 33 || h[0] != HANDSHAKE_VERSION {
        return None;
    }
    let mut k = [0u8; 32];
    k.copy_from_slice(&h[1..]);
    Some(PublicKey::from(k))
}

/// The relay's own key, created once per run.
pub struct RelayKey {
    secret: StaticSecret,
    public: PublicKey,
}

impl RelayKey {
    pub fn ephemeral() -> Self {
        let fresh = StaticSecret::random_from_rng(rand::rngs::OsRng);
        let public = PublicKey::from(&fresh);
        Self { secret: fresh, public }
    }
}

/// Register the protocol. Call BEFORE `build_network`. Returns the config to add with
/// `net_config.add_request_response_protocol` and the queue the relay task reads.
pub fn protocol_config<N: NetworkBackend<Block, Hash>>(
) -> (N::RequestResponseProtocolConfig, async_channel::Receiver<IncomingRequest>) {
    let (tx, rx) = async_channel::bounded(1024);
    let cfg = N::request_response_config(
        PROTOCOL_NAME.into(),
        Vec::new(),
        MAX_REQUEST,
        MAX_RESPONSE,
        Duration::from_secs(10),
        Some(tx),
    );
    (cfg, rx)
}

/// Bounded set of already-seen packets (by their shared secret): a replayed packet is dropped, so an observer
/// cannot re-inject a captured packet to watch where it goes.
struct ReplayCache {
    seen: HashSet<[u8; 32]>,
    order: VecDeque<[u8; 32]>,
}

impl ReplayCache {
    fn new() -> Self {
        Self { seen: HashSet::new(), order: VecDeque::new() }
    }
    /// `true` if new (and remembers it), `false` if already seen.
    fn insert(&mut self, tag: [u8; 32]) -> bool {
        if !self.seen.insert(tag) {
            return false;
        }
        self.order.push_back(tag);
        if self.order.len() > REPLAY_CAP {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

/// Results of the requests this relay has in flight.
enum Done {
    Hello(PeerId, Option<PublicKey>),
    Sent(bool),
    Peers(Vec<PeerId>),
}

struct Relay {
    key: RelayKey,
    network: Arc<dyn NetworkService>,
    /// Neighbours whose relay key we know: onion address -> (peer, key).
    peers: HashMap<[u8; 32], (PeerId, PublicKey)>,
    by_peer: HashMap<PeerId, [u8; 32]>,
    /// Greeted and still waiting for an answer (no double HELLO).
    asking: HashSet<PeerId>,
    replay: ReplayCache,
    /// Requests to launch; the task loop moves them to its own `FuturesUnordered` (a field here would keep `relay`
    /// borrowed inside `select!`).
    outbox: Vec<BoxFuture<'static, Done>>,
}

impl Relay {
    fn request(&self, peer: PeerId, payload: Vec<u8>) -> BoxFuture<'static, Result<Vec<u8>, ()>> {
        let net = self.network.clone();
        async move {
            net.request(peer.into(), PROTOCOL_NAME.into(), payload, None, IfDisconnected::ImmediateError)
                .await
                .map(|(resp, _)| resp)
                .map_err(|_| ())
        }
        .boxed()
    }

    fn send_packet(&mut self, peer: PeerId, packet: Vec<u8>) {
        let mut payload = Vec::with_capacity(packet.len() + 1);
        payload.push(REQ_PACKET);
        payload.extend_from_slice(&packet);
        let fut = self.request(peer, payload);
        self.outbox.push(async move { Done::Sent(fut.await.is_ok()) }.boxed());
    }

    /// The connected set changed: forget the gone, greet the new.
    fn on_peers(&mut self, connected: Vec<PeerId>) {
        let now: HashSet<PeerId> = connected.iter().copied().collect();
        let gone: Vec<PeerId> = self.by_peer.keys().filter(|p| !now.contains(p)).copied().collect();
        for p in gone {
            if let Some(a) = self.by_peer.remove(&p) {
                self.peers.remove(&a);
            }
        }
        for p in connected {
            if self.by_peer.contains_key(&p) || !self.asking.insert(p) {
                continue;
            }
            let fut = self.request(p, alloc_hello());
            self.outbox.push(async move { Done::Hello(p, fut.await.ok().and_then(|r| parse_handshake(&r))) }.boxed());
        }
    }

    fn on_done(&mut self, d: Done) {
        match d {
            Done::Hello(peer, key) => {
                self.asking.remove(&peer);
                if let Some(pk) = key {
                    let addr = node_address(&peer);
                    self.peers.insert(addr, (peer, pk));
                    self.by_peer.insert(peer, addr);
                    info!(target: TARGET, "relay: neighbour known, {} with the relay", self.peers.len());
                }
            },
            Done::Sent(ok) =>
                if !ok {
                    info!(target: TARGET, "packet out: the next hop did not take it");
                },
            Done::Peers(list) => self.on_peers(list),
        }
    }

    fn on_request(&mut self, req: IncomingRequest) {
        let IncomingRequest { payload, pending_response, .. } = req;
        let answer = |result: Result<Vec<u8>, ()>| {
            let _ = pending_response.send(OutgoingResponse {
                result,
                reputation_changes: Vec::new(),
                sent_feedback: None,
            });
        };
        match payload.first() {
            Some(&REQ_HELLO) if payload.len() == 1 => answer(Ok(handshake_bytes(&self.key.public))),
            Some(&REQ_PACKET) => {
                answer(Ok(Vec::new())); // acknowledge at once: the sender learns only "taken"
                self.on_packet(&payload[1..]);
            },
            _ => answer(Err(())),
        }
    }

    fn on_packet(&mut self, bytes: &[u8]) {
        let size = bytes.len();
        let packet = match SphinxPacket::from_bytes(bytes) {
            Ok(p) => p,
            Err(_) => {
                debug!(target: TARGET, "packet in: malformed, {size} bytes, dropped");
                return;
            },
        };
        if !self.replay.insert(*packet.shared_secret().as_bytes()) {
            info!(target: TARGET, "packet in: replay, {size} bytes, dropped");
            return;
        }
        let processed: ProcessedPacket = match packet.process(&self.key.secret) {
            Ok(p) => p,
            Err(_) => {
                debug!(target: TARGET, "packet in: not for my key, {size} bytes, dropped");
                return;
            },
        };
        match processed.data {
            ProcessedPacketData::ForwardHop { next_hop_packet, next_hop_address, .. } => {
                // M0: no mixing delay yet (that is M4). The relay knows only the next hop.
                let next = next_hop_address.to_bytes();
                match self.peers.get(&next).map(|(p, _)| *p) {
                    Some(peer) => {
                        let out = next_hop_packet.to_bytes();
                        info!(target: TARGET, "packet in {size} B, out {} B", out.len());
                        self.send_packet(peer, out);
                    },
                    None => info!(target: TARGET, "packet in {size} B, next hop not connected, dropped"),
                }
            },
            ProcessedPacketData::FinalHop { payload, .. } => match payload.recover_plaintext() {
                Ok(msg) => {
                    let tag = hex_tag(&msg);
                    info!(target: TARGET, "packet delivered here, {size} B, probe {tag}");
                },
                Err(_) => info!(target: TARGET, "packet delivered here, {size} B, payload unreadable"),
            },
        }
    }

    /// Send a probe to ourselves through `HOPS` distinct neighbours: me -> r1 -> r2 -> r3 (final).
    /// The final hop is the third relay; origin and final hop log the same probe tag.
    fn probe(&mut self) {
        use rand::seq::IteratorRandom;
        if self.peers.len() < HOPS {
            info!(target: TARGET, "probe skipped: {} neighbours with the relay, need {HOPS}", self.peers.len());
            return;
        }
        let mut rng = rand::thread_rng();
        let hops: Vec<([u8; 32], PeerId, PublicKey)> = self
            .peers
            .iter()
            .choose_multiple(&mut rng, HOPS)
            .into_iter()
            .map(|(a, (p, k))| (*a, *p, *k))
            .collect();
        let route: Vec<Node> =
            hops.iter().map(|(a, _, k)| Node::new(NodeAddressBytes::from_bytes(*a), *k)).collect();
        let last = hops[HOPS - 1].0;
        let destination = Destination::new(DestinationAddressBytes::from_bytes(last), [0u8; 16]);
        let delays = vec![Delay::new_from_nanos(0); HOPS];
        let mut nonce = [0u8; 16];
        rand::RngCore::fill_bytes(&mut rng, &mut nonce);
        let mut msg = b"hlq-probe:".to_vec();
        msg.extend_from_slice(&nonce);
        let packet = match SphinxPacket::new(msg.clone(), &route, &destination, &delays) {
            Ok(p) => p,
            Err(e) => {
                warn!(target: TARGET, "probe: cannot build packet: {e:?}");
                return;
            },
        };
        let bytes = packet.to_bytes();
        info!(target: TARGET, "probe sent, {} B, {HOPS} hops, probe {}", bytes.len(), hex_tag(&msg));
        self.send_packet(hops[0].1, bytes);
    }
}

fn alloc_hello() -> Vec<u8> {
    vec![REQ_HELLO]
}

/// Short public tag of a probe message, so two logs can be matched without printing the message.
fn hex_tag(msg: &[u8]) -> String {
    let h = sp_core::hashing::blake2_128(msg);
    h[..6].iter().map(|b| format!("{b:02x}")).collect()
}

/// The relay task. Spawned NOT essential: if it stops, the node keeps following and sealing the chain.
pub async fn run(
    key: RelayKey,
    network: Arc<dyn NetworkService>,
    sync: Arc<SyncingService<Block>>,
    inbound: async_channel::Receiver<IncomingRequest>,
    probe_secs: Option<u64>,
) {
    let mut relay = Relay {
        key,
        network,
        peers: HashMap::new(),
        by_peer: HashMap::new(),
        asking: HashSet::new(),
        replay: ReplayCache::new(),
        outbox: Vec::new(),
    };
    let mut inflight: FuturesUnordered<BoxFuture<'static, Done>> = FuturesUnordered::new();
    info!(
        target: TARGET,
        "messenger relay on ({PROTOCOL_NAME}, request-response); probes {}",
        probe_secs.map(|s| format!("every {s} s")).unwrap_or_else(|| "off".into())
    );
    let ask_peers = |sync: Arc<SyncingService<Block>>| -> BoxFuture<'static, Done> {
        async move {
            let list = sync.peers_info().await.map(|v| v.into_iter().map(|(p, _)| p).collect()).unwrap_or_default();
            Done::Peers(list)
        }
        .boxed()
    };
    inflight.push(ask_peers(sync.clone()));
    let mut refresh = futures_timer::Delay::new(REFRESH).fuse();
    let new_tick = || -> BoxFuture<'static, ()> {
        match probe_secs {
            Some(s) if s > 0 => futures_timer::Delay::new(Duration::from_secs(s)).boxed(),
            _ => futures::future::pending().boxed(),
        }
    };
    let mut tick = new_tick().fuse();
    loop {
        futures::select! {
            req = inbound.recv().fuse() => match req {
                Ok(req) => relay.on_request(req),
                Err(_) => { warn!(target: TARGET, "request queue closed; relay stops"); return; },
            },
            done = inflight.select_next_some() => relay.on_done(done),
            _ = refresh => {
                inflight.push(ask_peers(sync.clone()));
                refresh = futures_timer::Delay::new(REFRESH).fuse();
            },
            _ = tick => {
                relay.probe();
                tick = new_tick().fuse();
            },
        }
        inflight.extend(relay.outbox.drain(..));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The packet the node builds crosses three keys in order and the last one reads the probe — the same
    /// path `on_packet` takes, without the network.
    #[test]
    fn three_hops_end_to_end() {
        let keys: Vec<RelayKey> = (0..3).map(|_| RelayKey::ephemeral()).collect();
        let addrs: Vec<[u8; 32]> = (0..3u8).map(|i| [i; 32]).collect();
        let route: Vec<Node> = keys
            .iter()
            .zip(&addrs)
            .map(|(k, a)| Node::new(NodeAddressBytes::from_bytes(*a), k.public))
            .collect();
        let dest = Destination::new(DestinationAddressBytes::from_bytes(addrs[2]), [0u8; 16]);
        let msg = b"hlq-probe:0123456789abcdef".to_vec();
        let mut bytes =
            SphinxPacket::new(msg.clone(), &route, &dest, &vec![Delay::new_from_nanos(0); 3]).unwrap().to_bytes();
        for (i, k) in keys.iter().enumerate() {
            let p = SphinxPacket::from_bytes(&bytes).unwrap().process(&k.secret).unwrap();
            match p.data {
                ProcessedPacketData::ForwardHop { next_hop_packet, next_hop_address, .. } => {
                    assert!(i < 2, "hop {i} should be final");
                    assert_eq!(next_hop_address.to_bytes(), addrs[i + 1], "relay {i} must point to relay {}", i + 1);
                    let out = next_hop_packet.to_bytes();
                    assert_eq!(out.len(), bytes.len(), "fixed size: a relay cannot tell its position by length");
                    bytes = out;
                },
                ProcessedPacketData::FinalHop { payload, .. } => {
                    assert_eq!(i, 2);
                    assert_eq!(payload.recover_plaintext().unwrap(), msg);
                    return;
                },
            }
        }
        panic!("never reached the final hop");
    }

    /// A relay holding the WRONG key cannot open the layer (negative control).
    #[test]
    fn wrong_key_cannot_open() {
        let right = RelayKey::ephemeral();
        let wrong = RelayKey::ephemeral();
        let route = vec![Node::new(NodeAddressBytes::from_bytes([7; 32]), right.public)];
        let dest = Destination::new(DestinationAddressBytes::from_bytes([7; 32]), [0u8; 16]);
        let bytes = SphinxPacket::new(b"x".to_vec(), &route, &dest, &[Delay::new_from_nanos(0)]).unwrap().to_bytes();
        assert!(SphinxPacket::from_bytes(&bytes).unwrap().process(&wrong.secret).is_err());
    }

    #[test]
    fn replay_is_dropped() {
        let mut c = ReplayCache::new();
        assert!(c.insert([1; 32]));
        assert!(!c.insert([1; 32]));
    }

    #[test]
    fn handshake_roundtrip_and_rejects_garbage() {
        let k = RelayKey::ephemeral();
        assert_eq!(parse_handshake(&handshake_bytes(&k.public)).unwrap().as_bytes(), k.public.as_bytes());
        assert!(parse_handshake(&[9u8; 33]).is_none());
        assert!(parse_handshake(&[1u8; 5]).is_none());
    }
}
