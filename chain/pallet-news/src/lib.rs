//! Harlequin news board — #665. An **append-only, ordered** index of the society's news/announcements.
//!
//! Same brick as the forum/market, but **flat** (no threads — it is announcements, not discussion). Each
//! item carries a short **headline on-chain** (so the board lists without fetching IPFS) and a content
//! hash of the full **body**, which lives off-chain in IPFS (content-addressed, verify-by-recompute).
//!
//! "Inside the society": posting requires a signed mask; the member client surfaces the board in the
//! plaza. The on-chain index is, by nature, public (it is a chain); the *surfacing* is the client's.
//! **Default: any member publishes.** Ordering/visibility weighted by the author's **reputation** is a
//! client computation (off `pallet-reputation`) — NOT stored or done here (news ≠ power; the pallet keeps
//! the orthogonality of market/tokens).
//!
//! **Withdraw vs hide.** The author may `withdraw` their own item (flag, append-only); a jury may `hide`
//! an abusive one via `ModerationOrigin` (never a signed admin). Both keep the entry and free the
//! author's *active* cap slot. **Anti-spam:** every `publish` passes a pluggable [`PublishGate`] (default
//! no-op; production wires the B6 micro-fee).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// Content hash of a news body (its IPFS address as a 32-byte multihash digest, e.g. CIDv1 raw+sha256).
pub type BodyHash = [u8; 32];

#[frame::pallet]
pub mod pallet {
    use super::BodyHash;
    use frame::prelude::*;

    /// Pre-publish gate (anti-spam). Production wires it to the B6 micro-fee or a reputation /
    /// registered-mask check; the default `()` impl is a no-op.
    pub trait PublishGate<AccountId> {
        fn ensure_can_publish(who: &AccountId) -> DispatchResult;
    }
    impl<AccountId> PublishGate<AccountId> for () {
        fn ensure_can_publish(_who: &AccountId) -> DispatchResult {
            Ok(())
        }
    }

    /// One news index entry. The body is off-chain (IPFS at `body`); the headline is on-chain.
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
    pub struct NewsItem<T: Config> {
        /// Who posted it (the pseudonymous account / mask). Reputation is read in the client, not here.
        pub author: T::AccountId,
        /// Short headline, ON-chain so the board lists without fetching IPFS. Bounded by `MaxHeadlineLen`.
        pub headline: BoundedVec<u8, T::MaxHeadlineLen>,
        /// Content hash of the full body (IPFS address). The chain never holds the text.
        pub body: BodyHash,
        /// Block at which it was indexed.
        pub at: BlockNumberFor<T>,
        /// The author retracted it (flag; entry kept — append-only). Frees the active cap slot.
        pub withdrawn: bool,
        /// Hidden by a jury verdict (flag; entry kept — no memory-holing). Frees the active cap slot.
        pub hidden: bool,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Origin allowed to hide an item — the **jury verdict** path (`pallet-justice`). NOT a signed admin.
        type ModerationOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Pre-publish gate (anti-spam). Default `()` = no-op; production wires the B6 micro-fee here.
        type Postage: PublishGate<Self::AccountId>;

        /// Max length of the on-chain headline (keep small to bound on-chain state, e.g. ≤128 bytes).
        #[pallet::constant]
        type MaxHeadlineLen: Get<u32>;

        /// Max number of *active* (not withdrawn, not hidden) items a single mask may hold at once.
        #[pallet::constant]
        type MaxPerMask: Get<u32>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Next item id. Monotonic — the id IS the canonical global order (append-only).
    #[pallet::storage]
    pub type NextId<T> = StorageValue<_, u64, ValueQuery>;

    /// `id → item`. The ordered index. Append-only: entries are never removed (withdraw/hide set flags).
    #[pallet::storage]
    pub type Items<T: Config> = StorageMap<_, Blake2_128Concat, u64, NewsItem<T>, OptionQuery>;

    /// `mask → its ACTIVE item ids`. Bounded by `MaxPerMask`. publish adds; withdraw/hide remove.
    #[pallet::storage]
    pub type ItemsOf<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BoundedVec<u64, T::MaxPerMask>, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A news item was indexed: `id` (its order), by `author`, body at `body`.
        Published { id: u64, author: T::AccountId, body: BodyHash },
        /// An item was withdrawn by its author (entry kept; active slot freed).
        Withdrawn { id: u64 },
        /// An item was hidden by a jury verdict (entry kept; active slot freed).
        Hidden { id: u64 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// No item with that id.
        NewsNotFound,
        /// The caller is not the item's author.
        NotYourNews,
        /// The item is already withdrawn.
        AlreadyWithdrawn,
        /// The item is already hidden.
        AlreadyHidden,
        /// The mask already holds `MaxPerMask` active items.
        TooManyActiveNews,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Publish a news item: a short on-chain headline + the body content-hash (IPFS) under the caller's
        /// mask at the next order id. Append-only. Passes the `Postage` gate + the per-mask active cap.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(25_000, 0))]
        pub fn publish(
            origin: OriginFor<T>,
            headline: BoundedVec<u8, T::MaxHeadlineLen>,
            body: BodyHash,
        ) -> DispatchResult {
            let author = ensure_signed(origin)?;
            T::Postage::ensure_can_publish(&author)?;

            let id = NextId::<T>::get();
            // Reserve the active slot first; fail (no state change) if the mask is at the cap.
            ItemsOf::<T>::try_mutate(&author, |ids| ids.try_push(id))
                .map_err(|_| Error::<T>::TooManyActiveNews)?;

            Items::<T>::insert(
                id,
                NewsItem::<T> {
                    author: author.clone(),
                    headline,
                    body,
                    at: frame_system::Pallet::<T>::block_number(),
                    withdrawn: false,
                    hidden: false,
                },
            );
            NextId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::Published { id, author, body });
            Ok(())
        }

        /// Withdraw your own item. Signed; only the author. Flag (entry kept) + frees the active cap slot.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn withdraw(origin: OriginFor<T>, id: u64) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Items::<T>::try_mutate(id, |maybe| -> DispatchResult {
                let n = maybe.as_mut().ok_or(Error::<T>::NewsNotFound)?;
                ensure!(n.author == who, Error::<T>::NotYourNews);
                ensure!(!n.withdrawn, Error::<T>::AlreadyWithdrawn);
                n.withdrawn = true;
                Ok(())
            })?;
            ItemsOf::<T>::mutate(&who, |ids| ids.retain(|x| *x != id));
            Self::deposit_event(Event::Withdrawn { id });
            Ok(())
        }

        /// Hide an item. Gated by `ModerationOrigin` — a jury verdict, never a signed admin. Flag (entry
        /// kept — no memory-holing) + frees the author's active cap slot.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn hide(origin: OriginFor<T>, id: u64) -> DispatchResult {
            T::ModerationOrigin::ensure_origin(origin)?;
            let author = Items::<T>::try_mutate(id, |maybe| -> Result<T::AccountId, DispatchError> {
                let n = maybe.as_mut().ok_or(Error::<T>::NewsNotFound)?;
                ensure!(!n.hidden, Error::<T>::AlreadyHidden);
                n.hidden = true;
                Ok(n.author.clone())
            })?;
            ItemsOf::<T>::mutate(&author, |ids| ids.retain(|x| *x != id));
            Self::deposit_event(Event::Hidden { id });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Fetch an item by id (None if no such id).
        pub fn get(id: u64) -> Option<NewsItem<T>> {
            Items::<T>::get(id)
        }

        /// Total items ever indexed (= next id), the canonical order bound.
        pub fn count() -> u64 {
            NextId::<T>::get()
        }

        /// A mask's current ACTIVE item ids.
        pub fn active_of(who: &T::AccountId) -> alloc::vec::Vec<u64> {
            ItemsOf::<T>::get(who).into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_news;
    use crate::{Error, Items};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            News: pallet_news,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    impl pallet_news::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type ModerationOrigin = EnsureRoot<Self::AccountId>;
        type Postage = ();
        type MaxHeadlineLen = ConstU32<32>;
        type MaxPerMask = ConstU32<2>;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    fn b(n: u8) -> [u8; 32] {
        [n; 32]
    }
    fn hl(s: &[u8]) -> BoundedVec<u8, ConstU32<32>> {
        s.to_vec().try_into().unwrap()
    }

    #[test]
    fn items_are_ordered_append_only() {
        new_test_ext().execute_with(|| {
            assert_ok!(News::publish(RuntimeOrigin::signed(1), hl(b"Genesis at last"), b(1)));
            assert_ok!(News::publish(RuntimeOrigin::signed(2), hl(b"Market opens"), b(2)));
            assert_eq!(News::get(0).unwrap().author, 1);
            assert_eq!(News::get(1).unwrap().body, b(2));
            assert_eq!(News::get(0).unwrap().headline.to_vec(), b"Genesis at last".to_vec());
            assert_eq!(News::count(), 2);
        });
    }

    #[test]
    fn withdraw_only_by_author_flags_and_frees_slot() {
        new_test_ext().execute_with(|| {
            assert_ok!(News::publish(RuntimeOrigin::signed(1), hl(b"x"), b(1)));
            assert_noop!(News::withdraw(RuntimeOrigin::signed(2), 0), Error::<Test>::NotYourNews);
            assert_ok!(News::withdraw(RuntimeOrigin::signed(1), 0));
            let n = Items::<Test>::get(0).unwrap();
            assert!(n.withdrawn);
            assert_eq!(n.author, 1); // entry not erased
            assert!(News::active_of(&1).is_empty()); // slot freed
            assert_noop!(News::withdraw(RuntimeOrigin::signed(1), 0), Error::<Test>::AlreadyWithdrawn);
        });
    }

    #[test]
    fn active_cap_counts_only_live_items() {
        new_test_ext().execute_with(|| {
            assert_ok!(News::publish(RuntimeOrigin::signed(1), hl(b"a"), b(1)));
            assert_ok!(News::publish(RuntimeOrigin::signed(1), hl(b"b"), b(2)));
            assert_noop!(
                News::publish(RuntimeOrigin::signed(1), hl(b"c"), b(3)),
                Error::<Test>::TooManyActiveNews
            );
            assert_eq!(News::count(), 2); // rejected publish burned no id (atomic)
            assert_ok!(News::withdraw(RuntimeOrigin::signed(1), 0));
            assert_ok!(News::publish(RuntimeOrigin::signed(1), hl(b"c"), b(3)));
            assert_eq!(News::count(), 3);
        });
    }

    #[test]
    fn hide_only_by_moderation_origin_keeps_entry_frees_slot() {
        new_test_ext().execute_with(|| {
            assert_ok!(News::publish(RuntimeOrigin::signed(1), hl(b"x"), b(1)));
            assert_noop!(News::hide(RuntimeOrigin::signed(1), 0), DispatchError::BadOrigin);
            assert_ok!(News::hide(RuntimeOrigin::root(), 0));
            let n = Items::<Test>::get(0).unwrap();
            assert!(n.hidden);
            assert_eq!(n.author, 1);
            assert!(News::active_of(&1).is_empty());
            assert_noop!(News::hide(RuntimeOrigin::root(), 0), Error::<Test>::AlreadyHidden);
        });
    }

    #[test]
    fn withdraw_missing_item_rejected() {
        new_test_ext().execute_with(|| {
            assert_noop!(News::withdraw(RuntimeOrigin::signed(1), 0), Error::<Test>::NewsNotFound);
        });
    }

    #[test]
    fn hide_missing_item_rejected() {
        new_test_ext().execute_with(|| {
            assert_noop!(News::hide(RuntimeOrigin::root(), 0), Error::<Test>::NewsNotFound);
        });
    }

    #[test]
    fn headline_bound_enforced() {
        new_test_ext().execute_with(|| {
            assert_ok!(News::publish(RuntimeOrigin::signed(1), hl(b"thirty_two_chars_headline_okkkk!"), b(1)));
            let too_long: Result<BoundedVec<u8, ConstU32<32>>, _> =
                b"this_headline_is_thirty_three_ch!".to_vec().try_into();
            assert!(too_long.is_err());
        });
    }
}
