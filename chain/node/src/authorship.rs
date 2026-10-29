//! Slot leadership and the authorship seal (L6-01, candidate 9).
//!
//! WHY. Until candidate 8 the single-leader rule lived only in the AUTHOR: each node elected the slot leader
//! from its own copy of the state and stayed quiet when it was not the leader, while the import verifier
//! accepted any block (`LongestChain`, no check). The rule held only because every node ran our binary. A
//! modified binary could seal in every slot, out of turn, and re-organise the unfinalised tip at will
//! (finality itself was never at risk: it needs ≥alpha committee signatures).
//!
//! WHAT. The leader rule is now ONE function, [`slot_leader`], used by both sides:
//!   - the author asks it whether this node leads the slot, puts the slot in a pre-runtime digest and seals
//!     the header with its session key ([`SlotClaims`] is the manual-seal digest provider that does it);
//!   - the import verifier ([`check_header`]) reads the slot back, re-runs [`slot_leader`] on the PARENT's
//!     state, and rejects the block unless the seal is the leader's signature over the pre-seal header hash,
//!     the slot is later than the parent's, and the slot is not in the future.
//! Same pattern as Aura (pre-digest + seal, leader from the parent's state), with our leader rule.
//!
//! Compatibility: blocks sealed by candidate ≤8 carry no claim and are rejected by this verifier once the
//! chain has bound nodes, so candidate 9 starts on a NEW genesis (it is a new rehearsal chain anyway).
//! A chain with no bound node and no reputation (a lone `--dev` chain) keeps the old behaviour: one dev
//! identity authors, nothing to check.

use crate::service::FullClient;
use harlequin_consensus_api::HarlequinConsensusApi;
use harlequin_runtime::interface::OpaqueBlock as Block;
use polkadot_sdk::{sc_consensus::BlockImportParams, sc_consensus_manual_seal, sp_blockchain::HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_core::{sr25519, Pair, H256};
use sp_runtime::{
    traits::{Block as BlockT, Header as HeaderT},
    ConsensusEngineId, Digest, DigestItem,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

/// Engine id of the slot claim (pre-runtime digest: the slot, 8 bytes LE) and of the seal (64-byte sr25519).
pub const ENGINE_ID: ConsensusEngineId = *b"HLQA";

/// Domain of the authorship signature. The session key also signs finality votes; a distinct prefix makes
/// sure no vote can ever be replayed as a seal or the other way round.
const SEAL_DOMAIN: &[u8] = b"harlequin/author-seal/v1";

/// How many slots ahead of this node's clock a block may claim (clock drift between honest nodes).
pub const MAX_FUTURE_SLOTS: u64 = 2;

/// Authoring committee size (sortition τ). Production τ=60 (`PARAMETERS.md` ♠, = 3·k); small on testnet.
#[cfg(feature = "mainnet")]
pub const TAU: u32 = 60;
#[cfg(not(feature = "mainnet"))]
pub const TAU: u32 = 3;

/// The network's slot length in ms. Set once by `new_full` from `--consensus` (followers use 12 000, the
/// mainnet default); the verifier only reads it to refuse blocks from the future.
static SLOT_MS: AtomicU64 = AtomicU64::new(12_000);

/// Record the slot length this node runs with (called once at start-up).
pub fn set_slot_ms(ms: u64) {
    SLOT_MS.store(ms.max(1), Ordering::Relaxed);
}

/// The wall-clock slot (Aura-style): every node derives the same number from real time.
pub fn current_slot(slot_ms: u64) -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        / slot_ms.max(1)
}

/// Who may author a slot, decided from the state at one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leadership {
    /// The elected leader: its committee account and the session key that must sign the seal.
    Leader {
        /// The committee account that won the slot.
        account: [u8; 32],
        /// Its bound session key (VoteKeys), which signs the seal.
        session: [u8; 32],
    },
    /// Nobody leads this slot (no bound node visible, empty committee, or a runtime call failed).
    Nobody,
    /// A lone development chain (no reputation and no bound node): the dev identity authors, unchecked.
    LoneDev,
}

/// The single slot-leader rule, shared by the author and the import verifier. Pure function of the state at
/// `at` and the slot number, so every honest node derives the same answer.
pub fn slot_leader(client: &FullClient, at: H256, slot: u64) -> Leadership {
    let api = client.runtime_api();
    let onchain = match api.consensus_reputation(at) {
        Ok(v) => v,
        Err(_) => return Leadership::Nobody,
    };
    let vote_keys = match api.vote_keys(at) {
        Ok(v) => v,
        Err(_) => return Leadership::Nobody,
    };
    // 10-oct-2026: only accounts with a node bound (VoteKeys owners) may LEAD a slot — the same rule as the
    // finality committee. A reputable mask without a node won slots it could never fill (blocks went from
    // ~12 s to ~40 s on the test chain).
    let session_of: BTreeMap<[u8; 32], [u8; 32]> = vote_keys.into_iter().collect();
    let had_reputation = !onchain.is_empty();
    let mut onchain: Vec<([u8; 32], i128)> =
        onchain.into_iter().filter(|(acc, _)| session_of.contains_key(acc)).collect();
    if onchain.is_empty() && !session_of.is_empty() {
        // LIVENESS FLOOR (11-oct-2026), the same rule as `elect_finality_committee`: bound nodes exist but none
        // of their owners is reputable → they lead with EQUAL weight (skipping froze the test chain at #500).
        onchain = session_of.keys().map(|acc| (*acc, 1i128)).collect();
    }
    if onchain.is_empty() {
        // Reputation but no bound node: nobody leads. Neither: a lone dev chain.
        return if had_reputation { Leadership::Nobody } else { Leadership::LoneDev };
    }

    // §2.1 anti-grinding: once the commit–reveal beacon is populated, draw off it — candidate key = its
    // committed secret, seed = beacon ⊕ slot. Before the pipeline fills, the reputation-only draw.
    let mut reputation: BTreeMap<String, i128> = BTreeMap::new();
    let mut keys: BTreeMap<String, String> = BTreeMap::new();
    let mut account_of: BTreeMap<String, [u8; 32]> = BTreeMap::new();
    let committed = api.beacon_committed_keys(at).unwrap_or_default();
    let seed = if committed.is_empty() {
        for (acc, rep) in &onchain {
            let id = crate::service::hex32(acc);
            keys.insert(id.clone(), format!("sk-{id}"));
            reputation.insert(id.clone(), *rep);
            account_of.insert(id, *acc);
        }
        format!("slot{slot}")
    } else {
        let kmap: BTreeMap<[u8; 32], String> = committed.into_iter().collect();
        for (acc, rep) in &onchain {
            if let Some(k) = kmap.get(acc) {
                let id = crate::service::hex32(acc);
                keys.insert(id.clone(), k.clone());
                reputation.insert(id.clone(), *rep);
                account_of.insert(id, *acc);
            }
        }
        let beacon = match api.beacon_seed(at) {
            Ok(b) => b,
            Err(_) => return Leadership::Nobody,
        };
        format!("{}-slot{slot}", consensus_core::beacon::hex(&beacon))
    };

    let committee = consensus_core::elect_committee_fp(&reputation, &keys, &seed, TAU);
    // SINGLE LEADER per slot: of the elected committee, the member minimising `H(seed | id)`.
    let leader = committee
        .keys()
        .min_by_key(|id| sp_core::blake2_256(format!("{seed}|{id}").as_bytes()));
    match leader.and_then(|id| account_of.get(id)) {
        Some(acc) => match session_of.get(acc) {
            Some(session) => Leadership::Leader { account: *acc, session: *session },
            None => Leadership::Nobody,
        },
        None => Leadership::Nobody,
    }
}

impl Leadership {
    /// Does the node holding the session public `me` lead? Only the bound session key counts: it is the key the
    /// verifier checks the seal against (a node holding the cold account key but another session would be
    /// rejected by every peer, so it must not seal).
    pub fn is_me(&self, me: Option<[u8; 32]>) -> bool {
        matches!((self, me), (Leadership::Leader { session, .. }, Some(me)) if *session == me)
    }
}

/// The message the leader signs: domain ‖ pre-seal header hash.
fn seal_message(pre_hash: &H256) -> Vec<u8> {
    let mut m = SEAL_DOMAIN.to_vec();
    m.extend_from_slice(pre_hash.as_bytes());
    m
}

/// The slot a header claims (its `HLQA` pre-runtime digest), if any.
pub fn claimed_slot<H: HeaderT>(header: &H) -> Option<u64> {
    header.digest().logs().iter().find_map(|d| match d {
        DigestItem::PreRuntime(id, data) if *id == ENGINE_ID && data.len() == 8 => {
            let mut b = [0u8; 8];
            b.copy_from_slice(data);
            Some(u64::from_le_bytes(b))
        },
        _ => None,
    })
}

/// Verify an imported block's authorship. On success the seal is moved from the header into
/// `post_digests` (as Aura does), so the block is imported with the hash it was gossiped with.
pub fn check_header(
    client: &FullClient,
    block: &mut BlockImportParams<Block>,
) -> Result<(), String> {
    let parent_hash = *block.header.parent_hash();
    let number = *block.header.number();
    let parent = client
        .header(parent_hash)
        .map_err(|e| format!("authorship: parent header of #{number}: {e}"))?
        .ok_or_else(|| format!("authorship: parent of #{number} unknown"))?;
    let slot = claimed_slot(&block.header);
    // Leadership must be computed for the claimed slot; a block without a claim is checked as slot 0, which
    // only a lone dev chain accepts.
    let leadership = slot_leader(client, parent_hash, slot.unwrap_or(0));
    if leadership == Leadership::LoneDev {
        return Ok(());
    }
    let slot = slot.ok_or_else(|| format!("authorship: #{number} carries no slot claim"))?;
    let parent_slot = claimed_slot(&parent).unwrap_or(0);
    if slot <= parent_slot {
        return Err(format!("authorship: #{number} claims slot {slot}, not after its parent's {parent_slot}"));
    }
    let now = current_slot(SLOT_MS.load(Ordering::Relaxed));
    if slot > now + MAX_FUTURE_SLOTS {
        return Err(format!("authorship: #{number} claims slot {slot}, {} ahead of ours ({now})", slot - now));
    }
    let session = match leadership {
        Leadership::Leader { session, .. } => session,
        _ => return Err(format!("authorship: nobody leads slot {slot} at the parent of #{number}")),
    };
    // The seal is the LAST digest item; everything before it is what the leader signed.
    let seal = match block.header.digest_mut().pop() {
        Some(DigestItem::Seal(id, sig)) if id == ENGINE_ID => sig,
        Some(other) => {
            block.header.digest_mut().push(other);
            return Err(format!("authorship: #{number} is not sealed"));
        },
        None => return Err(format!("authorship: #{number} is not sealed")),
    };
    let sig = <[u8; 64]>::try_from(seal.as_slice())
        .map_err(|_| format!("authorship: #{number} seal is {} bytes, not 64", seal.len()))?;
    let pre_hash = block.header.hash();
    if !sr25519::Pair::verify(
        &sr25519::Signature::from_raw(sig),
        &seal_message(&pre_hash),
        &sr25519::Public::from_raw(session),
    ) {
        return Err(format!("authorship: #{number} slot {slot} is not sealed by its leader"));
    }
    block.post_digests.push(DigestItem::Seal(ENGINE_ID, seal));
    block.post_hash = Some(block.header.hash_with_post(&block.post_digests));
    Ok(())
}

/// Hash of a header once the post-digests are appended (what peers see and what the client stores).
trait HashWithPost {
    fn hash_with_post(&self, post: &[DigestItem]) -> H256;
}
impl<H: HeaderT<Hash = H256>> HashWithPost for H {
    fn hash_with_post(&self, post: &[DigestItem]) -> H256 {
        let mut h = self.clone();
        h.digest_mut().logs.extend(post.iter().cloned());
        h.hash()
    }
}

/// The claim the authoring loop hands to the digest provider: "seal on THIS parent for THIS slot".
pub type PendingClaim = Arc<Mutex<Option<(H256, u64)>>>;

/// Manual-seal digest provider of the Woven-Trust author: writes the slot claim and seals the header.
pub struct SlotClaims {
    /// Set by the authoring loop right before it asks manual-seal for a block.
    pub pending: PendingClaim,
    /// The `--vote-as` session key that signs the seal (`None`: a lone dev chain, nothing to sign).
    pub pair: Option<sr25519::Pair>,
}

impl sc_consensus_manual_seal::consensus::ConsensusDataProvider<Block> for SlotClaims {
    fn create_digest(
        &self,
        parent: &<Block as BlockT>::Header,
        _inherents: &sp_inherents::InherentData,
    ) -> Result<Digest, sc_consensus_manual_seal::Error> {
        let claim = *self.pending.lock().map_err(|_| err("slot claim lock poisoned"))?;
        match claim {
            Some((hash, slot)) if hash == parent.hash() => {
                Ok(Digest { logs: vec![DigestItem::PreRuntime(ENGINE_ID, slot.to_le_bytes().to_vec())] })
            },
            // The author elected itself on another parent (the tip moved): no block rather than a wrong one.
            _ => Err(err("no slot claim for this parent")),
        }
    }

    fn append_block_import(
        &self,
        _parent: &<Block as BlockT>::Header,
        params: &mut BlockImportParams<Block>,
        _inherents: &sp_inherents::InherentData,
        _proof: sp_api::StorageProof,
    ) -> Result<(), sc_consensus_manual_seal::Error> {
        // A lone dev chain authors without a key and its blocks are not checked; anything else that gets this
        // far was elected through `is_me`, which needs the key.
        let Some(pair) = self.pair.as_ref() else { return Ok(()) };
        let pre_hash = params.header.hash();
        let sig = pair.sign(&seal_message(&pre_hash)).0;
        params.post_digests.push(DigestItem::Seal(ENGINE_ID, sig.to_vec()));
        Ok(())
    }
}

fn err(s: &str) -> sc_consensus_manual_seal::Error {
    sc_consensus_manual_seal::Error::StringError(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_runtime::traits::BlakeTwo256;
    type Header = sp_runtime::generic::Header<u32, BlakeTwo256>;

    fn header_with_slot(slot: Option<u64>) -> Header {
        let mut h = Header::new(
            1,
            Default::default(),
            Default::default(),
            Default::default(),
            Default::default(),
        );
        if let Some(s) = slot {
            h.digest_mut().push(DigestItem::PreRuntime(ENGINE_ID, s.to_le_bytes().to_vec()));
        }
        h
    }

    #[test]
    fn slot_claim_round_trips_and_ignores_other_engines() {
        assert_eq!(claimed_slot(&header_with_slot(Some(123_456))), Some(123_456));
        assert_eq!(claimed_slot(&header_with_slot(None)), None);
        let mut h = header_with_slot(None);
        h.digest_mut().push(DigestItem::PreRuntime(*b"aura", 9u64.to_le_bytes().to_vec()));
        assert_eq!(claimed_slot(&h), None, "another engine's pre-digest is not our claim");
    }

    #[test]
    fn seal_signs_the_pre_seal_hash_under_its_own_domain() {
        let leader = sr25519::Pair::from_string("//Leader", None).unwrap();
        let other = sr25519::Pair::from_string("//Other", None).unwrap();
        let h = header_with_slot(Some(7));
        let pre = h.hash();
        let sig = leader.sign(&seal_message(&pre));
        assert!(sr25519::Pair::verify(&sig, &seal_message(&pre), &leader.public()));
        assert!(!sr25519::Pair::verify(&sig, &seal_message(&pre), &other.public()), "only the leader's key");
        // a signature over the bare hash (as a vote-like message would be) is not a seal
        let bare = leader.sign(pre.as_bytes());
        assert!(!sr25519::Pair::verify(&bare, &seal_message(&pre), &leader.public()), "domain separation");
        // changing anything signed (here the slot) breaks the seal
        let pre2 = header_with_slot(Some(8)).hash();
        assert!(!sr25519::Pair::verify(&sig, &seal_message(&pre2), &leader.public()));
    }

    #[test]
    fn leadership_is_me_only_for_the_bound_session_key() {
        let l = Leadership::Leader { account: [1; 32], session: [2; 32] };
        assert!(l.is_me(Some([2; 32])));
        assert!(!l.is_me(Some([1; 32])), "the cold account key does not seal: the session key does");
        assert!(!l.is_me(Some([3; 32])));
        assert!(!l.is_me(None));
        assert!(!Leadership::Nobody.is_me(Some([2; 32])));
    }
}
