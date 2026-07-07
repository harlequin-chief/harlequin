//! Harlequin beacon pallet — un-grindable sortition randomness (macroaudit §2.1).
//!
//! The sortition's simulated VRF is grindable: the secret key is a freely-chosen string, so a node that
//! knows the epoch seed can grind ids offline to lift its committee/jury seats above its fair share
//! (`consensus-core::sortition_fp` known-defect test). This pallet removes that **without any new
//! dependency** by putting a **commit–reveal randomness beacon** on chain — a thin wrapper over the
//! validated `consensus_core::beacon` core (the deferred RANDAO pipeline `BeaconState` models exactly
//! this flow; dep-free core first, the proven project pattern).
//!
//! Pipeline (one node, across epochs):
//!  1. `commit(H(s_next))` — bind a fresh per-epoch secret BEFORE the next beacon is folded.
//!  2. `reveal(s)` — open last epoch's commitment; the secret is staged.
//!  3. `roll` (automatic at each epoch boundary, like the reputation recompute — **no privileged
//!     trigger, no king**) folds the staged reveals into the rolling beacon (deferred seed) and promotes
//!     them to this epoch's **active committed keys**. The draw value `H(s | beacon)` is over a secret
//!     committed a full epoch earlier → not grindable against the seed. The reputation anchor stays the
//!     real Sybil defence (zero reputation → zero seats regardless).
//!
//! The justice / consensus draw reads [`Pallet::seed_hex`] + [`Pallet::committed_key`] / [`Pallet::is_active`]
//! instead of the parent-hash seed and the freely-chosen `sk-{id}` (increment 2b wiring).
//!
//! ## Pre-mainnet notes (honest limitations)
//! - **Weights are placeholders**, not benchmarked. The per-epoch `roll` scans the participant maps; like
//!   the reputation recompute it moves to bounded/offchain handling before untrusted large state.
//! - **Last-revealer residual:** the last revealer of an epoch can withhold to veto its own contribution
//!   to the *next* beacon (a known RANDAO weakness) — it can never substitute a post-seed key for the
//!   *current* draw (the commitment blocks that). A VDF over the fold is the documented later hardening.
//! - **`EpochLength` MUST equal the reputation pallet's** so the beacon roll aligns with the recompute.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

#[frame::pallet]
pub mod pallet {
    use consensus_core::beacon;
    use frame::prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Max bytes of a revealed secret (a per-epoch random secret; 32 is plenty, allow a margin).
        #[pallet::constant]
        type MaxSecretLen: Get<u32>;

        /// Blocks per epoch. The beacon **rolls automatically** at each boundary (`on_initialize`), with NO
        /// privileged trigger. MUST equal the reputation pallet's `EpochLength` so the roll aligns with the
        /// recompute (the active committed keys and the reputation snapshot belong to the same epoch).
        #[pallet::constant]
        type EpochLength: Get<BlockNumberFor<Self>>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// The rolling beacon — the seed for THIS epoch's draws (hex via [`Pallet::seed_hex`]).
    #[pallet::storage]
    pub type Beacon<T> = StorageValue<_, [u8; 32], ValueQuery>;

    /// Commitments made this epoch (`account → H(secret)`), awaited for reveal next epoch.
    #[pallet::storage]
    pub type Pending<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 32], OptionQuery>;

    /// Commitments from last epoch, against which this epoch's reveals are checked.
    #[pallet::storage]
    pub type Awaiting<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 32], OptionQuery>;

    /// Reveals staged this epoch that opened an awaited commitment.
    #[pallet::storage]
    pub type Staged<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BoundedVec<u8, T::MaxSecretLen>, OptionQuery>;

    /// Secrets active for THIS epoch's draws (set at `roll` from the staged reveals). The committed
    /// sortition key of a node is `hex(H(secret))` — derived on read by [`Pallet::committed_key`].
    #[pallet::storage]
    pub type Active<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BoundedVec<u8, T::MaxSecretLen>, OptionQuery>;

    /// Genesis: the deferred genesis beacon (e.g. the committed BTC-block hash mixed with the founder
    /// phrases + VDF). All-zero = start from zero (the first reveals fold in from there).
    #[pallet::genesis_config]
    #[derive(frame::prelude::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub genesis_beacon: [u8; 32],
        #[serde(skip)]
        pub _marker: core::marker::PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            Beacon::<T>::put(self.genesis_beacon);
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// `who` committed `H(secret)` for the next roll.
        Committed { who: T::AccountId },
        /// `who` revealed a secret that opened its awaited commitment.
        Revealed { who: T::AccountId },
        /// The beacon rolled at an epoch boundary: the new seed and how many keys went active.
        BeaconRolled { epoch_seed: [u8; 32], active: u32 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The reveal does not open the account's awaited commitment (or there is none awaited).
        BadReveal,
        /// The revealed secret exceeds `MaxSecretLen`.
        SecretTooLong,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// No-king beacon roll: at every epoch boundary the beacon advances by ITSELF — no privileged
        /// origin presses a button. Mirrors the reputation recompute cadence.
        fn on_initialize(n: BlockNumberFor<T>) -> Weight {
            let len = T::EpochLength::get();
            if !n.is_zero() && !len.is_zero() && (n % len).is_zero() {
                Self::roll();
            }
            Weight::from_parts(1_000_000, 0)
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// **COMMIT** `H(secret)` for the next roll. Replaces any earlier commitment this epoch (a node
        /// holds one outstanding commitment, so it cannot register many cheap pseudo-commitments).
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn commit(origin: OriginFor<T>, commitment: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Pending::<T>::insert(&who, commitment);
            Self::deposit_event(Event::Committed { who });
            Ok(())
        }

        /// **REVEAL** the secret committed last epoch. Accepted iff it opens the account's awaited
        /// commitment; staged to fold into the beacon at the next roll.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn reveal(origin: OriginFor<T>, secret: alloc::vec::Vec<u8>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let c = Awaiting::<T>::get(&who).ok_or(Error::<T>::BadReveal)?;
            ensure!(beacon::verify_reveal(&c, &secret), Error::<T>::BadReveal);
            let bounded: BoundedVec<u8, T::MaxSecretLen> =
                secret.try_into().map_err(|_| Error::<T>::SecretTooLong)?;
            Staged::<T>::insert(&who, bounded);
            Self::deposit_event(Event::Revealed { who });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// **ROLL** one epoch boundary: fold the staged reveals into the beacon (deferred seed), promote
        /// them to the active committed keys for the new epoch, and advance the pipeline (this epoch's
        /// `Pending` commitments become next epoch's `Awaiting`). Deterministic (`iter` is ordered).
        pub(crate) fn roll() {
            use alloc::vec::Vec;
            // 1. staged reveals in deterministic (account) order.
            let staged: Vec<(T::AccountId, Vec<u8>)> =
                Staged::<T>::iter().map(|(k, v)| (k, v.into_inner())).collect();
            let contributions: Vec<Vec<u8>> = staged.iter().map(|(_, s)| s.clone()).collect();
            // 2. fold into the rolling beacon.
            let new = beacon::beacon_seed(&Beacon::<T>::get(), &contributions);
            Beacon::<T>::put(new);
            // 3. promote staged → active (clear last epoch's active first).
            let _ = Active::<T>::clear(u32::MAX, None);
            let mut count = 0u32;
            for (who, secret) in staged {
                if let Ok(b) = BoundedVec::try_from(secret) {
                    Active::<T>::insert(&who, b);
                    count = count.saturating_add(1);
                }
            }
            // 4. advance pipeline: Pending → Awaiting; clear Pending + Staged.
            let _ = Awaiting::<T>::clear(u32::MAX, None);
            for (who, c) in Pending::<T>::iter() {
                Awaiting::<T>::insert(&who, c);
            }
            let _ = Pending::<T>::clear(u32::MAX, None);
            let _ = Staged::<T>::clear(u32::MAX, None);
            Self::deposit_event(Event::BeaconRolled { epoch_seed: new, active: count });
        }

        /// Current beacon seed (hex) for the sortition draw.
        pub fn seed_hex() -> alloc::string::String {
            beacon::hex(&Beacon::<T>::get())
        }

        /// The committed sortition key (hex) of `who` for this epoch, if it revealed; else `None`.
        /// Drop-in as the `secret_key` for `consensus_core::sortition_fp::elect_committee_fp`.
        pub fn committed_key(who: &T::AccountId) -> Option<alloc::string::String> {
            Active::<T>::get(who)
                .map(|s| beacon::hex(&consensus_core::sha256::sha256(&s.into_inner())))
        }

        /// Whether `who` has an active committed key this epoch (revealed its last commitment).
        pub fn is_active(who: &T::AccountId) -> bool {
            Active::<T>::contains_key(who)
        }

        /// The current beacon as raw 32 bytes (for the consensus runtime API / node committee draw).
        pub fn beacon_raw() -> [u8; 32] {
            Beacon::<T>::get()
        }

        /// Every account with an active committed key this epoch → `(account, hex key)`. Feeds the node's
        /// committee sortition (via the consensus runtime API) so it keys on committed secrets, not ids.
        pub fn active_committed_keys() -> alloc::vec::Vec<(T::AccountId, alloc::string::String)> {
            Active::<T>::iter()
                .map(|(who, s)| {
                    (who, beacon::hex(&consensus_core::sha256::sha256(&s.into_inner())))
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_beacon;
    use crate::{Awaiting, Beacon, Error};
    use consensus_core::beacon::{commit as mk_commit, hex};
    use consensus_core::sha256::sha256;
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            BeaconPallet: pallet_beacon,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    impl pallet_beacon::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type MaxSecretLen = ConstU32<64>;
        type EpochLength = ConstU64<10>;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    fn secret(tag: &str) -> alloc::vec::Vec<u8> {
        sha256(tag.as_bytes()).to_vec()
    }

    #[test]
    fn commit_then_reveal_next_epoch_activates_key() {
        new_test_ext().execute_with(|| {
            let s = secret("v1-e1");
            // commit in epoch 1.
            assert_ok!(BeaconPallet::commit(RuntimeOrigin::signed(1), mk_commit(&s)));
            // a reveal before the commitment is awaited fails.
            assert_noop!(
                BeaconPallet::reveal(RuntimeOrigin::signed(1), s.clone()),
                Error::<Test>::BadReveal
            );
            BeaconPallet::roll(); // epoch 1 → 2: the commitment is now awaited
            assert!(!BeaconPallet::is_active(&1));
            assert!(Awaiting::<Test>::contains_key(1u64));
            // now the committed secret opens it; a wrong secret does not.
            assert_noop!(
                BeaconPallet::reveal(RuntimeOrigin::signed(1), secret("wrong")),
                Error::<Test>::BadReveal
            );
            assert_ok!(BeaconPallet::reveal(RuntimeOrigin::signed(1), s.clone()));
            BeaconPallet::roll(); // epoch 2 → 3: reveal folds in, key goes active
            assert!(BeaconPallet::is_active(&1));
            assert_eq!(BeaconPallet::committed_key(&1), Some(hex(&sha256(&s))));
        });
    }

    #[test]
    fn beacon_advances_only_on_folded_reveals() {
        new_test_ext().execute_with(|| {
            let seed0 = Beacon::<Test>::get();
            let s = secret("c");
            assert_ok!(BeaconPallet::commit(RuntimeOrigin::signed(2), mk_commit(&s)));
            BeaconPallet::roll();
            let seed_empty = Beacon::<Test>::get(); // no reveals folded yet
            assert_ok!(BeaconPallet::reveal(RuntimeOrigin::signed(2), s));
            BeaconPallet::roll();
            let seed_revealed = Beacon::<Test>::get();
            assert_eq!(seed0, seed_empty, "an empty roll does not move the beacon");
            assert_ne!(seed_empty, seed_revealed, "a folded reveal advances the beacon");
        });
    }

    #[test]
    fn roll_drops_non_revealers_and_advances_pipeline() {
        new_test_ext().execute_with(|| {
            // three nodes commit in epoch 1.
            for i in 1..=3u64 {
                assert_ok!(BeaconPallet::commit(
                    RuntimeOrigin::signed(i),
                    mk_commit(&secret(&alloc::format!("sk-{i}")))
                ));
            }
            BeaconPallet::roll(); // commitments awaited
            // only 1 and 2 reveal.
            assert_ok!(BeaconPallet::reveal(RuntimeOrigin::signed(1), secret("sk-1")));
            assert_ok!(BeaconPallet::reveal(RuntimeOrigin::signed(2), secret("sk-2")));
            BeaconPallet::roll(); // fold → active
            assert!(BeaconPallet::is_active(&1));
            assert!(BeaconPallet::is_active(&2));
            assert!(!BeaconPallet::is_active(&3), "the non-revealer has no active key");
        });
    }

    #[test]
    fn second_commit_replaces_first_within_epoch() {
        new_test_ext().execute_with(|| {
            assert_ok!(BeaconPallet::commit(RuntimeOrigin::signed(1), mk_commit(&secret("throwaway"))));
            let real = secret("real");
            assert_ok!(BeaconPallet::commit(RuntimeOrigin::signed(1), mk_commit(&real)));
            BeaconPallet::roll();
            // the replaced commitment cannot be opened; only the latest is awaited.
            assert_noop!(
                BeaconPallet::reveal(RuntimeOrigin::signed(1), secret("throwaway")),
                Error::<Test>::BadReveal
            );
            assert_ok!(BeaconPallet::reveal(RuntimeOrigin::signed(1), real));
        });
    }

    #[test]
    fn beacon_rolls_automatically_at_epoch_boundary_no_trigger() {
        new_test_ext().execute_with(|| {
            let s = secret("auto");
            assert_ok!(BeaconPallet::commit(RuntimeOrigin::signed(1), mk_commit(&s)));
            // a non-boundary block does nothing (EpochLength = 10 in tests).
            BeaconPallet::on_initialize(5u64);
            assert!(!Awaiting::<Test>::contains_key(1u64));
            // the boundary rolls by ITSELF — no origin, no king.
            BeaconPallet::on_initialize(10u64);
            assert!(Awaiting::<Test>::contains_key(1u64));
            assert_ok!(BeaconPallet::reveal(RuntimeOrigin::signed(1), s));
            BeaconPallet::on_initialize(20u64);
            assert!(BeaconPallet::is_active(&1));
        });
    }

    #[test]
    fn genesis_seeds_the_beacon() {
        let mut ext: TestState = RuntimeGenesisConfig {
            system: Default::default(),
            beacon_pallet: pallet_beacon::GenesisConfig {
                genesis_beacon: sha256(b"harlequin-genesis"),
                _marker: Default::default(),
            },
        }
        .build_storage()
        .unwrap()
        .into();
        ext.execute_with(|| {
            assert_eq!(Beacon::<Test>::get(), sha256(b"harlequin-genesis"));
        });
    }
}
