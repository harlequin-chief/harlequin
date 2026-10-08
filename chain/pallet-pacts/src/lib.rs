//! Harlequin **pacts** — private contracts signed on chain (design/PACTOS-CONTRATOS-PRIVADOS-2026-10-01.md).
//!
//! The chain never sees the document. It holds a **salted commitment**
//! `c = blake2_256("hlq-pact-v1" ‖ salt ‖ sha256(document))`, built by the parties' wallets: the 32-byte salt
//! stays with the parties, so nobody can confirm a guessed text (a standard contract template) against `c`.
//!
//! Two modes:
//! - **A — between parties.** `propose(c, parties, ttl)` by one party; every other party `sign`s with its own
//!   mask (the signed extrinsic IS the signature). When all have signed the pact is **completed**, for good.
//!   Unfinished pacts expire and anyone may `reap` them, so open proposals are never eternal state.
//! - **B — sealed.** `seal(k)` records only that `k` existed at a block, where
//!   `k = blake2_256("hlq-pact-sealed-v1" ‖ c ‖ sha256(off-chain signatures))`. Who signed with whom is known
//!   only to whoever holds the pact package.
//!
//! **No money moves here** (no escrow: a newborn mask holds no coins). A broken pact goes to the villa's
//! justice: the plaintiff reveals text + salt to the jury, off chain; the jury checks `c`.
//!
//! **Anti-abuse** (lesson of #57/#61: free accounts × free writes = eternal rows): at most `MaxParties`
//! parties, at most `MaxOpenPerMask` unfinished proposals per proposer, a completed pact needs EVERY party's
//! own signature (a spammer cannot complete pacts "with" victims), unfinished ones expire, and the runtime
//! keeps these calls on the rationed feeless lane. Weights count real reads/writes and proof size.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// A pact commitment (mode A) or sealed key (mode B): 32 bytes, computed by the wallet.
pub type Commitment = [u8; 32];

#[frame::pallet]
pub mod pallet {
    use super::Commitment;
    use frame::prelude::*;

    #[derive(
        codec::Encode,
        codec::Decode,
        codec::DecodeWithMemTracking,
        codec::MaxEncodedLen,
        scale_info::TypeInfo,
        Clone,
        PartialEq,
        Eq,
        Debug,
    )]
    #[scale_info(skip_type_params(T))]
    pub struct Pact<T: Config> {
        /// Salted commitment to the document. The text never touches the chain.
        pub commitment: Commitment,
        /// Who proposed it (always one of `parties`).
        pub proposer: T::AccountId,
        /// Every party that must sign, proposer included. No duplicates, at least two.
        pub parties: BoundedVec<T::AccountId, T::MaxParties>,
        /// Parties that have signed so far (the proposer signs by proposing).
        pub signed: BoundedVec<T::AccountId, T::MaxParties>,
        /// Block it was proposed at.
        pub at: BlockNumberFor<T>,
        /// Last block at which it can still be signed; after it, if unfinished, anyone may reap it.
        pub expires: BlockNumberFor<T>,
        /// Every party signed. Immutable from here on.
        pub completed: bool,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Max parties in one pact (proposer included).
        #[pallet::constant]
        type MaxParties: Get<u32>;

        /// Max unfinished pacts a single mask may have proposed at once.
        #[pallet::constant]
        type MaxOpenPerMask: Get<u32>;

        /// Longest signing window a proposal may ask for, in blocks.
        #[pallet::constant]
        type MaxPactTtl: Get<BlockNumberFor<Self>>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Next pact id. Monotonic; ids of reaped/withdrawn proposals are never reused.
    #[pallet::storage]
    pub type NextPactId<T> = StorageValue<_, u64, ValueQuery>;

    /// `id → pact` (mode A). Completed pacts stay forever; unfinished ones leave on withdraw/reap.
    #[pallet::storage]
    pub type Pacts<T: Config> = StorageMap<_, Blake2_128Concat, u64, Pact<T>, OptionQuery>;

    /// `proposer → number of its unfinished pacts`. Bounded by `MaxOpenPerMask`.
    #[pallet::storage]
    pub type OpenOf<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// `sealed key → block first seen` (mode B). Hashed key: the caller chooses `key`, and an identity
    /// hasher on caller-chosen keys lets anyone shape the storage trie.
    #[pallet::storage]
    pub type Sealed<T: Config> = StorageMap<_, Blake2_128Concat, Commitment, BlockNumberFor<T>, OptionQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        Proposed { id: u64, proposer: T::AccountId, commitment: Commitment },
        Signed { id: u64, who: T::AccountId },
        /// Every party signed: the pact is closed and permanent.
        Completed { id: u64, commitment: Commitment },
        Withdrawn { id: u64 },
        Reaped { id: u64 },
        Sealed { key: Commitment },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// A pact needs at least two distinct parties.
        TooFewParties,
        /// The same mask appears twice among the parties.
        DuplicateParty,
        /// The caller must be one of the parties.
        NotAParty,
        /// Signing window must be > 0 and ≤ `MaxPactTtl`.
        BadTtl,
        /// The proposer already has `MaxOpenPerMask` unfinished pacts.
        TooManyOpenPacts,
        PactNotFound,
        AlreadySigned,
        AlreadyCompleted,
        /// The signing window closed.
        Expired,
        /// Still inside its signing window: cannot be reaped yet.
        NotExpired,
        /// Only the proposer withdraws.
        NotProposer,
        AlreadySealed,
    }

    impl<T: Config> Pallet<T> {
        fn pact_proof() -> u64 {
            // Max encoded pact + map key overhead: what a light client must read to check one.
            (Pact::<T>::max_encoded_len() as u64).saturating_add(48)
        }
        fn w(reads: u64, writes: u64, proof: u64) -> Weight {
            T::DbWeight::get().reads_writes(reads, writes).saturating_add(Weight::from_parts(20_000_000, proof))
        }
        fn close_open_slot(proposer: &T::AccountId) {
            OpenOf::<T>::mutate_exists(proposer, |n| {
                let left = n.unwrap_or(0).saturating_sub(1);
                *n = if left == 0 { None } else { Some(left) };
            });
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Mode A: propose a pact over `commitment` among `parties` (caller included), signable for `ttl` blocks.
        #[pallet::call_index(0)]
        #[pallet::weight(Pallet::<T>::w(2, 3, Pallet::<T>::pact_proof().saturating_add(64)))]
        pub fn propose(
            origin: OriginFor<T>,
            commitment: Commitment,
            parties: BoundedVec<T::AccountId, T::MaxParties>,
            ttl: BlockNumberFor<T>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(parties.len() >= 2, Error::<T>::TooFewParties);
            for (i, p) in parties.iter().enumerate() {
                ensure!(!parties.iter().skip(i + 1).any(|q| q == p), Error::<T>::DuplicateParty);
            }
            ensure!(parties.contains(&who), Error::<T>::NotAParty);
            ensure!(!ttl.is_zero() && ttl <= T::MaxPactTtl::get(), Error::<T>::BadTtl);
            OpenOf::<T>::try_mutate(&who, |n| -> DispatchResult {
                ensure!(*n < T::MaxOpenPerMask::get(), Error::<T>::TooManyOpenPacts);
                *n += 1;
                Ok(())
            })?;
            let now = frame_system::Pallet::<T>::block_number();
            let mut signed = BoundedVec::<T::AccountId, T::MaxParties>::new();
            signed.try_push(who.clone()).map_err(|_| Error::<T>::TooFewParties)?; // cannot fail: MaxParties ≥ 2
            let id = NextPactId::<T>::get();
            Pacts::<T>::insert(
                id,
                Pact::<T> {
                    commitment,
                    proposer: who.clone(),
                    parties,
                    signed,
                    at: now,
                    expires: now.saturating_add(ttl),
                    completed: false,
                },
            );
            NextPactId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::Proposed { id, proposer: who, commitment });
            Ok(())
        }

        /// Mode A: a party signs. The last signature completes the pact.
        #[pallet::call_index(1)]
        #[pallet::weight(Pallet::<T>::w(2, 2, Pallet::<T>::pact_proof().saturating_add(64)))]
        pub fn sign(origin: OriginFor<T>, id: u64) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let now = frame_system::Pallet::<T>::block_number();
            let done = Pacts::<T>::try_mutate(id, |maybe| -> Result<Option<(T::AccountId, Commitment)>, DispatchError> {
                let p = maybe.as_mut().ok_or(Error::<T>::PactNotFound)?;
                ensure!(!p.completed, Error::<T>::AlreadyCompleted);
                ensure!(now <= p.expires, Error::<T>::Expired);
                ensure!(p.parties.contains(&who), Error::<T>::NotAParty);
                ensure!(!p.signed.contains(&who), Error::<T>::AlreadySigned);
                p.signed.try_push(who.clone()).map_err(|_| Error::<T>::AlreadySigned)?;
                if p.signed.len() == p.parties.len() {
                    p.completed = true;
                    return Ok(Some((p.proposer.clone(), p.commitment)));
                }
                Ok(None)
            })?;
            Self::deposit_event(Event::Signed { id, who });
            if let Some((proposer, commitment)) = done {
                Self::close_open_slot(&proposer);
                Self::deposit_event(Event::Completed { id, commitment });
            }
            Ok(())
        }

        /// Mode A: the proposer takes back an unfinished pact.
        #[pallet::call_index(2)]
        #[pallet::weight(Pallet::<T>::w(2, 2, Pallet::<T>::pact_proof().saturating_add(64)))]
        pub fn withdraw(origin: OriginFor<T>, id: u64) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let p = Pacts::<T>::get(id).ok_or(Error::<T>::PactNotFound)?;
            ensure!(p.proposer == who, Error::<T>::NotProposer);
            ensure!(!p.completed, Error::<T>::AlreadyCompleted);
            Pacts::<T>::remove(id);
            Self::close_open_slot(&who);
            Self::deposit_event(Event::Withdrawn { id });
            Ok(())
        }

        /// Mode A: anyone clears an unfinished pact whose signing window has closed.
        #[pallet::call_index(3)]
        #[pallet::weight(Pallet::<T>::w(2, 2, Pallet::<T>::pact_proof().saturating_add(64)))]
        pub fn reap(origin: OriginFor<T>, id: u64) -> DispatchResult {
            ensure_signed(origin)?;
            let p = Pacts::<T>::get(id).ok_or(Error::<T>::PactNotFound)?;
            ensure!(!p.completed, Error::<T>::AlreadyCompleted);
            ensure!(frame_system::Pallet::<T>::block_number() > p.expires, Error::<T>::NotExpired);
            Pacts::<T>::remove(id);
            Self::close_open_slot(&p.proposer);
            Self::deposit_event(Event::Reaped { id });
            Ok(())
        }

        /// Mode B: record that `key` existed at this block. Who signed is not on chain.
        #[pallet::call_index(4)]
        #[pallet::weight(Pallet::<T>::w(1, 1, 16 + 32 + 4 + 48))]
        pub fn seal(origin: OriginFor<T>, key: Commitment) -> DispatchResult {
            ensure_signed(origin)?;
            ensure!(!Sealed::<T>::contains_key(key), Error::<T>::AlreadySealed);
            Sealed::<T>::insert(key, frame_system::Pallet::<T>::block_number());
            Self::deposit_event(Event::Sealed { key });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn get(id: u64) -> Option<Pact<T>> {
            Pacts::<T>::get(id)
        }
        pub fn sealed_at(key: Commitment) -> Option<BlockNumberFor<T>> {
            Sealed::<T>::get(key)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_pacts;
    use crate::{Error, OpenOf};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Pacts: pallet_pacts,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    impl pallet_pacts::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type MaxParties = ConstU32<4>;
        type MaxOpenPerMask = ConstU32<2>;
        type MaxPactTtl = ConstU64<100>;
    }

    fn ext() -> TestState {
        let mut t: TestState = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into();
        t.execute_with(|| System::set_block_number(1));
        t
    }
    fn c(n: u8) -> [u8; 32] {
        [n; 32]
    }
    fn parties(v: &[u64]) -> BoundedVec<u64, ConstU32<4>> {
        v.to_vec().try_into().unwrap()
    }

    #[test]
    fn pact_completes_only_when_every_party_signs() {
        ext().execute_with(|| {
            assert_ok!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1, 2, 3]), 10));
            assert!(!Pacts::get(0).unwrap().completed);
            assert_noop!(Pacts::sign(RuntimeOrigin::signed(1), 0), Error::<Test>::AlreadySigned);
            assert_noop!(Pacts::sign(RuntimeOrigin::signed(9), 0), Error::<Test>::NotAParty);
            assert_ok!(Pacts::sign(RuntimeOrigin::signed(2), 0));
            assert!(!Pacts::get(0).unwrap().completed, "two of three is not a pact");
            assert_ok!(Pacts::sign(RuntimeOrigin::signed(3), 0));
            assert!(Pacts::get(0).unwrap().completed);
            assert_eq!(OpenOf::<Test>::get(1), 0, "a completed pact frees the proposer's open slot");
            // permanent: no more signatures, no withdraw, no reap
            assert_noop!(Pacts::sign(RuntimeOrigin::signed(2), 0), Error::<Test>::AlreadyCompleted);
            assert_noop!(Pacts::withdraw(RuntimeOrigin::signed(1), 0), Error::<Test>::AlreadyCompleted);
            System::set_block_number(500);
            assert_noop!(Pacts::reap(RuntimeOrigin::signed(9), 0), Error::<Test>::AlreadyCompleted);
            assert!(Pacts::get(0).is_some());
        });
    }

    #[test]
    fn proposal_rules() {
        ext().execute_with(|| {
            assert_noop!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1]), 10), Error::<Test>::TooFewParties);
            assert_noop!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1, 2, 2]), 10), Error::<Test>::DuplicateParty);
            // nobody proposes a pact in someone else's name
            assert_noop!(Pacts::propose(RuntimeOrigin::signed(9), c(1), parties(&[1, 2]), 10), Error::<Test>::NotAParty);
            assert_noop!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1, 2]), 0), Error::<Test>::BadTtl);
            assert_noop!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1, 2]), 101), Error::<Test>::BadTtl);
            // open cap per proposer, and a rejected proposal burns no id
            assert_ok!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1, 2]), 10));
            assert_ok!(Pacts::propose(RuntimeOrigin::signed(1), c(2), parties(&[1, 3]), 10));
            assert_noop!(Pacts::propose(RuntimeOrigin::signed(1), c(3), parties(&[1, 4]), 10), Error::<Test>::TooManyOpenPacts);
            assert_eq!(crate::NextPactId::<Test>::get(), 2);
            // a privileged origin is as powerless as anyone
            assert!(Pacts::propose(RuntimeOrigin::root(), c(5), parties(&[1, 2]), 10).is_err());
            assert!(Pacts::sign(RuntimeOrigin::root(), 0).is_err());
        });
    }

    #[test]
    fn unfinished_pacts_expire_and_anyone_reaps_them() {
        ext().execute_with(|| {
            assert_ok!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1, 2]), 10));
            assert_noop!(Pacts::reap(RuntimeOrigin::signed(7), 0), Error::<Test>::NotExpired);
            System::set_block_number(11); // the last block of the window (proposed at 1, ttl 10) still signs
            assert_ok!(Pacts::sign(RuntimeOrigin::signed(2), 0));
            assert!(Pacts::get(0).unwrap().completed);
        });
        ext().execute_with(|| {
            assert_ok!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1, 2]), 10));
            System::set_block_number(12);
            assert_noop!(Pacts::sign(RuntimeOrigin::signed(2), 0), Error::<Test>::Expired);
            assert_ok!(Pacts::reap(RuntimeOrigin::signed(7), 0));
            assert!(crate::Pacts::<Test>::get(0).is_none());
            assert_eq!(OpenOf::<Test>::get(1), 0, "reaping frees the slot");
        });
    }

    #[test]
    fn only_the_proposer_withdraws_an_unfinished_pact() {
        ext().execute_with(|| {
            assert_ok!(Pacts::propose(RuntimeOrigin::signed(1), c(1), parties(&[1, 2]), 10));
            assert_noop!(Pacts::withdraw(RuntimeOrigin::signed(2), 0), Error::<Test>::NotProposer);
            assert_ok!(Pacts::withdraw(RuntimeOrigin::signed(1), 0));
            assert!(Pacts::get(0).is_none());
            assert_eq!(OpenOf::<Test>::get(1), 0);
            assert_noop!(Pacts::sign(RuntimeOrigin::signed(2), 0), Error::<Test>::PactNotFound);
        });
    }

    #[test]
    fn sealing_records_once() {
        ext().execute_with(|| {
            assert_ok!(Pacts::seal(RuntimeOrigin::signed(5), c(9)));
            assert_eq!(Pacts::sealed_at(c(9)), Some(1));
            System::set_block_number(7);
            assert_noop!(Pacts::seal(RuntimeOrigin::signed(6), c(9)), Error::<Test>::AlreadySealed);
            assert_eq!(Pacts::sealed_at(c(9)), Some(1), "first sight is the proof of existence");
            assert!(Pacts::seal(RuntimeOrigin::none(), c(8)).is_err());
        });
    }
}
