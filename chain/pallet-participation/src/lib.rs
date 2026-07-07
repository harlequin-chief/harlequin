//! # pallet-participation — the on-chain participation feed
//!
//! **Why this exists (finding).** The two signals that measure *honest node service* —
//! block authorship *in a node's committee turn* and *finality votes cast in committee* — live only at
//! the NODE level today (manual-seal authoring; the Snowball finality gadget in `consensus-core`). They are
//! never written to chain, so nothing on-chain can read them. Both **proof-of-service rewards**
//! (`pallet-tokens::record_service`) and **production reputation** (`pallet-reputation`'s `EvidenceOrigin`,
//! still a root placeholder) need EXACTLY this feed. We build it once, here, and both consume it.
//!
//! **How.** Each block the authoring node provides an **inherent** (no privileged origin, no signed extrinsic)
//! declaring who participated. The pallet folds it into per-epoch counters. At the epoch boundary the
//! consumers read the counters (rewards splits the pool by service; reputation credits evidence) and the
//! epoch is reset.
//!
//! **Invariant §0 / Art. VI — participation ≠ power.** This pallet records *measured work*, nothing else.
//! It mints no coin, grants no vote, and moves no reputation by itself. Running a node buys no consensus
//! weight: the counters here are only an input, capped and orthogonal, to the money split and the evidence
//! record. Money ≠ power.
//!
//! ## Contract with the node (the interface the reviewer fills)
//! The node-side inherent provider (consensus/node — the reviewer) emits [`ParticipationRecord`] per block. This
//! pallet owns the on-chain side (storage + folding + the consumer read API). Three things are pinned WITH
//! the reviewer before activation (see the TODOs):
//!   (a) IDENTITY: the `[u8; 32]` in the record is the consensus `AccountId` (what `consensus_reputation`
//!       keys on). Must confirm the vote-key (`--vote-as-file session.secret`) maps to that same account;
//!       if not, a voteKey→AccountId registry is added.
//!   (b) SERVICE UNIT: what counts as authorship "in turn" (the committee slot the node was sortitioned
//!       into — not out-of-turn spam) and a finality vote "in committee". the reviewer defines the exact rule; the
//!       fold below filters to it so weight never rewards spam. (§3.1 cap is applied by the consumer.)
//!   (c) FINALITY DETERMINISM: a finality justification arrives with lag (~15–30 blocks) and different nodes
//!       import it at different heights, so an author-provided voter list could differ between authors. The
//!       credited voters MUST be canonical — taken from the finalization as it enters canonically, not from
//!       whatever each author happened to have seen. the reviewer designs the node mechanism; this pallet credits
//!       votes only against a canonical finalization marker.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;

/// Inherent identifier for the participation record (8 bytes, unique on the chain). Matched by the
/// node-side provider.
pub const INHERENT_IDENTIFIER: [u8; 8] = *b"hlqpartp";

/// A committee finality vote, re-verifiable on-chain. Mirrors the node's `finality.rs` `Vote`: an sr25519
/// signature by `signer` over the canonical payload `height ‖ block_hash`. Carried inside the inherent so
/// the pallet VERIFIES rather than trusts the author (contract (c), STRONG anti-gaming — this feeds money).
#[derive(Encode, Decode, codec::DecodeWithMemTracking, MaxEncodedLen, TypeInfo, Clone, PartialEq, Eq, Debug)]
pub struct SignedVote {
    /// The voter's reputation account = the sr25519 public key that signed (contract (a)).
    pub signer: [u8; 32],
    /// sr25519 signature over `signed_message(height, block_hash)`.
    pub sig: [u8; 64],
    /// Height of the finalized block this vote is for. **u64** — matches `finality.rs` `Vote.height`.
    pub height: u64,
    /// Hash of the finalized block this vote is for.
    pub block_hash: [u8; 32],
}

/// The exact bytes a committee vote signs. Matches the node's `finality.rs` signing payload byte-for-byte
/// (confirmed w/ the reviewer): `height` as **u64 little-endian (8 bytes)** ‖ `block_hash` (32 bytes) = 40 bytes,
/// no domain/prefix. The pallet re-derives this and checks the sr25519 signature against it.
pub fn signed_message(height: u64, block_hash: &[u8; 32]) -> alloc::vec::Vec<u8> {
    let mut m = alloc::vec::Vec::with_capacity(8 + 32);
    m.extend_from_slice(&height.to_le_bytes());
    m.extend_from_slice(block_hash);
    m
}

/// The exact bytes a block AUTHOR signs to prove authorship (P-1/#838): a domain-separation tag ‖ the
/// block number (u64 LE). The distinct tag makes an author signature un-confusable with a finality vote
/// (`signed_message`), and binding the block number stops replay of one block's attestation onto another.
/// The node's `participation_inherent.rs` builds this identically and signs it with its vote key; the
/// pallet re-derives it in `note_participation` and checks the sr25519 signature before crediting authorship.
pub fn author_message(at_block: u64) -> alloc::vec::Vec<u8> {
    let mut m = alloc::vec::Vec::with_capacity(20 + 8);
    m.extend_from_slice(b"harlequin-author:v1:");
    m.extend_from_slice(&at_block.to_le_bytes());
    m
}

/// The conservative on-chain reputation the committee sortition uses: `consensus_reputation` = the min of
/// the four suits (i128) per account, as pallet-reputation exposes it. The runtime wires this to
/// pallet-reputation; kept as a trait so this pallet does not hard-depend on it and stays unit-testable.
pub trait ConsensusReputation<AccountId> {
    /// Every account with its conservative (min-of-suits) reputation, at the CURRENT block. The committee for
    /// an epoch is built from this AT THAT EPOCH'S START and then cached, so later blocks re-use the cached
    /// set instead of re-deriving against rotated reputation (the determinism fix).
    fn consensus_reputation() -> alloc::vec::Vec<(AccountId, i128)>;
}

/// The REST of the committee-election inputs (F2 piece 6 — the mainnet bomb fix): beacon state, the
/// entrenchment-halt flag, and the hot→cold vote-key delegation. The runtime wires these to
/// pallet-beacon / pallet-reputation; a trait so the pallet stays unit-testable (mock = fallback path).
/// With these, `build_committee` feeds `consensus_core::elect_finality_committee` — the SAME election
/// the node runs — instead of a fallback-only reimplementation that would seat the WRONG committee the
/// moment the beacon populates (and silently drop every finality vote on mainnet).
pub trait CommitteeInputs {
    /// The current commit–reveal beacon seed (§2.1). All-zero/default before the pipeline populates.
    fn beacon_seed() -> [u8; 32];
    /// Accounts with an ACTIVE committed sortition key this epoch: `(account, committed key hex)`.
    /// Empty → the election takes the genesis/bootstrap fallback path.
    fn beacon_committed_keys() -> alloc::vec::Vec<([u8; 32], alloc::string::String)>;
    /// §1.4b entrenchment-guard halt flag: `true` → the committee is EMPTY (finality frozen).
    fn entrenchment_halted() -> bool;
    /// Resolve a HOT session signing key to its COLD committee account (the vote-key delegation,
    /// `pallet-reputation::VoteKeys` reversed). `None` → the signer votes as itself (direct path).
    fn vote_key_owner(hot: &[u8; 32]) -> Option<[u8; 32]>;
}

/// COMMITTEE epoch length (blocks): the committee is re-drawn every this-many blocks. Governs the committee
/// cache + `is_member` ONLY — SEPARATE from the rewards era (`Config::EpochLength`). **MUST match the node's
/// `finality.rs::EPOCH_LENGTH` byte-for-byte, and it is feature-gated there**: 300 on mainnet (1h @12s), 10 on testnet.
/// Hardcoding 10 would silently break mainnet (committee misaligned → all finality votes dropped) — a bug
/// invisible on testnet. Gated to mirror the node exactly.
#[cfg(feature = "mainnet")]
pub const COMMITTEE_EPOCH_LENGTH: u64 = 300;
#[cfg(not(feature = "mainnet"))]
pub const COMMITTEE_EPOCH_LENGTH: u64 = 10;

/// Committee sortition threshold τ — the expected committee size. **MUST match the node's
/// `finality.rs::COMMITTEE_TAU`, also feature-gated**: 60 on mainnet, 4 on testnet. Same mainnet-bomb risk.
#[cfg(feature = "mainnet")]
pub const COMMITTEE_TAU: u32 = 60;
#[cfg(not(feature = "mainnet"))]
pub const COMMITTEE_TAU: u32 = 4;

/// 64-char lowercase hex of 32 bytes, NO `0x`, no separators — BYTE-IDENTICAL to the node's `service::hex32`
/// (confirmed w/ the reviewer). Any difference changes the VRF input → a different committee → broken. Uses the
/// exact table-push form the node uses.
pub fn hex32(b: &[u8; 32]) -> alloc::string::String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = alloc::string::String::with_capacity(64);
    for &byte in b.iter() {
        s.push(HEX[(byte >> 4) as usize] as char);
        s.push(HEX[(byte & 0x0f) as usize] as char);
    }
    s
}

/// What the authoring node declares each block. THE CONTRACT with the node-side provider. (No
/// `MaxEncodedLen`: it carries an unbounded `Vec<SignedVote>` — it is inherent data, never storage.)
#[derive(Encode, Decode, codec::DecodeWithMemTracking, TypeInfo, Clone, PartialEq, Eq, Debug)]
pub struct ParticipationRecord {
    /// The HOT vote key that signed this block's authorship (P-1/#838). The pallet does NOT trust a
    /// free-form author field: it credits `vote_key_owner(author_key)` (the COLD account this key delegates
    /// for), gated by an sr25519 signature (`author_sig`) proving control of the key. So authorship credit
    /// (+ fee node-share + P-2 emission) binds to the real key-holder — a validator cannot name an arbitrary
    /// alt. Same primitive as the finality votes (contract (c)).
    pub author_key: [u8; 32],
    /// sr25519 signature by `author_key` over `author_message(at_block)`. Verified in `note_participation`;
    /// an invalid signature simply drops the authorship credit (no error — the inherent is MANDATORY, so a
    /// hard reject would halt; consistent with how non-member votes are dropped, not fatal).
    pub author_sig: [u8; 64],
    /// The SIGNED committee votes from the canonical justification(s) of the block(s) that finalized since
    /// the previous record. The pallet re-verifies each signature AND committee membership (contract (c)):
    /// a malicious author cannot inflate — signatures are unforgeable and non-members are dropped.
    pub finality_votes: alloc::vec::Vec<SignedVote>,
    /// The block height this record is for (sanity: must equal the current block). u64, matching heights.
    pub at_block: u64,
}

#[frame::pallet]
pub mod pallet {
    use super::{CommitteeInputs, ConsensusReputation, ParticipationRecord};
    use frame::prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Blocks per SERVICE epoch — the fold cadence (the reviewer's spec): MUST equal the
        /// reputation pallet's `EpochLength`, because pallet-reputation consumes the just-ended
        /// epoch's counters at ITS epoch boundary (same block; reputation's `on_initialize` runs
        /// first by pallet index). Reputation measures CONDUCT — fast, per epoch.
        #[pallet::constant]
        type EpochLength: Get<BlockNumberFor<Self>>;

        /// Blocks per REWARDS era — MUST equal pallet-tokens `EraLength`. At each epoch fold the
        /// ended epoch's counters roll up into [`EraServiceAccumulator`] under the era that OWNS the
        /// epoch's start block; the economy pays the ACCUMULATED era at its own slow cadence. Each
        /// service unit is counted EXACTLY once per consumer: read per-epoch (reputation) before the
        /// fold, summed per-era (rewards) from the accumulator after it.
        #[pallet::constant]
        type EraLength: Get<BlockNumberFor<Self>>;

        /// Max committee size cached per epoch (bound for the on-chain `BoundedVec`). ~64 is safe headroom
        /// over `COMMITTEE_TAU`-scale committees.
        #[pallet::constant]
        type MaxCommittee: Get<u32>;

        /// Source of the conservative on-chain reputation (min of suits) that the committee sortition reads.
        /// The runtime wires it to pallet-reputation's `consensus_reputation`.
        type Reputation: super::ConsensusReputation<Self::AccountId>;

        /// The remaining committee-election inputs: beacon, halt flag, hot→cold delegation (F2 piece
        /// 6). The runtime wires them to pallet-beacon / pallet-reputation.
        type Committee: super::CommitteeInputs;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Current epoch index (bumped every `EpochLength` blocks by `on_initialize`).
    #[pallet::storage]
    pub type CurrentEpoch<T> = StorageValue<_, u64, ValueQuery>;

    /// Blocks a node authored (in its committee turn) in the given epoch. Reset at the epoch boundary after
    /// the consumers read it.
    #[pallet::storage]
    pub type AuthoredInEpoch<T: Config> =
        StorageDoubleMap<_, Twox64Concat, u64, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// Finality votes a node cast (in committee, against canonical finalizations) in the given epoch.
    #[pallet::storage]
    pub type FinalityVotesInEpoch<T: Config> =
        StorageDoubleMap<_, Twox64Concat, u64, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// Guard: at most one participation inherent may be applied per block.
    #[pallet::storage]
    pub type NotedThisBlock<T> = StorageValue<_, bool, ValueQuery>;

    /// The account that authored the CURRENT block, as declared by this block's participation inherent
    /// (WTC1-gated, self-attested-and-sound — same trust base as `AuthoredInEpoch`). Reset each
    /// `on_initialize`; absent until the inherent lands (inherents apply before extrinsics, so fee logic
    /// reading this during extrinsic application sees the author). Consumers: the runtime's HLQ fee
    /// adapter (fee node-share, SPEC-RELAUNCH §2) — if absent, the share is burned (public, no discretion).
    #[pallet::storage]
    pub type BlockAuthor<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

    /// Highest finalized height whose committee votes have already been credited. `note_participation` only
    /// credits votes for heights ABOVE this and then advances it — so each finalized height is credited
    /// EXACTLY ONCE, in order, no matter which proposer reports it or whether two records overlap. Idempotent
    /// + deterministic + double-count-proof via on-chain state (not a fragile per-node local cursor).
    #[pallet::storage]
    pub type LastCreditedHeight<T> = StorageValue<_, u64, ValueQuery>;

    /// Per-ERA service rollup: `(era, account) -> accumulated weight`. Written by the epoch fold in
    /// `on_initialize` (ended epoch's counters summed in, then the per-epoch entries removed — bounded
    /// state, exactly-once). Read by the rewards consumer (pallet-tokens) at the era boundary, then
    /// cleared with [`Pallet::clear_era`].
    #[pallet::storage]
    pub type EraServiceAccumulator<T: Config> =
        StorageDoubleMap<_, Twox64Concat, u64, Blake2_128Concat, T::AccountId, u64, ValueQuery>;

    /// The finality committee CACHED for each committee epoch (`height / EPOCH_LENGTH`). Computed ONCE when
    /// the epoch starts (with that epoch's reputation) and only queried afterwards — so a vote for a past
    /// height re-checks against the committee that was actually in force then, not against rotated
    /// reputation. This is the determinism fix. Old epochs are pruned (a few kept) at each epoch bump.
    #[pallet::storage]
    pub type CommitteeForEpoch<T: Config> =
        StorageMap<_, Twox64Concat, u64, BoundedVec<[u8; 32], T::MaxCommittee>, OptionQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A block's participation was folded in: `author` credited one authored block, `voters` finality
        /// votes credited, for `epoch`.
        ParticipationNoted { epoch: u64, author: Option<T::AccountId>, voters: u32 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// More than one participation inherent in a block.
        AlreadyNoted,
        /// The record's `at_block` did not match the current block.
        WrongBlock,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_initialize(n: BlockNumberFor<T>) -> Weight {
            NotedThisBlock::<T>::put(false);
            BlockAuthor::<T>::kill(); // per-block value; a block without the inherent has no author
            let bn: u64 = n.saturated_into::<u64>();

            // COMMITTEE epoch boundary (cadence EPOCH_LENGTH=10): cache the committee for the epoch that
            // STARTS now, using the reputation in force NOW — this matches the node's epoch-start anchor, so
            // a later vote for a past height re-checks against the committee that actually held then. Retain
            // a few epochs and prune the rest. TODO(the reviewer): seed CommitteeForEpoch[0] at genesis (heights
            // 0..EPOCH_LENGTH finalize before the first boundary fires).
            // Fire at the epoch boundary, OR whenever the current epoch has no cached committee yet — the
            // latter auto-seeds epoch 0 at block 1 (heights 0..EPOCH_LENGTH finalize before the first
            // boundary would fire), using the genesis reputation the runtime already grants the founders.
            // Robust without depending on genesis_build ordering.
            let cepoch = bn / super::COMMITTEE_EPOCH_LENGTH;
            if bn % super::COMMITTEE_EPOCH_LENGTH == 0 || !CommitteeForEpoch::<T>::contains_key(cepoch) {
                let committee = Self::build_committee(cepoch);
                CommitteeForEpoch::<T>::insert(cepoch, committee);
                if cepoch >= 4 {
                    CommitteeForEpoch::<T>::remove(cepoch - 4);
                }
            }

            // SERVICE epoch boundary (= the reputation epoch, the reviewer's spec): the ended
            // epoch's counters ROLL UP into the era accumulator and the per-epoch entries are removed.
            // Order inside this block is what makes it exactly-once: pallet-reputation (lower pallet
            // index) already read the ended epoch's counters in ITS on_initialize; this fold then
            // moves them—once—into the era bucket the epoch's START block belongs to, where the
            // rewards consumer reads them at the era boundary. No double count, no boundary loss,
            // bounded state (per-epoch maps live exactly one epoch).
            let len = T::EpochLength::get();
            if !n.is_zero() && !len.is_zero() && (n % len).is_zero() {
                let bn_len: u64 = len.saturated_into::<u64>();
                let ended = (bn / bn_len).saturating_sub(1);
                let era_len: u64 = T::EraLength::get().saturated_into::<u64>();
                if era_len > 0 {
                    let era = ended.saturating_mul(bn_len) / era_len;
                    for (who, w) in Self::service_weights(ended) {
                        EraServiceAccumulator::<T>::mutate(era, &who, |acc| {
                            *acc = acc.saturating_add(w)
                        });
                    }
                }
                let _ = AuthoredInEpoch::<T>::clear_prefix(ended, u32::MAX, None);
                let _ = FinalityVotesInEpoch::<T>::clear_prefix(ended, u32::MAX, None);
                CurrentEpoch::<T>::mutate(|e| *e = e.saturating_add(1));
            }
            Weight::from_parts(2_000_000, 0) // placeholder; benchmark pre-mainnet
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Fold this block's participation into the current epoch's counters. Inherent-only (`ensure_none`):
        /// provided by the authoring node, never by a user.
        ///
        /// SECURITY (the reviewer review): this pallet implements NO `ValidateUnsigned` for this call. So an
        /// unsigned `note_participation` submitted to the TXPOOL is invalid and dropped — ONLY
        /// `ProvideInherent` (the block author, once per block) can insert it. This closes the "inflate my
        /// AuthoredInEpoch without producing a block" vector. And `NotedThisBlock` is a HARD one-per-block
        /// guard (reset in `on_initialize`, set here) — a second `note_participation` in the same block errors
        /// with `AlreadyNoted`, so the author cannot double-count itself. Vote content is re-verified below
        /// (sr25519 + committee membership), identically on every importing node ⇒ deterministic; a forged
        /// vote is dropped everywhere the same way.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn note_participation(origin: OriginFor<T>, record: ParticipationRecord) -> DispatchResult {
            ensure_none(origin)?;
            ensure!(!NotedThisBlock::<T>::get(), Error::<T>::AlreadyNoted);
            let now: u64 = frame_system::Pallet::<T>::block_number().saturated_into::<u64>();
            ensure!(record.at_block == now, Error::<T>::WrongBlock);
            NotedThisBlock::<T>::put(true);

            let epoch = CurrentEpoch::<T>::get();
            // AUTHOR — P-1/#838: DERIVE the author from the signing vote key, never trust a free-form field.
            // The record proves control of a hot vote key (sr25519 sig over `author_message(now)`); the
            // credited account is that key's COLD owner (`vote_key_owner`), and it must be committee-eligible
            // for this height. This binds authorship credit (+ fee node-share + P-2 emission) to the real
            // key-holder — a validator with a tampered binary can't redirect it to an arbitrary alt.
            // SOFT-DROP (not a hard `ensure!`): `note_participation` is a MANDATORY inherent, so a hard
            // reject of a bad author would HALT the chain. Instead — exactly like a non-member finality vote
            // below — an unverifiable or non-committee author is simply NOT credited (no `AuthoredInEpoch`,
            // no `BlockAuthor` → that block's node-share is forfeit), and the block stays valid. No wall-clock
            // / slot recomputation anywhere (option A was rejected for its halt risk).
            let mut credited_author: Option<T::AccountId> = None;
            let author_ok = Self::verify_vote(
                &record.author_key,
                &record.author_sig,
                &super::author_message(now),
            );
            if author_ok {
                let cold = T::Committee::vote_key_owner(&record.author_key)
                    .unwrap_or(record.author_key);
                // Committee membership is checked on the RAW cold key (same as the votes below), then decode.
                if Self::is_member(now, &cold) {
                    let author = Self::account_from_bytes(&cold);
                    AuthoredInEpoch::<T>::mutate(epoch, &author, |c| *c = c.saturating_add(1));
                    BlockAuthor::<T>::put(&author); // this block's author, for the fee node-share
                    credited_author = Some(author);
                }
            }
            // FINALITY VOTES — VERIFY, do not trust the author (STRONG, contract (c); this feeds money).
            // For each carried vote: re-check the sr25519 signature over the canonical payload AND that the
            // signer was in that height's committee. Forged or non-committee votes are silently dropped, so a
            // malicious author cannot inflate anyone's service weight.
            // On-chain cursor: credit ONLY heights strictly above the last credited one, then advance to the
            // max valid height seen. Exactly-once per height, dup-proof, deterministic across proposers.
            let cursor = LastCreditedHeight::<T>::get();
            let mut max_h = cursor;
            let mut counted: u32 = 0;
            for v in record.finality_votes.iter() {
                if v.height <= cursor {
                    continue; // already credited in a previous block
                }
                let msg = super::signed_message(v.height, &v.block_hash);
                if !Self::verify_vote(&v.signer, &v.sig, &msg) {
                    continue; // forged / invalid signature
                }
                // Hot→cold (F2 piece 6, the second half of the mainnet bomb): the SIGNER of a vote is
                // a hot session key; committee membership and service credit belong to the COLD
                // account it delegates for — exactly the node's `resolve_voter`. No delegation →
                // the signer votes as itself (direct/dev path).
                let cold = T::Committee::vote_key_owner(&v.signer).unwrap_or(v.signer);
                if !Self::is_member(v.height, &cold) {
                    continue; // the resolved account was not in that height's committee
                }
                let who = Self::account_from_bytes(&cold);
                FinalityVotesInEpoch::<T>::mutate(epoch, &who, |c| *c = c.saturating_add(1));
                if v.height > max_h {
                    max_h = v.height;
                }
                counted = counted.saturating_add(1);
            }
            if max_h > cursor {
                LastCreditedHeight::<T>::put(max_h);
            }
            Self::deposit_event(Event::ParticipationNoted {
                epoch,
                author: credited_author,
                voters: counted,
            });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Decode raw consensus account bytes into the runtime `AccountId`.
        /// TODO(identity, contract (a)): confirm `T::AccountId` is a 32-byte account equal to the consensus
        /// `AccountId`. If a voteKey→AccountId registry is needed, resolve it here.
        fn account_from_bytes(b: &[u8; 32]) -> T::AccountId {
            T::AccountId::decode(&mut &b[..]).expect("AccountId is 32 bytes on this chain; qed")
        }

        /// Re-verify a committee vote's sr25519 signature over `msg`. True iff `sig` is a valid signature of
        /// `msg` by the public key `signer`. This is the non-forgeability backbone of the STRONG anti-gaming
        /// rule — the author cannot fabricate votes.
        /// TODO(SDK-verify, with the reviewer at compile): confirm the primitive path under the `polkadot-sdk-frame`
        /// umbrella. Expected `sp_io::crypto::sr25519_verify(&Signature, msg, &Public)`; the payload byte
        /// layout in `super::signed_message` must match the node's `finality.rs` signing exactly.
        fn verify_vote(signer: &[u8; 32], sig: &[u8; 64], msg: &[u8]) -> bool {
            use frame::deps::sp_core::sr25519::{Public, Signature};
            let public = Public::from_raw(*signer);
            let signature = Signature::from_raw(*sig);
            frame::deps::sp_io::crypto::sr25519_verify(&signature, msg, &public)
        }

        /// AccountId → raw 32 bytes (AccountId32 encodes as exactly its 32 bytes, no prefix).
        fn account_to_bytes(acc: &T::AccountId) -> [u8; 32] {
            let mut out = [0u8; 32];
            let enc = acc.encode();
            let n = core::cmp::min(32, enc.len());
            out[..n].copy_from_slice(&enc[..n]);
            out
        }

        /// Build the finality committee for `epoch` — by calling THE election
        /// (`consensus_core::elect_finality_committee`, the same fn the node's `committee_for_height`
        /// feeds; F2 piece 6). Fallback, beacon (§2.1) and halt (§1.4b) paths all come with it: when
        /// the beacon populates on mainnet this pallet's committee flips to the committed-key draw in
        /// lockstep with the node — the "wrong committee → every vote dropped" mainnet bomb is dead by
        /// construction, not by parallel maintenance. Called once at the epoch's start; cached.
        ///
        /// Known, accepted divergence during an entrenchment HALT: the node's self-heal re-anchors a
        /// halt-band height's committee to the recovery block H_r; this cache was written at the
        /// band's epoch start (halted → EMPTY) and is not rewritten, so votes finalizing the band
        /// after recovery earn no service credit. Credit fairness in a ~never event, deterministic
        /// either way; never a consensus split.
        fn build_committee(epoch: u64) -> BoundedVec<[u8; 32], T::MaxCommittee> {
            let onchain: alloc::vec::Vec<([u8; 32], i128)> = T::Reputation::consensus_reputation()
                .iter()
                .map(|(acc, rep)| (Self::account_to_bytes(acc), *rep))
                .collect();
            let elected = consensus_core::elect_finality_committee(
                &onchain,
                &T::Committee::beacon_committed_keys(),
                &T::Committee::beacon_seed(),
                epoch as u32,
                super::COMMITTEE_TAU,
                consensus_core::FINALITY_COMMITTEE_SEED,
                T::Committee::entrenchment_halted(),
            );
            BoundedVec::truncate_from(elected.into_iter().collect())
        }

        /// Was `who` in the finality committee for `height`? Reads the CACHED committee for `height /
        /// EPOCH_LENGTH` (built at that epoch's start) — deterministic regardless of later reputation drift.
        fn is_member(height: u64, who: &[u8; 32]) -> bool {
            let epoch = height / super::COMMITTEE_EPOCH_LENGTH;
            CommitteeForEpoch::<T>::get(epoch).map_or(false, |c| c.iter().any(|m| m == who))
        }

        /// CONSUMER API — the measured service of `who` in `epoch`:
        ///   weight = blocks authored in turn + finality votes in committee.
        /// The per-node anti-Sybil cap (§3.1) is applied by the CONSUMER (pallet-tokens builds the reward
        /// vector and caps the tail; pallet-reputation weights evidence its own way). This returns the raw,
        /// honest counts.
        pub fn service_weight(epoch: u64, who: &T::AccountId) -> u64 {
            let authored = AuthoredInEpoch::<T>::get(epoch, who) as u64;
            let votes = FinalityVotesInEpoch::<T>::get(epoch, who) as u64;
            authored.saturating_add(votes)
        }

        /// All (account, weight) pairs with non-zero service in `epoch`, for the reward split / evidence.
        pub fn service_weights(epoch: u64) -> alloc::vec::Vec<(T::AccountId, u64)> {
            let mut out = alloc::vec::Vec::new();
            for (who, authored) in AuthoredInEpoch::<T>::iter_prefix(epoch) {
                let votes = FinalityVotesInEpoch::<T>::get(epoch, &who) as u64;
                out.push((who, (authored as u64).saturating_add(votes)));
            }
            // voters that authored nothing but voted:
            for (who, votes) in FinalityVotesInEpoch::<T>::iter_prefix(epoch) {
                if !AuthoredInEpoch::<T>::contains_key(epoch, &who) {
                    out.push((who, votes as u64));
                }
            }
            out
        }

        /// CONSUMER API (rewards, slow cadence) — the accumulated `(account, weight)` service of `era`,
        /// rolled up epoch by epoch by the fold. pallet-tokens reads this at ITS era boundary and then
        /// calls [`clear_era`](Self::clear_era).
        pub fn era_service(era: u64) -> alloc::vec::Vec<(T::AccountId, u64)> {
            EraServiceAccumulator::<T>::iter_prefix(era).collect()
        }

        /// Clear an era's accumulator once the rewards consumer has paid it. Deterministic; no
        /// privileged trigger.
        pub fn clear_era(era: u64) {
            let _ = EraServiceAccumulator::<T>::clear_prefix(era, u32::MAX, None);
        }
    }

    // ── INHERENT PLUMBING (sketch for the reviewer's consensus review, then compiled against the runtime) ──────
    // Security intent (the reviewer's two points):
    //   · NON-FORGEABLE: `create_inherent` only shuttles the node's record into the call; the REAL gate is
    //     `note_participation` on execution — it re-verifies every vote's sr25519 signature AND committee
    //     membership and drops the rest, and `author` is self-attest which WTC1's proposer gating makes
    //     sound. So a crafted record cannot inflate anyone's weight.
    //   · MANDATORY: `is_inherent_required` returns an error ⇒ a block that omits the participation inherent
    //     is rejected. The author always knows itself and the finalized votes, so it can always provide one
    //     (with empty `finality_votes` when nothing finalized since the previous block).
    // TODO(compile w/ the reviewer): confirm the exact trait/type paths under `polkadot-sdk-frame 0.17` + SDK
    // 2604.2.0 (no in-repo `ProvideInherent` example). Expected below.
    use frame::deps::sp_inherents::{InherentData, InherentIdentifier, IsFatalError};

    #[derive(codec::Encode, codec::Decode)]
    #[cfg_attr(feature = "std", derive(codec::DecodeWithMemTracking))]
    pub enum InherentError {
        /// The mandatory participation inherent was missing from the block.
        Missing,
    }
    impl IsFatalError for InherentError {
        fn is_fatal_error(&self) -> bool {
            true
        }
    }

    #[pallet::inherent]
    impl<T: Config> ProvideInherent for Pallet<T> {
        type Call = Call<T>;
        type Error = InherentError;
        const INHERENT_IDENTIFIER: InherentIdentifier = super::INHERENT_IDENTIFIER;

        fn create_inherent(data: &InherentData) -> Option<Self::Call> {
            let record = data
                .get_data::<super::ParticipationRecord>(&super::INHERENT_IDENTIFIER)
                .ok()
                .flatten()?;
            Some(Call::note_participation { record })
        }

        fn is_inherent_required(_: &InherentData) -> Result<Option<Self::Error>, Self::Error> {
            Ok(Some(InherentError::Missing)) // mandatory: block without it is rejected
        }

        fn is_inherent(call: &Self::Call) -> bool {
            matches!(call, Call::note_participation { .. })
        }
    }
}

#[cfg(test)]
mod tests {
    //! P-1/#838 author-seal: `note_participation` credits authorship ONLY to a vote-key holder whose cold
    //! account is committee-eligible, and does so via SOFT-DROP (a bad/non-committee author is not credited
    //! but the inherent still succeeds — no halt of the mandatory inherent). Both rejection branches covered.
    use crate as pallet_participation;
    use crate::{
        author_message, AuthoredInEpoch, BlockAuthor, CommitteeForEpoch, CommitteeInputs,
        ConsensusReputation, ParticipationRecord,
    };
    use frame::deps::sp_core::{sr25519, Pair};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Participation: pallet_participation,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    // Committee membership is seeded directly (see `seat`), so reputation is never consulted here.
    pub struct MockReputation;
    impl ConsensusReputation<u64> for MockReputation {
        fn consensus_reputation() -> alloc::vec::Vec<(u64, i128)> {
            Default::default()
        }
    }
    // vote_key_owner returns None → the identity path: the credited cold account == the signing hot key.
    pub struct MockCommittee;
    impl CommitteeInputs for MockCommittee {
        fn beacon_seed() -> [u8; 32] {
            [0u8; 32]
        }
        fn beacon_committed_keys() -> alloc::vec::Vec<([u8; 32], alloc::string::String)> {
            Default::default()
        }
        fn entrenchment_halted() -> bool {
            false
        }
        fn vote_key_owner(_hot: &[u8; 32]) -> Option<[u8; 32]> {
            None
        }
    }

    impl pallet_participation::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type EpochLength = ConstU64<10>;
        type EraLength = ConstU64<10>;
        type MaxCommittee = ConstU32<64>;
        type Reputation = MockReputation;
        type Committee = MockCommittee;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    /// Seat `key` in the committee that governs block `now` (`is_member` reads `CommitteeForEpoch`).
    fn seat(now: u64, key: [u8; 32]) {
        let cepoch = now / crate::COMMITTEE_EPOCH_LENGTH;
        let bv: BoundedVec<[u8; 32], ConstU32<64>> = alloc::vec![key].try_into().unwrap();
        CommitteeForEpoch::<Test>::insert(cepoch, bv);
    }
    fn record(author_key: [u8; 32], author_sig: [u8; 64], at: u64) -> ParticipationRecord {
        ParticipationRecord { author_key, author_sig, finality_votes: alloc::vec::Vec::new(), at_block: at }
    }
    /// `account_from_bytes` decodes the u64 mock AccountId from the first 8 LE bytes of the key.
    fn acct_of(key: &[u8; 32]) -> u64 {
        u64::from_le_bytes(key[..8].try_into().unwrap())
    }

    #[test]
    fn honest_committee_author_is_credited() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            let pair = sr25519::Pair::from_seed(&[1u8; 32]);
            let key = pair.public().0;
            seat(1, key);
            let sig = pair.sign(&author_message(1)).0;
            assert_ok!(Participation::note_participation(RuntimeOrigin::none(), record(key, sig, 1)));
            let who = acct_of(&key);
            assert_eq!(AuthoredInEpoch::<Test>::get(0, who), 1, "honest committee author must be credited");
            assert_eq!(BlockAuthor::<Test>::get(), Some(who));
        });
    }

    #[test]
    fn forged_author_bad_signature_not_credited_soft_drop() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            let pair = sr25519::Pair::from_seed(&[2u8; 32]);
            let key = pair.public().0;
            seat(1, key); // even seated, an INVALID signature must not credit
            // Soft-drop: bad sig → no credit, but the MANDATORY inherent still returns Ok (block valid, no halt).
            assert_ok!(Participation::note_participation(RuntimeOrigin::none(), record(key, [0u8; 64], 1)));
            assert_eq!(AuthoredInEpoch::<Test>::get(0, acct_of(&key)), 0, "bad-sig author must NOT be credited");
            assert_eq!(BlockAuthor::<Test>::get(), None);
        });
    }

    #[test]
    fn forged_author_non_committee_not_credited_soft_drop() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            let pair = sr25519::Pair::from_seed(&[3u8; 32]);
            let key = pair.public().0;
            // Valid signature, but the key is NOT seated in the committee → not credited, inherent still Ok.
            let sig = pair.sign(&author_message(1)).0;
            assert_ok!(Participation::note_participation(RuntimeOrigin::none(), record(key, sig, 1)));
            assert_eq!(AuthoredInEpoch::<Test>::get(0, acct_of(&key)), 0, "non-committee author must NOT be credited");
            assert_eq!(BlockAuthor::<Test>::get(), None);
        });
    }
}
