//! Node-side provider of the PARTICIPATION inherent (F2 piece 6c — the reviewer's spec §4).
//!
//! Every block this node authors must carry a `ParticipationRecord` (the runtime marks the inherent
//! MANDATORY): the author's committee identity plus the **canonical** finality votes of every height
//! finalized since the on-chain cursor. The record is assembled from what THIS node's backend holds as
//! applied state — the justifications stored with finalized blocks — and **never from gossip**: a vote
//! that never made it into a canonical justification does not exist for the counters, however loudly it
//! was heard on the wire. The on-chain side re-verifies every signature and committee membership anyway
//! (`note_participation`), so this provider is a shuttle, not a gate.
//!
//! Startup shape (spec'd): cursor == finalized → a record with the author and NO votes is VALID.

use crate::service::FullClient;
use harlequin_consensus_api::HarlequinConsensusApi;
use harlequin_runtime::interface::OpaqueBlock as Block;
use pallet_participation::{author_message, ParticipationRecord, SignedVote, INHERENT_IDENTIFIER};
use polkadot_sdk::{sc_client_api::BlockBackend, sp_blockchain::HeaderBackend};
use sp_api::ProvideRuntimeApi;
use sp_core::{sr25519, Pair};
use sp_inherents::{InherentData, InherentDataProvider, InherentIdentifier};
use sp_runtime::traits::Block as BlockT;
use std::sync::Arc;

type Hash = <Block as BlockT>::Hash;

/// Finalized heights carried per record, at most: a long-offline author catches the on-chain cursor up
/// across a few blocks instead of building one unbounded inherent (the cursor advances monotonically,
/// so nothing is lost — only spread out).
const MAX_HEIGHTS_PER_RECORD: u64 = 64;

/// The per-block provider: holds the record built for the block being authored NOW.
pub struct ParticipationInherentProvider {
    record: ParticipationRecord,
}

#[async_trait::async_trait]
impl InherentDataProvider for ParticipationInherentProvider {
    async fn provide_inherent_data(
        &self,
        inherent_data: &mut InherentData,
    ) -> Result<(), sp_inherents::Error> {
        inherent_data.put_data(INHERENT_IDENTIFIER, &self.record)
    }

    async fn try_handle_error(
        &self,
        identifier: &InherentIdentifier,
        _error: &[u8],
    ) -> Option<Result<(), sp_inherents::Error>> {
        if *identifier == INHERENT_IDENTIFIER {
            Some(Err(sp_inherents::Error::Application(Box::from(
                "participation inherent rejected",
            ))))
        } else {
            None
        }
    }
}

/// Build the provider for a block being authored on top of `parent_hash`.
///
/// - `vote_pair`: this node's `--vote-as` sr25519 keypair (the same identity the slot-leader election used
///   to let this node author at all). P-1/#838: the record no longer carries a free-form author field — it
///   carries the vote-key PUBLIC plus an sr25519 SIGNATURE over `author_message(at_block)`, so the pallet
///   credits authorship to `vote_key_owner(pubkey)` only after verifying the signature. A node without a
///   vote key (dev/instant seal) signs nothing (zero key + zero sig) → the pallet credits nobody, which is
///   exactly right for a keyless dev author.
/// - votes: for each height in `(on-chain cursor .. finalized]` (capped), the votes decoded from the
///   canonical `WTC1` justification stored with that block. Heights without a stored justification
///   (e.g. finalized via a peer's proof that carried another height's batch) are skipped — the cursor
///   logic on-chain only advances past what was actually credited.
pub fn provider_for(
    client: &Arc<FullClient>,
    parent_hash: Hash,
    vote_pair: Option<&sr25519::Pair>,
) -> ParticipationInherentProvider {
    let at_block = client
        .number(parent_hash)
        .ok()
        .flatten()
        .map(|n| n as u64 + 1)
        .unwrap_or(0);
    let cursor = client
        .runtime_api()
        .participation_cursor(parent_hash)
        .unwrap_or(0);
    let finalized = client.info().finalized_number as u64;
    let mut finality_votes: Vec<SignedVote> = Vec::new();
    let from = cursor.saturating_add(1);
    let to = finalized.min(cursor.saturating_add(MAX_HEIGHTS_PER_RECORD));
    for height in from..=to {
        if let Some(votes) = votes_for_height(client, height) {
            finality_votes.extend(votes);
        }
    }
    // P-1: prove authorship by signing `author_message(at_block)` with the vote key. Keyless (dev) → zero
    // key + zero sig, which the pallet treats as an unverifiable author and simply does not credit.
    let (author_key, author_sig) = match vote_pair {
        Some(pair) => (pair.public().0, pair.sign(&author_message(at_block)).0),
        None => ([0u8; 32], [0u8; 64]),
    };
    ParticipationInherentProvider {
        record: ParticipationRecord {
            author_key,
            author_sig,
            finality_votes,
            at_block,
        },
    }
}

/// Decode the canonical justification of finalized `height` into its signed votes. Wire layout is the
/// finality gadget's `encode_proof` (finality.rs): `height (u64 LE) ‖ hash (32) ‖ count (u16 LE)` then
/// `count × (signer 32 ‖ sig 64)`. Returns `None` when the block or its `WTC1` justification is absent.
fn votes_for_height(client: &Arc<FullClient>, height: u64) -> Option<Vec<SignedVote>> {
    let hash = client.hash((height as u32).into()).ok().flatten()?;
    let justifications = client.justifications(hash).ok().flatten()?;
    let proof = justifications.get(crate::finality::ENGINE_ID)?;
    decode_proof_votes(proof)
}

/// Parse the proof bytes into `SignedVote`s (pure; layout above). `None` on any malformed shape — a
/// canonical justification written by this codebase always parses; garbage is simply not carried.
fn decode_proof_votes(proof: &[u8]) -> Option<Vec<SignedVote>> {
    const HEADER: usize = 8 + 32 + 2;
    const VOTE: usize = 32 + 64;
    if proof.len() < HEADER {
        return None;
    }
    let height = u64::from_le_bytes(proof[0..8].try_into().ok()?);
    let mut block_hash = [0u8; 32];
    block_hash.copy_from_slice(&proof[8..40]);
    let count = u16::from_le_bytes(proof[40..42].try_into().ok()?) as usize;
    if proof.len() < HEADER + count * VOTE {
        return None;
    }
    let mut votes = Vec::with_capacity(count);
    for i in 0..count {
        let base = HEADER + i * VOTE;
        let mut signer = [0u8; 32];
        signer.copy_from_slice(&proof[base..base + 32]);
        let mut sig = [0u8; 64];
        sig.copy_from_slice(&proof[base + 32..base + 96]);
        votes.push(SignedVote { signer, sig, height, block_hash });
    }
    Some(votes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_gadgets_proof_layout() {
        // height 7, hash 0xAA…, two votes — the exact encode_proof shape.
        let mut proof = Vec::new();
        proof.extend_from_slice(&7u64.to_le_bytes());
        proof.extend_from_slice(&[0xAA; 32]);
        proof.extend_from_slice(&2u16.to_le_bytes());
        for b in [1u8, 2] {
            proof.extend_from_slice(&[b; 32]); // signer
            proof.extend_from_slice(&[b; 64]); // sig
        }
        let votes = decode_proof_votes(&proof).expect("canonical layout parses");
        assert_eq!(votes.len(), 2);
        assert_eq!(votes[0].height, 7);
        assert_eq!(votes[0].block_hash, [0xAA; 32]);
        assert_eq!(votes[1].signer, [2u8; 32]);
        // truncated garbage does not parse into phantom votes
        assert!(decode_proof_votes(&proof[..proof.len() - 1]).is_none());
        assert!(decode_proof_votes(&[]).is_none());
    }
}
