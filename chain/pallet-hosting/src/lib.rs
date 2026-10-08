//! Harlequin **hosting** — the input of the hosting reward (design/COBRO-POR-HOSPEDAR-E1-2026-10-02.md, E1).
//!
//! Today only committee service is paid, and the committee is drawn by reputation: a node that only runs the software
//! starts at zero and can never earn (measured: half to four fifths of nodes never get paid). This pallet lets a node
//! that is BOUND to a mask (`pallet-reputation::set_vote_key`) prove once a day that it follows the chain:
//!
//! * `heartbeat(at, hash, hot, sig)` — UNSIGNED (the hot key holds no coins), carrying an sr25519 signature by the
//!   node's hot session key over `"hlq-heartbeat-v1" ‖ hot ‖ at (u32 LE) ‖ hash`, where `hash` is the hash of block
//!   `at`, at most `MaxBeatAge` blocks old. The hot key is resolved to its mask through the vote-key reverse map.
//! * One beat per mask per day (`DayLength`). A gap longer than `MaxGapDays` restarts the streak.
//! * Only after `MinStreakDays` of unbroken beats do days COUNT: `HostDays(era, mask) += 1`. Farms cost time.
//!
//! `pallet-tokens` reads [`Pallet::era_hosts`] at the era boundary and shares the hosting pool (a fixed slice of the
//! HLQ tranche) among masks by days present, then calls [`Pallet::clear_era`]. Hosting pays presence, never power: it
//! gives no reputation and no committee seat. What it cannot prove — that the machine really stores the chain rather
//! than asking another node for the latest block — is bounded by design: farmers can take at most the pool's slice.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// Domain label of the heartbeat message: never valid as a transaction, a login, a bind proof or a pact signature.
pub const HEARTBEAT_LABEL: &[u8; 16] = b"hlq-heartbeat-v1";

/// The exact bytes the node's hot key signs: `label ‖ hot pubkey ‖ at (u32 LE) ‖ block hash` = 84 bytes.
pub fn heartbeat_message(hot: &[u8; 32], at: u32, hash: &[u8; 32]) -> [u8; 84] {
    let mut m = [0u8; 84];
    m[..16].copy_from_slice(HEARTBEAT_LABEL);
    m[16..48].copy_from_slice(hot);
    m[48..52].copy_from_slice(&at.to_le_bytes());
    m[52..].copy_from_slice(hash);
    m
}

/// hot session pubkey → its mask (the runtime wires `pallet-reputation::VoteKeyOwner`).
pub trait MaskOfHotKey<AccountId> {
    fn mask_of(hot: &[u8; 32]) -> Option<AccountId>;
}

#[frame::pallet]
pub mod pallet {
    use super::{heartbeat_message, MaskOfHotKey};
    use frame::prelude::*;

    #[derive(codec::Encode, codec::Decode, codec::MaxEncodedLen, scale_info::TypeInfo, Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct BeatState {
        /// Day index of the last counted beat (block / DayLength).
        pub last_day: u32,
        /// Day the current unbroken streak started.
        pub streak_start: u32,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Resolves a hot session key to the mask it is bound to.
        type MaskOf: MaskOfHotKey<Self::AccountId>;
        /// Blocks per day (7,200 at 12 s).
        #[pallet::constant]
        type DayLength: Get<BlockNumberFor<Self>>;
        /// Blocks per reward era — MUST equal pallet-tokens' and pallet-participation's `EraLength`.
        #[pallet::constant]
        type EraLength: Get<BlockNumberFor<Self>>;
        /// Unbroken days before days start counting.
        #[pallet::constant]
        type MinStreakDays: Get<u32>;
        /// Longest gap (days) that does not break a streak.
        #[pallet::constant]
        type MaxGapDays: Get<u32>;
        /// Oldest block a beat may point at.
        #[pallet::constant]
        type MaxBeatAge: Get<BlockNumberFor<Self>>;
        /// Pool priority of a valid beat (low: it never competes with user transactions).
        #[pallet::constant]
        type UnsignedPriority: Get<TransactionPriority>;
        /// Most masks counted in one era. Bounds the era-end read and clear (review 2-oct: they run unmetered in
        /// pallet-tokens' on_finalize). A mask already counted keeps counting; new ones wait for the next era.
        #[pallet::constant]
        type MaxHostsPerEra: Get<u32>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// `mask → streak state`.
    #[pallet::storage]
    pub type Beats<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, BeatState, OptionQuery>;

    /// `era → number of masks with counted days` (bound: `MaxHostsPerEra`).
    #[pallet::storage]
    pub type HostCount<T: Config> = StorageMap<_, Twox64Concat, u64, u32, ValueQuery>;

    /// `(era, mask) → counted days in that era`.
    #[pallet::storage]
    pub type HostDays<T: Config> =
        StorageDoubleMap<_, Twox64Concat, u64, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// `mask` beat on `day`; `counted` = the day counts toward the era's hosting pool.
        Beat { mask: T::AccountId, day: u32, counted: bool },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The hot key is not bound to any mask.
        NotBound,
        /// The block the beat points at is too old, in the future, or its hash does not match.
        StaleOrWrongBlock,
        /// Bad signature by the hot key.
        BadSignature,
        /// This mask already beat today.
        AlreadyToday,
    }

    impl<T: Config> Pallet<T> {
        fn day_of(n: BlockNumberFor<T>) -> u32 {
            let d = T::DayLength::get();
            if d.is_zero() { 0 } else { (n / d).saturated_into::<u32>() }
        }
        fn era_of(n: BlockNumberFor<T>) -> u64 {
            let e = T::EraLength::get();
            if e.is_zero() { 0 } else { (n / e).saturated_into::<u64>() }
        }
        /// Every check of a beat, shared by the pool validation and the dispatch (they must never disagree).
        fn check(at: u32, hash: &[u8; 32], hot: &[u8; 32], sig: &[u8; 64]) -> Result<(T::AccountId, u32), Error<T>> {
            // Cheapest checks FIRST (review 2-oct): this runs in every node's pool for free on any gossiped candidate,
            // so an unbound key, a stale block or a repeat of today must be rejected before any signature work.
            let now = frame_system::Pallet::<T>::block_number();
            let at_bn: BlockNumberFor<T> = at.into();
            ensure!(at_bn < now && now - at_bn <= T::MaxBeatAge::get(), Error::<T>::StaleOrWrongBlock);
            let mask = T::MaskOf::mask_of(hot).ok_or(Error::<T>::NotBound)?;
            let today = Self::day_of(now);
            if let Some(st) = Beats::<T>::get(&mask) {
                ensure!(st.last_day < today, Error::<T>::AlreadyToday);
            }
            let real = frame_system::Pallet::<T>::block_hash(at_bn);
            ensure!(real.as_ref() == &hash[..], Error::<T>::StaleOrWrongBlock);
            use frame::deps::sp_core::sr25519::{Public, Signature};
            let ok = frame::deps::sp_io::crypto::sr25519_verify(
                &Signature::from_raw(*sig),
                &heartbeat_message(hot, at, hash),
                &Public::from_raw(*hot),
            );
            ensure!(ok, Error::<T>::BadSignature);
            Ok((mask, today))
        }

        /// CONSUMER API (pallet-tokens, era boundary): `(mask, counted days)` for `era`.
        pub fn era_hosts(era: u64) -> alloc::vec::Vec<(T::AccountId, u64)> {
            HostDays::<T>::iter_prefix(era).map(|(m, d)| (m, d as u64)).collect()
        }
        /// Clear `era` once paid.
        pub fn clear_era(era: u64) {
            // bounded by MaxHostsPerEra (+1 margin): never an unbounded deletion
            let _ = HostDays::<T>::clear_prefix(era, T::MaxHostsPerEra::get().saturating_add(1), None);
            HostCount::<T>::remove(era);
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// The daily beat of a node bound to a mask. Unsigned; all checks in [`Pallet::check`].
        #[pallet::call_index(0)]
        #[pallet::weight(T::DbWeight::get().reads_writes(4, 2).saturating_add(Weight::from_parts(60_000_000, 300)))]
        pub fn heartbeat(origin: OriginFor<T>, at: u32, hash: [u8; 32], hot: [u8; 32], sig: [u8; 64]) -> DispatchResult {
            ensure_none(origin)?;
            let (mask, today) = Self::check(at, &hash, &hot, &sig)?;
            let prev = Beats::<T>::get(&mask);
            let streak_start = match prev {
                Some(st) if today.saturating_sub(st.last_day) <= T::MaxGapDays::get().saturating_add(1) => st.streak_start,
                _ => today,
            };
            Beats::<T>::insert(&mask, BeatState { last_day: today, streak_start });
            let counted = today.saturating_sub(streak_start) >= T::MinStreakDays::get();
            let mut counted = counted;
            if counted {
                let era = Self::era_of(frame_system::Pallet::<T>::block_number());
                if HostDays::<T>::contains_key(era, &mask) {
                    HostDays::<T>::mutate(era, &mask, |d| *d = d.saturating_add(1));
                } else if HostCount::<T>::get(era) < T::MaxHostsPerEra::get() {
                    HostDays::<T>::insert(era, &mask, 1);
                    HostCount::<T>::mutate(era, |c| *c = c.saturating_add(1));
                } else {
                    counted = false; // era full: the streak still advances, the day is not paid this era
                }
            }
            Self::deposit_event(Event::Beat { mask, day: today, counted });
            Ok(())
        }
    }

    #[pallet::validate_unsigned]
    impl<T: Config> ValidateUnsigned for Pallet<T> {
        type Call = Call<T>;
        fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
            let Call::heartbeat { at, hash, hot, sig } = call else {
                return InvalidTransaction::Call.into();
            };
            match Self::check(*at, hash, hot, sig) {
                Ok((mask, today)) => ValidTransaction::with_tag_prefix("HlqHeartbeat")
                    .priority(T::UnsignedPriority::get())
                    // one beat per mask per day in the pool, whatever node sends it
                    .and_provides((mask, today))
                    .longevity(64)
                    .propagate(true)
                    .build(),
                Err(Error::<T>::AlreadyToday) => InvalidTransaction::Stale.into(),
                Err(Error::<T>::NotBound) => InvalidTransaction::BadSigner.into(),
                Err(Error::<T>::BadSignature) => InvalidTransaction::BadProof.into(),
                Err(_) => InvalidTransaction::Stale.into(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_hosting;
    use crate::{heartbeat_message, Error, HostDays};
    use frame::deps::sp_core::{sr25519, Pair, H256};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Hosting: pallet_hosting,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    /// Test binding: hot key whose first byte is b ≥ 100 is bound to mask (b as u64); others are unbound.
    pub struct Bound;
    impl crate::MaskOfHotKey<u64> for Bound {
        fn mask_of(hot: &[u8; 32]) -> Option<u64> {
            KEYS.with(|k| k.borrow().iter().find(|(h, _)| h == hot).map(|(_, m)| *m))
        }
    }
    thread_local! { static KEYS: core::cell::RefCell<Vec<([u8; 32], u64)>> = core::cell::RefCell::new(Vec::new()); }

    impl pallet_hosting::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type MaskOf = Bound;
        type DayLength = ConstU64<10>;
        type EraLength = ConstU64<1000>;
        type MinStreakDays = ConstU32<3>;
        type MaxGapDays = ConstU32<1>;
        type MaxBeatAge = ConstU64<5>;
        type UnsignedPriority = ConstU64<1>;
        type MaxHostsPerEra = ConstU32<2>;
    }

    fn ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }
    fn node(seed: u8, mask: Option<u64>) -> sr25519::Pair {
        let p = sr25519::Pair::from_seed(&[seed; 32]);
        if let Some(m) = mask {
            KEYS.with(|k| k.borrow_mut().push((p.public().0, m)));
        }
        p
    }
    /// Move to block n, record the hash of n-1, and build a beat over it.
    fn at_block(n: u64) -> (u32, [u8; 32]) {
        System::set_block_number(n);
        let h = H256::repeat_byte((n % 251) as u8);
        frame_system::BlockHash::<Test>::insert(n - 1, h);
        ((n - 1) as u32, h.0)
    }
    fn beat(p: &sr25519::Pair, at: u32, hash: [u8; 32]) -> DispatchResult {
        let hot = p.public().0;
        let sig = p.sign(&heartbeat_message(&hot, at, &hash)).0;
        Hosting::heartbeat(RuntimeOrigin::none(), at, hash, hot, sig)
    }

    #[test]
    fn days_count_only_after_the_streak_and_once_a_day() {
        ext().execute_with(|| {
            let p = node(101, Some(7));
            for day in 0..6u64 {
                let (at, h) = at_block(day * 10 + 2);
                assert_ok!(beat(&p, at, h));
                if day == 0 {
                    let (at2, h2) = at_block(day * 10 + 4);
                    assert_noop!(beat(&p, at2, h2), Error::<Test>::AlreadyToday);
                }
            }
            // MinStreakDays = 3: days 3, 4 and 5 count, days 0..2 are the unpaid run-in
            assert_eq!(HostDays::<Test>::get(0u64, 7u64), 3);
            assert_eq!(Hosting::era_hosts(0), vec![(7u64, 3u64)]);
        });
    }

    #[test]
    fn a_gap_restarts_the_streak() {
        ext().execute_with(|| {
            let p = node(102, Some(8));
            for day in [0u64, 1, 2, 3] {
                let (at, h) = at_block(day * 10 + 1);
                assert_ok!(beat(&p, at, h));
            }
            assert_eq!(HostDays::<Test>::get(0u64, 8u64), 1); // day 3 counted
            // skip days 4,5,6 (gap 3 > MaxGapDays 1): day 7 restarts the streak, does not count
            let (at, h) = at_block(7 * 10 + 1);
            assert_ok!(beat(&p, at, h));
            assert_eq!(HostDays::<Test>::get(0u64, 8u64), 1);
            assert_eq!(crate::Beats::<Test>::get(8u64).unwrap().streak_start, 7);
        });
    }

    #[test]
    fn forged_stale_and_unbound_beats_are_refused() {
        ext().execute_with(|| {
            let p = node(103, Some(9));
            let stranger = node(104, None);
            let (at, h) = at_block(20);
            // signature by another key over the same message
            let hot = p.public().0;
            let bad = stranger.sign(&heartbeat_message(&hot, at, &h)).0;
            assert_noop!(Hosting::heartbeat(RuntimeOrigin::none(), at, h, hot, bad), Error::<Test>::BadSignature);
            // a key nobody bound
            assert_noop!(beat(&stranger, at, h), Error::<Test>::NotBound);
            // wrong hash for that block
            assert_noop!(beat(&p, at, [9u8; 32]), Error::<Test>::StaleOrWrongBlock);
            // too old: MaxBeatAge = 5
            System::set_block_number(40);
            assert_noop!(beat(&p, at, h), Error::<Test>::StaleOrWrongBlock);
            // a signed origin is not a beat
            let sig = p.sign(&heartbeat_message(&hot, at, &h)).0;
            assert!(Hosting::heartbeat(RuntimeOrigin::signed(1), at, h, hot, sig).is_err());
        });
    }

    #[test]
    fn the_pool_validation_agrees_with_the_dispatch() {
        use frame::deps::sp_runtime::transaction_validity::TransactionSource;
        ext().execute_with(|| {
            let p = node(105, Some(10));
            let (at, h) = at_block(30);
            let hot = p.public().0;
            let sig = p.sign(&heartbeat_message(&hot, at, &h)).0;
            let call = crate::Call::<Test>::heartbeat { at, hash: h, hot, sig };
            assert!(<Hosting as ValidateUnsigned>::validate_unsigned(TransactionSource::External, &call).is_ok());
            assert_ok!(Hosting::heartbeat(RuntimeOrigin::none(), at, h, hot, sig));
            // the same mask again today: the pool refuses it too
            assert!(<Hosting as ValidateUnsigned>::validate_unsigned(TransactionSource::External, &call).is_err());
        });
    }

    #[test]
    fn an_era_counts_at_most_max_hosts_and_unbound_is_refused_before_any_signature_check() {
        ext().execute_with(|| {
            let (a, b, c) = (node(110, Some(20)), node(111, Some(21)), node(112, Some(22)));
            for day in 0..5u64 {
                let (at, h) = at_block(day * 10 + 2);
                for p in [&a, &b, &c] {
                    assert_ok!(beat(p, at, h));
                }
            }
            // MaxHostsPerEra = 2: the first two masks counted are paid, the third waits for the next era
            assert_eq!(Hosting::era_hosts(0).len(), 2);
            assert_eq!(crate::HostCount::<Test>::get(0u64), 2);
            // an unbound key with a GARBAGE signature is refused as NotBound (cheap check first)
            let (at, h) = at_block(60);
            let stranger = node(113, None);
            assert_noop!(Hosting::heartbeat(RuntimeOrigin::none(), at, h, stranger.public().0, [0u8; 64]), Error::<Test>::NotBound);
            Hosting::clear_era(0);
            assert_eq!(crate::HostCount::<Test>::get(0u64), 0);
        });
    }

    #[test]
    fn clear_era_forgets_the_paid_era_only() {
        ext().execute_with(|| {
            HostDays::<Test>::insert(0u64, 1u64, 5);
            HostDays::<Test>::insert(1u64, 1u64, 2);
            Hosting::clear_era(0);
            assert!(Hosting::era_hosts(0).is_empty());
            assert_eq!(Hosting::era_hosts(1), vec![(1u64, 2u64)]);
        });
    }
}
