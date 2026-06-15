//! Harlequin reputation pallet — on-chain inputs, reputation DERIVED.
//!
//! The chain stores the **verifiable inputs** (objective evidence per suit + the vouch graph), never an
//! asserted reputation number. Reputation is recomputed from those inputs by `reputation-core` on the
//! deterministic fixed-point path — so every node can re-derive and reject a wrong result (no trusted
//! oracle of who is reputable). See `../PALLET-DESIGN.md`.
//!
//! This increment implements the **storage + input extrinsics** (the "inputs on-chain" half). The epoch
//! recompute that runs `reputation-core` over this storage and writes `ReputationSnapshot` is the next
//! increment (PALLET-DESIGN "Epoch flow": offchain-worker recompute, verify-by-recompute).

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

/// The four reputation dimensions = the four suits of Harlequin (LORE.md, canon). Fixed set.
#[derive(
    codec::Encode,
    codec::Decode,
    codec::DecodeWithMemTracking,
    codec::MaxEncodedLen,
    scale_info::TypeInfo,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
)]
pub enum Suit {
    /// ♦ commerce / ambition
    Commerce,
    /// ♣ technical contribution / freedom
    Technical,
    /// ♠ judicial function / struggle
    Judicial,
    /// ♥ governance / love
    Governance,
}

#[frame::pallet]
pub mod pallet {
    use super::Suit;
    use frame::prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Origin allowed to record validated evidence. Evidence is NOT self-asserted: what counts as
        /// proven work is pluggable (a settled-deal proof, a completed-job attestation, an oracle, a
        /// governance body…), exactly like personhood is pluggable in SPEC §1.5. The pallet does not
        /// hard-code it — it trusts this origin to have verified the proof.
        type EvidenceOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Max live vouches a single account can hold (bounds storage / PoV).
        #[pallet::constant]
        type MaxVouches: Get<u32>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Objective verifiable evidence per (account, suit) — the bootstrap anchor of reputation (§1.3a).
    #[pallet::storage]
    pub type Evidence<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, T::AccountId, Blake2_128Concat, Suit, u128, ValueQuery>;

    /// The vouch graph (§1.3b): for each account, the edges it casts — (target, suit, weight). Putting
    /// reputation at stake on someone; persistent liability lives alongside (§1.5c, next increment).
    #[pallet::storage]
    pub type Vouches<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BoundedVec<(T::AccountId, Suit, u32), T::MaxVouches>,
        ValueQuery,
    >;

    /// Last epoch's computed reputation per (account, suit), raw fixed-point i128 (what consensus and
    /// justice READ). Written by the epoch recompute (next increment); read-only between epochs.
    #[pallet::storage]
    pub type ReputationSnapshot<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, T::AccountId, Blake2_128Concat, Suit, i128, ValueQuery>;

    /// Current epoch number (drives recompute cadence + committee rotation, SPEC §2.2).
    #[pallet::storage]
    pub type Epoch<T> = StorageValue<_, u64, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Validated evidence was recorded for `who` in `suit` (new cumulative total).
        EvidenceRecorded { who: T::AccountId, suit: Suit, total: u128 },
        /// `voucher` vouched for `target` in `suit` with `weight`.
        Vouched { voucher: T::AccountId, target: T::AccountId, suit: Suit, weight: u32 },
        /// `voucher` revoked its vouch(es) for `target` in `suit`.
        VouchRevoked { voucher: T::AccountId, target: T::AccountId, suit: Suit },
        /// A new epoch began.
        EpochAdvanced { epoch: u64 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Nobody vouches for themselves (§1.3b).
        SelfVouch,
        /// The caller has reached its vouch capacity.
        TooManyVouches,
        /// Vouch weight must be non-zero.
        ZeroWeight,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Record validated objective evidence for `who` in `suit` (adds to the running total). Gated by
        /// `EvidenceOrigin` — the pluggable proof authority (§1.3a). Not self-asserted.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn submit_evidence(
            origin: OriginFor<T>,
            who: T::AccountId,
            suit: Suit,
            amount: u128,
        ) -> DispatchResult {
            T::EvidenceOrigin::ensure_origin(origin)?;
            let total = Evidence::<T>::mutate(&who, suit, |e| {
                *e = e.saturating_add(amount);
                *e
            });
            Self::deposit_event(Event::EvidenceRecorded { who, suit, total });
            Ok(())
        }

        /// Vouch for `target` in `suit` with `weight` — put reputation at stake on them (§1.3b).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn vouch(
            origin: OriginFor<T>,
            target: T::AccountId,
            suit: Suit,
            weight: u32,
        ) -> DispatchResult {
            let voucher = ensure_signed(origin)?;
            ensure!(voucher != target, Error::<T>::SelfVouch);
            ensure!(weight > 0, Error::<T>::ZeroWeight);
            Vouches::<T>::try_mutate(&voucher, |edges| {
                edges
                    .try_push((target.clone(), suit, weight))
                    .map_err(|_| Error::<T>::TooManyVouches)
            })?;
            Self::deposit_event(Event::Vouched { voucher, target, suit, weight });
            Ok(())
        }

        /// Revoke all of the caller's vouches for `target` in `suit`.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn revoke_vouch(
            origin: OriginFor<T>,
            target: T::AccountId,
            suit: Suit,
        ) -> DispatchResult {
            let voucher = ensure_signed(origin)?;
            Vouches::<T>::mutate(&voucher, |edges| {
                edges.retain(|(t, s, _)| !(*t == target && *s == suit));
            });
            Self::deposit_event(Event::VouchRevoked { voucher, target, suit });
            Ok(())
        }
    }
}
