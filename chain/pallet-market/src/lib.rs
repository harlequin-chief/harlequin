//! Harlequin market index — level A: the **offers board**. An **append-only, ordered** index of
//! content-addressed offers on chain.
//!
//! Like the forum, the offer **detail** (title, description, terms, how to contact) lives off-chain in
//! **IPFS** (content-addressed); the chain holds only a compact index entry — the offerer's mask, the
//! detail's content hash (its IPFS address), a monotonic order id, an optional category tag, and two
//! flags. The buyer reads the offer, fetches the detail from IPFS (verifying it against the on-chain
//! hash), checks the **seller's reputation in the client** (off `pallet-reputation` — this pallet stores
//! no reputation: markets are not power, like `pallet-tokens`), and contacts the seller over the chat.
//!
//! **No payment, no escrow** here (those are levels B/C). The trade is closed by the two masks
//! themselves. Settlement in HLQ + jury-backed disputes come later if and when the society wants them.
//!
//! **Withdraw vs hide.** The offerer may `withdraw` their own offer (a flag — the entry is never erased,
//! append-only, no memory-holing); a jury may `hide` an abusive/illegal one via `ModerationOrigin`
//! (never a signed admin — moderation is by reputation, not authority). Either way the entry remains;
//! both free the offerer's *active-offer* cap slot (the cap counts only live listings, not history).
//!
//! **Anti-spam.** A free, unbounded index is DoS-able, so every `publish` passes through a pluggable
//! [`PublishGate`] in `Config`: the default is a no-op, and production wires it to the B6 micro-fee
//! (`pallet-tokens`) / a registered-mask or reputation check — the *hook* is here so it activates
//! without a rewrite; the *policy* is the runtime's, not hard-coded.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// Content hash of an offer detail (its IPFS address as a 32-byte multihash digest, e.g. CIDv1 raw+sha256).
/// The chain stores only this; the detail is fetched from IPFS by the client and verified against it.
pub type DetailHash = [u8; 32];

#[frame::pallet]
pub mod pallet {
    use super::DetailHash;
    use frame::prelude::*;

    /// Pre-publish gate (anti-spam / anti-DoS). Production wires it to the B6 micro-fee or a reputation /
    /// registered-mask check; the default `()` impl is a no-op. The hook lives here so the policy can be
    /// switched on in the runtime without changing this pallet.
    pub trait PublishGate<AccountId> {
        fn ensure_can_publish(who: &AccountId) -> DispatchResult;
    }
    impl<AccountId> PublishGate<AccountId> for () {
        fn ensure_can_publish(_who: &AccountId) -> DispatchResult {
            Ok(())
        }
    }

    /// One market index entry. The detail is off-chain (IPFS at `detail`); this is the consensus-ordered
    /// metadata.
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
    pub struct Offer<T: Config> {
        /// Who offers it (the pseudonymous account / mask). Handle = `sha256(pubkey)[..16]`, derived for
        /// display; reputation is read in the client, never stored here.
        pub offerer: T::AccountId,
        /// Content hash of the detail (IPFS address). The chain never holds the text.
        pub detail: DetailHash,
        /// Optional free category tag (e.g. "tools", "lessons"), bounded by `MaxCategoryLen`.
        pub category: BoundedVec<u8, T::MaxCategoryLen>,
        /// Block at which it was indexed (ordering tiebreak / display).
        pub at: BlockNumberFor<T>,
        /// The offerer withdrew it (flag; entry kept — append-only). Frees the active-offer cap slot.
        pub withdrawn: bool,
        /// Hidden by a jury verdict (flag; entry kept — no memory-holing). Frees the active-offer cap slot.
        pub hidden: bool,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Origin allowed to hide an offer — the **jury verdict** path (`pallet-justice`), pluggable like
        /// the other pallets' privileged origins. NOT a signed admin: moderation is by reputation.
        type ModerationOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Pre-publish gate (anti-spam). Default `()` = no-op; production wires the B6 micro-fee here.
        type Postage: PublishGate<Self::AccountId>;

        /// Max length of the optional category tag.
        #[pallet::constant]
        type MaxCategoryLen: Get<u32>;

        /// Max number of *active* (not withdrawn, not hidden) offers a single mask may hold at once.
        #[pallet::constant]
        type MaxPerMask: Get<u32>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// The next offer id to assign. Monotonic — the id IS the canonical global order (append-only).
    #[pallet::storage]
    pub type NextId<T> = StorageValue<_, u64, ValueQuery>;

    /// `id → offer`. The ordered index. Ids are dense and increasing, so iterating ids is chronological.
    /// Append-only: entries are never removed (withdraw/hide set flags), so the record is non-repudiable.
    #[pallet::storage]
    pub type Offers<T: Config> = StorageMap<_, Blake2_128Concat, u64, Offer<T>, OptionQuery>;

    /// `mask → its ACTIVE offer ids`. Bounded by `MaxPerMask`. `publish` adds; `withdraw`/`hide` remove —
    /// so the cap counts only live listings (history stays in `Offers`), never blocking a mask forever.
    #[pallet::storage]
    pub type OffersOf<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BoundedVec<u64, T::MaxPerMask>, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// An offer was indexed: `id` (its order), by `offerer`, detail at `detail`.
        Published { id: u64, offerer: T::AccountId, detail: DetailHash },
        /// An offer was withdrawn by its offerer (the entry remains; the active-offer slot is freed).
        Withdrawn { id: u64 },
        /// An offer was hidden by a jury verdict (the entry remains; the active-offer slot is freed).
        Hidden { id: u64 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// No offer with that id.
        OfferNotFound,
        /// The caller is not the offer's offerer.
        NotYourOffer,
        /// The offer is already withdrawn.
        AlreadyWithdrawn,
        /// The offer is already hidden.
        AlreadyHidden,
        /// The mask already holds `MaxPerMask` active offers.
        TooManyActiveOffers,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Publish an offer: index its detail content-hash (IPFS address) under the caller's mask at the
        /// next order id. Append-only. Passes the `Postage` gate (anti-spam) and the per-mask active cap.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(25_000, 0))]
        pub fn publish(
            origin: OriginFor<T>,
            detail: DetailHash,
            category: BoundedVec<u8, T::MaxCategoryLen>,
        ) -> DispatchResult {
            let offerer = ensure_signed(origin)?;
            T::Postage::ensure_can_publish(&offerer)?;

            let id = NextId::<T>::get();
            // Reserve the active-offer slot first; fail (no state change) if the mask is at the cap.
            OffersOf::<T>::try_mutate(&offerer, |ids| ids.try_push(id))
                .map_err(|_| Error::<T>::TooManyActiveOffers)?;

            Offers::<T>::insert(
                id,
                Offer::<T> {
                    offerer: offerer.clone(),
                    detail,
                    category,
                    at: frame_system::Pallet::<T>::block_number(),
                    withdrawn: false,
                    hidden: false,
                },
            );
            NextId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::Published { id, offerer, detail });
            Ok(())
        }

        /// Withdraw your own offer. Signed; only the offerer. Sets the `withdrawn` flag (the entry is kept
        /// — append-only) and frees the offerer's active-offer cap slot.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn withdraw(origin: OriginFor<T>, id: u64) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Offers::<T>::try_mutate(id, |maybe| -> DispatchResult {
                let o = maybe.as_mut().ok_or(Error::<T>::OfferNotFound)?;
                ensure!(o.offerer == who, Error::<T>::NotYourOffer);
                ensure!(!o.withdrawn, Error::<T>::AlreadyWithdrawn);
                o.withdrawn = true;
                Ok(())
            })?;
            OffersOf::<T>::mutate(&who, |ids| ids.retain(|x| *x != id));
            Self::deposit_event(Event::Withdrawn { id });
            Ok(())
        }

        /// Hide an offer. Gated by `ModerationOrigin` — a jury verdict, never a signed admin. Sets the
        /// `hidden` flag (entry kept — no memory-holing) and frees the offerer's active-offer cap slot.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn hide(origin: OriginFor<T>, id: u64) -> DispatchResult {
            T::ModerationOrigin::ensure_origin(origin)?;
            let offerer =
                Offers::<T>::try_mutate(id, |maybe| -> Result<T::AccountId, DispatchError> {
                    let o = maybe.as_mut().ok_or(Error::<T>::OfferNotFound)?;
                    ensure!(!o.hidden, Error::<T>::AlreadyHidden);
                    o.hidden = true;
                    Ok(o.offerer.clone())
                })?;
            OffersOf::<T>::mutate(&offerer, |ids| ids.retain(|x| *x != id));
            Self::deposit_event(Event::Hidden { id });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Fetch an offer by id (None if no such id).
        pub fn get(id: u64) -> Option<Offer<T>> {
            Offers::<T>::get(id)
        }

        /// Total offers ever indexed (= next id), the canonical order bound.
        pub fn count() -> u64 {
            NextId::<T>::get()
        }

        /// A mask's current ACTIVE offer ids.
        pub fn active_of(who: &T::AccountId) -> alloc::vec::Vec<u64> {
            OffersOf::<T>::get(who).into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_market;
    use crate::{Error, Offers};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Market: pallet_market,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    impl pallet_market::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        // Mock: the jury-verdict origin stands in as root; the runtime wires it to pallet-justice.
        type ModerationOrigin = EnsureRoot<Self::AccountId>;
        // Mock: no anti-spam gate (no-op); production wires the B6 micro-fee here.
        type Postage = ();
        type MaxCategoryLen = ConstU32<16>;
        type MaxPerMask = ConstU32<2>;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    fn h(n: u8) -> [u8; 32] {
        [n; 32]
    }
    fn cat(s: &[u8]) -> BoundedVec<u8, ConstU32<16>> {
        s.to_vec().try_into().unwrap()
    }

    #[test]
    fn offers_are_ordered_append_only() {
        new_test_ext().execute_with(|| {
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"tools")));
            assert_ok!(Market::publish(RuntimeOrigin::signed(2), h(2), cat(b"lessons")));
            assert_eq!(Market::get(0).unwrap().offerer, 1);
            assert_eq!(Market::get(1).unwrap().detail, h(2));
            assert_eq!(Market::get(0).unwrap().category.to_vec(), b"tools".to_vec());
            assert_eq!(Market::count(), 2);
        });
    }

    #[test]
    fn withdraw_only_by_offerer_flags_and_frees_slot() {
        new_test_ext().execute_with(|| {
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"")));
            assert_noop!(Market::withdraw(RuntimeOrigin::signed(2), 0), Error::<Test>::NotYourOffer);
            assert_ok!(Market::withdraw(RuntimeOrigin::signed(1), 0));
            let o = Offers::<Test>::get(0).unwrap();
            assert!(o.withdrawn); // flagged...
            assert_eq!(o.offerer, 1); // ...entry NOT erased (no memory-holing)
            assert!(Market::active_of(&1).is_empty()); // ...but the active-offer slot is freed
            assert_noop!(Market::withdraw(RuntimeOrigin::signed(1), 0), Error::<Test>::AlreadyWithdrawn);
        });
    }

    #[test]
    fn active_cap_counts_only_live_offers() {
        new_test_ext().execute_with(|| {
            // MaxPerMask = 2
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"")));
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(2), cat(b"")));
            assert_noop!(
                Market::publish(RuntimeOrigin::signed(1), h(3), cat(b"")),
                Error::<Test>::TooManyActiveOffers
            );
            // the rejected publish was atomic — it burned no id (NextId only advances on success)
            assert_eq!(Market::count(), 2);
            // withdrawing one frees a slot — the mask is NOT blocked forever
            assert_ok!(Market::withdraw(RuntimeOrigin::signed(1), 0));
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(3), cat(b"")));
            assert_eq!(Market::count(), 3); // 3 successful publishes (the capped one never got an id)
        });
    }

    #[test]
    fn hide_only_by_moderation_origin_keeps_entry_frees_slot() {
        new_test_ext().execute_with(|| {
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"")));
            assert_noop!(Market::hide(RuntimeOrigin::signed(1), 0), DispatchError::BadOrigin);
            assert_ok!(Market::hide(RuntimeOrigin::root(), 0));
            let o = Offers::<Test>::get(0).unwrap();
            assert!(o.hidden);
            assert_eq!(o.offerer, 1); // entry kept
            assert!(Market::active_of(&1).is_empty()); // slot freed
            assert_noop!(Market::hide(RuntimeOrigin::root(), 0), Error::<Test>::AlreadyHidden);
        });
    }

    #[test]
    fn withdraw_missing_offer_rejected() {
        new_test_ext().execute_with(|| {
            assert_noop!(Market::withdraw(RuntimeOrigin::signed(1), 0), Error::<Test>::OfferNotFound);
        });
    }

    #[test]
    fn hide_missing_offer_rejected() {
        new_test_ext().execute_with(|| {
            assert_noop!(Market::hide(RuntimeOrigin::root(), 0), Error::<Test>::OfferNotFound);
        });
    }

    #[test]
    fn category_bound_enforced() {
        new_test_ext().execute_with(|| {
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"sixteen_chars_ok")));
            let too_long: Result<BoundedVec<u8, ConstU32<16>>, _> =
                b"seventeen_chars__".to_vec().try_into();
            assert!(too_long.is_err());
        });
    }
}
