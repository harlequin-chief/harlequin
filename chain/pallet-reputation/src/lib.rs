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

extern crate alloc;

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
    serde::Serialize,
    serde::Deserialize,
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

impl Suit {
    /// All four suits, canonical order.
    pub const ALL: [Suit; 4] = [Suit::Commerce, Suit::Technical, Suit::Judicial, Suit::Governance];

    /// The reputation-core dimension name. MUST match `reputation_core::DIMENSIONS` exactly.
    pub const fn dim_name(&self) -> &'static str {
        match self {
            Suit::Commerce => "commerce",
            Suit::Technical => "technical_contribution",
            Suit::Judicial => "judicial_function",
            Suit::Governance => "governance",
        }
    }
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

    /// Genesis: seed the founding cohort's objective evidence (§1.4), so the chain boots with members
    /// who already have standing to bootstrap trust. Nothing here is reputation — only the evidence
    /// inputs; reputation is still DERIVED on the first epoch recompute.
    #[pallet::genesis_config]
    #[derive(frame::prelude::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        /// `(account, suit, evidence_amount)` granted at genesis.
        pub evidence: alloc::vec::Vec<(T::AccountId, Suit, u128)>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            for (who, suit, amount) in &self.evidence {
                Evidence::<T>::insert(who, *suit, *amount);
            }
        }
    }

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

        /// Close the epoch: recompute every member's reputation from the on-chain inputs, write the new
        /// `ReputationSnapshot`, and advance the epoch counter. Root-only for now (a testnet trigger);
        /// production moves the recompute to an offchain worker (PALLET-DESIGN "Epoch flow").
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(1_000_000_000, 0))]
        pub fn advance_epoch(origin: OriginFor<T>) -> DispatchResult {
            ensure_root(origin)?;
            Self::recompute_reputation();
            let epoch = Epoch::<T>::mutate(|e| {
                *e = e.saturating_add(1);
                *e
            });
            Self::deposit_event(Event::EpochAdvanced { epoch });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Recompute every member's reputation from the on-chain inputs (`Evidence` + `Vouches`) and
        /// overwrite `ReputationSnapshot`. Deterministic — it runs `reputation-core` on the fixed-point
        /// path, so every node derives the same numbers and can reject a wrong snapshot. The chain
        /// stores INPUTS, never an asserted reputation (no oracle).
        ///
        /// HEAVY (EigenTrust over the whole graph): this in-runtime version is for testnet/triggering;
        /// production runs it in an offchain worker and verifies by recompute.
        pub fn recompute_reputation() {
            use alloc::collections::{BTreeMap, BTreeSet};
            use alloc::string::ToString;
            use alloc::vec::Vec;
            use reputation_core::{reputation_dimension_fully_fixed_fp, Agent, Params, TrustGraph};

            // 1. participants: anyone with evidence, who vouches, or who is vouched for.
            let mut set: BTreeSet<T::AccountId> = BTreeSet::new();
            for (acc, _suit, _v) in Evidence::<T>::iter() {
                set.insert(acc);
            }
            for (acc, edges) in Vouches::<T>::iter() {
                set.insert(acc);
                for (target, _s, _w) in edges.into_iter() {
                    set.insert(target);
                }
            }
            let accounts: Vec<T::AccountId> = set.into_iter().collect();
            if accounts.is_empty() {
                return;
            }
            // reputation-core keys agents by short string ids = their index in `accounts`.
            let mut idx: BTreeMap<T::AccountId, usize> = BTreeMap::new();
            for (i, a) in accounts.iter().enumerate() {
                idx.insert(a.clone(), i);
            }

            // 2. agents (evidence per suit)
            let mut agents: Vec<Agent> = Vec::with_capacity(accounts.len());
            for (i, acc) in accounts.iter().enumerate() {
                let mut agent = Agent::new(&i.to_string());
                for suit in Suit::ALL {
                    let ev = Evidence::<T>::get(acc, suit);
                    if ev > 0 {
                        agent = agent.with_evidence(suit.dim_name(), ev as f64);
                    }
                }
                agents.push(agent);
            }

            // 3. trust graph (vouches)
            let mut graph = TrustGraph::new();
            for (acc, edges) in Vouches::<T>::iter() {
                let si = match idx.get(&acc) {
                    Some(s) => *s,
                    None => continue,
                };
                for (target, suit, weight) in edges.into_iter() {
                    if let Some(ti) = idx.get(&target) {
                        graph.attest(&si.to_string(), &ti.to_string(), suit.dim_name(), weight as f64);
                    }
                }
            }

            // 4. compute per suit (deterministic fixed-point) and overwrite the snapshot.
            let params = Params { community: true, in_concentration: true, ..Default::default() };
            for suit in Suit::ALL {
                let rep = reputation_dimension_fully_fixed_fp(&agents, &graph, suit.dim_name(), &params);
                for (id, value) in rep.into_iter() {
                    if let Ok(i) = id.parse::<usize>() {
                        if let Some(acc) = accounts.get(i) {
                            ReputationSnapshot::<T>::insert(acc, suit, value);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_reputation;
    use crate::{Epoch, Error, Evidence, ReputationSnapshot, Suit};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Reputation: pallet_reputation,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    impl pallet_reputation::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type EvidenceOrigin = EnsureRoot<Self::AccountId>;
        type MaxVouches = ConstU32<64>;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    #[test]
    fn recompute_writes_snapshot_and_advances_epoch() {
        new_test_ext().execute_with(|| {
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 1u64, Suit::Commerce, 5));
            assert_ok!(Reputation::submit_evidence(RuntimeOrigin::root(), 2u64, Suit::Commerce, 5));
            assert_ok!(Reputation::vouch(RuntimeOrigin::signed(1), 2u64, Suit::Commerce, 1));
            assert_ok!(Reputation::advance_epoch(RuntimeOrigin::root()));
            assert_eq!(Epoch::<Test>::get(), 1);
            // members who put in evidence/vouches earn standing
            assert!(ReputationSnapshot::<Test>::get(1u64, Suit::Commerce) > 0);
            assert!(ReputationSnapshot::<Test>::get(2u64, Suit::Commerce) > 0);
            // an account that contributed nothing has zero standing (reputation is EARNED)
            assert_eq!(ReputationSnapshot::<Test>::get(99u64, Suit::Commerce), 0);
        });
    }

    #[test]
    fn evidence_requires_privileged_origin() {
        new_test_ext().execute_with(|| {
            // evidence is not self-asserted: a plain signed origin cannot record it
            assert_noop!(
                Reputation::submit_evidence(RuntimeOrigin::signed(1), 1u64, Suit::Commerce, 5),
                DispatchError::BadOrigin
            );
        });
    }

    #[test]
    fn nobody_vouches_for_themselves() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                Reputation::vouch(RuntimeOrigin::signed(1), 1u64, Suit::Commerce, 1),
                Error::<Test>::SelfVouch
            );
        });
    }

    #[test]
    fn genesis_seeds_the_founding_cohort() {
        // the chain boots with the founding cohort's evidence already in place (§1.4)
        let mut ext: TestState = RuntimeGenesisConfig {
            system: Default::default(),
            reputation: pallet_reputation::GenesisConfig {
                evidence: vec![(1u64, Suit::Commerce, 5), (2u64, Suit::Governance, 3)],
            },
        }
        .build_storage()
        .unwrap()
        .into();
        ext.execute_with(|| {
            assert_eq!(Evidence::<Test>::get(1u64, Suit::Commerce), 5);
            assert_eq!(Evidence::<Test>::get(2u64, Suit::Governance), 3);
            // and the founders earn reputation on the first epoch recompute
            assert_ok!(Reputation::advance_epoch(RuntimeOrigin::root()));
            assert!(ReputationSnapshot::<Test>::get(1u64, Suit::Commerce) > 0);
        });
    }
}
