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
//! **Withdraw only — nothing is hidden.** The offerer may `withdraw` their own offer (a flag — the entry
//! is never erased, append-only, no memory-holing; frees the *active-offer* cap slot, which counts only
//! live listings). There is NO hide path (moderation-B doctrine): the chain keeps no moderation surface —
//! jury verdicts label conduct in `pallet-justice`, and clients/readers decide what to render.
//!
//! **Anti-spam.** A free, unbounded index is DoS-able, so every `publish` passes through a pluggable
//! [`PublishGate`] in `Config`. What the runtime ACTUALLY wires (28-sep-2026): the gate is still the no-op
//! `()` — no registered-mask check — and the real limits are the feeless lane's (per-mask budget per epoch,
//! per-block weight ceiling, per-block COUNT ceiling `MaxFeelessPerBlock`) plus the normal fee beyond them.
//! The hook stays so a stricter policy can activate without a rewrite (design/DECISION-E-GENESIS-2026-09-28.md).

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

    /// What the rest of the runtime learns when two masks close a deal (both signed). Reputation for
    /// commerce hangs here; the default does nothing, so the market works without it.
    pub trait OnDealClosed<AccountId> {
        fn on_deal_closed(offer: u64, a: &AccountId, b: &AccountId);
    }
    impl<AccountId> OnDealClosed<AccountId> for () {
        fn on_deal_closed(_offer: u64, _a: &AccountId, _b: &AccountId) {}
    }

    /// A deal over an offer: proposed by one party, closed when the OTHER one confirms. The chain holds
    /// no money here — it records that two masks both say the deal happened (proposal, 2026-09-27).
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
    pub struct Deal<T: Config> {
        pub offer: u64,
        pub proposer: T::AccountId,
        pub counterparty: T::AccountId,
        pub at: BlockNumberFor<T>,
        /// Last block the counterparty can confirm; after it an unconfirmed proposal can be reaped by anyone.
        pub expires: BlockNumberFor<T>,
        pub closed: bool,
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
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Pre-publish gate (anti-spam). Default `()` = no-op; production wires the B6 micro-fee here.
        type Postage: PublishGate<Self::AccountId>;
        type OnDealClosed: OnDealClosed<Self::AccountId>;

        /// Max unconfirmed deal proposals a mask may have open at once (free accounts × free writes must not
        /// become eternal rows: lesson of #57/#61).
        #[pallet::constant]
        type MaxOpenDealsPerMask: Get<u32>;

        /// Blocks the counterparty has to confirm a proposal.
        #[pallet::constant]
        type DealTtl: Get<BlockNumberFor<Self>>;

        /// Max length of the optional category tag.
        #[pallet::constant]
        type MaxCategoryLen: Get<u32>;

        /// Max number of *active* (not withdrawn) offers a single mask may hold at once.
        #[pallet::constant]
        type MaxPerMask: Get<u32>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// The next offer id to assign. Monotonic — the id IS the canonical global order (append-only).
    #[pallet::storage]
    pub type NextId<T> = StorageValue<_, u64, ValueQuery>;
    #[pallet::storage]
    pub type NextDealId<T> = StorageValue<_, u64, ValueQuery>;
    #[pallet::storage]
    pub type Deals<T: Config> = StorageMap<_, Blake2_128Concat, u64, Deal<T>, OptionQuery>;
    /// `proposer → its unconfirmed proposals`. Bounded by `MaxOpenDealsPerMask`.
    #[pallet::storage]
    pub type OpenDealsOf<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// `id → offer`. The ordered index. Ids are dense and increasing, so iterating ids is chronological.
    /// Append-only: entries are never removed (withdraw sets a flag), so the record is non-repudiable.
    #[pallet::storage]
    pub type Offers<T: Config> = StorageMap<_, Blake2_128Concat, u64, Offer<T>, OptionQuery>;

    /// `mask → its ACTIVE offer ids`. Bounded by `MaxPerMask`. `publish` adds; `withdraw` removes —
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
        DealProposed { deal: u64, offer: u64, proposer: T::AccountId, counterparty: T::AccountId },
        DealClosed { deal: u64, offer: u64 },
        DealCancelled { deal: u64 },
        DealReaped { deal: u64 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// No offer with that id.
        OfferNotFound,
        /// The caller is not the offer's offerer.
        NotYourOffer,
        /// The offer is already withdrawn.
        AlreadyWithdrawn,
        /// The mask already holds `MaxPerMask` active offers.
        TooManyActiveOffers,
        /// Neither side of the proposed deal is the offerer.
        NotAPartyToOffer,
        /// A mask cannot close a deal with itself.
        SelfDeal,
        DealNotFound,
        /// Only the named counterparty confirms; only the proposer cancels.
        NotYourDeal,
        DealAlreadyClosed,
        /// The proposer already has `MaxOpenDealsPerMask` unconfirmed proposals.
        TooManyOpenDeals,
        /// The confirmation window closed.
        DealExpired,
        /// Still inside its confirmation window.
        DealNotExpired,
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

        /// Propose a deal over `offer` with `counterparty`. One of the two must be the offerer (the seller
        /// proposes to a buyer, or a buyer to the seller); a withdrawn offer takes no deals.
        #[pallet::call_index(2)]
        #[pallet::weight(T::DbWeight::get().reads_writes(3, 3).saturating_add(Weight::from_parts(15_000_000, 400)))]
        pub fn propose_deal(origin: OriginFor<T>, offer: u64, counterparty: T::AccountId) -> DispatchResult {
            let proposer = ensure_signed(origin)?;
            ensure!(proposer != counterparty, Error::<T>::SelfDeal);
            let o = Offers::<T>::get(offer).ok_or(Error::<T>::OfferNotFound)?;
            ensure!(!o.withdrawn, Error::<T>::AlreadyWithdrawn);
            ensure!(o.offerer == proposer || o.offerer == counterparty, Error::<T>::NotAPartyToOffer);
            OpenDealsOf::<T>::try_mutate(&proposer, |n| -> DispatchResult {
                ensure!(*n < T::MaxOpenDealsPerMask::get(), Error::<T>::TooManyOpenDeals);
                *n += 1;
                Ok(())
            })?;
            let now = frame_system::Pallet::<T>::block_number();
            let deal = NextDealId::<T>::get();
            Deals::<T>::insert(
                deal,
                Deal::<T> {
                    offer,
                    proposer: proposer.clone(),
                    counterparty: counterparty.clone(),
                    at: now,
                    expires: now.saturating_add(T::DealTtl::get()),
                    closed: false,
                },
            );
            NextDealId::<T>::put(deal.saturating_add(1));
            Self::deposit_event(Event::DealProposed { deal, offer, proposer, counterparty });
            Ok(())
        }

        /// The named counterparty confirms: with both signatures the deal is CLOSED, for good.
        #[pallet::call_index(3)]
        #[pallet::weight(T::DbWeight::get().reads_writes(3, 2).saturating_add(Weight::from_parts(15_000_000, 400)))]
        pub fn confirm_deal(origin: OriginFor<T>, deal: u64) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let d = Deals::<T>::try_mutate(deal, |maybe| -> Result<Deal<T>, DispatchError> {
                let d = maybe.as_mut().ok_or(Error::<T>::DealNotFound)?;
                ensure!(d.counterparty == who, Error::<T>::NotYourDeal);
                ensure!(!d.closed, Error::<T>::DealAlreadyClosed);
                ensure!(frame_system::Pallet::<T>::block_number() <= d.expires, Error::<T>::DealExpired);
                // a deal over an offer withdrawn after the proposal would close something no longer on offer
                let o = Offers::<T>::get(d.offer).ok_or(Error::<T>::OfferNotFound)?;
                ensure!(!o.withdrawn, Error::<T>::AlreadyWithdrawn);
                d.closed = true;
                Ok(d.clone())
            })?;
            Self::free_deal_slot(&d.proposer);
            T::OnDealClosed::on_deal_closed(d.offer, &d.proposer, &d.counterparty);
            Self::deposit_event(Event::DealClosed { deal, offer: d.offer });
            Ok(())
        }

        /// The proposer withdraws a proposal the other side has not confirmed yet. The record stays.
        #[pallet::call_index(4)]
        #[pallet::weight(T::DbWeight::get().reads_writes(1, 2).saturating_add(Weight::from_parts(10_000_000, 400)))]
        pub fn cancel_deal(origin: OriginFor<T>, deal: u64) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let d = Deals::<T>::get(deal).ok_or(Error::<T>::DealNotFound)?;
            ensure!(d.proposer == who, Error::<T>::NotYourDeal);
            ensure!(!d.closed, Error::<T>::DealAlreadyClosed);
            Deals::<T>::remove(deal);
            Self::free_deal_slot(&who);
            Self::deposit_event(Event::DealCancelled { deal });
            Ok(())
        }

        /// Anyone clears an unconfirmed proposal whose window has closed (the proposer's slot comes back).
        #[pallet::call_index(5)]
        #[pallet::weight(T::DbWeight::get().reads_writes(1, 2).saturating_add(Weight::from_parts(10_000_000, 200)))]
        pub fn reap_deal(origin: OriginFor<T>, deal: u64) -> DispatchResult {
            ensure_signed(origin)?;
            let d = Deals::<T>::get(deal).ok_or(Error::<T>::DealNotFound)?;
            ensure!(!d.closed, Error::<T>::DealAlreadyClosed);
            ensure!(frame_system::Pallet::<T>::block_number() > d.expires, Error::<T>::DealNotExpired);
            Deals::<T>::remove(deal);
            Self::free_deal_slot(&d.proposer);
            Self::deposit_event(Event::DealReaped { deal });
            Ok(())
        }

    }

    impl<T: Config> Pallet<T> {
        fn free_deal_slot(proposer: &T::AccountId) {
            OpenDealsOf::<T>::mutate_exists(proposer, |n| {
                let left = n.unwrap_or(0).saturating_sub(1);
                *n = if left == 0 { None } else { Some(left) };
            });
        }

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
        // Mock: no anti-spam gate (no-op); production wires the B6 micro-fee here.
        type Postage = ();
        type OnDealClosed = ();
        type MaxOpenDealsPerMask = ConstU32<2>;
        type DealTtl = ConstU64<10>;
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
    fn withdraw_missing_offer_rejected() {
        new_test_ext().execute_with(|| {
            assert_noop!(Market::withdraw(RuntimeOrigin::signed(1), 0), Error::<Test>::OfferNotFound);
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

    // ── Adversarial pass () ────────────────────────────────────────────────────────────
    // The tests above cover ordering, ownership and the cap. These attack the market's claims:
    // that there is no privileged seller, that ids are never reused, and that a rejected publish
    // leaves nothing behind.

    #[test]
    fn no_privileged_origin_can_publish_or_withdraw() {
        new_test_ext().execute_with(|| {
            // A market with an admin is not a free market. Root/none must be as powerless as anyone.
            assert!(Market::publish(RuntimeOrigin::root(), h(1), cat(b"tools")).is_err());
            assert!(Market::publish(RuntimeOrigin::none(), h(1), cat(b"tools")).is_err());
            assert_eq!(Market::count(), 0);

            // And no privileged takedown of somebody else's offer.
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"tools")));
            assert!(Market::withdraw(RuntimeOrigin::root(), 0).is_err());
            assert!(Market::withdraw(RuntimeOrigin::none(), 0).is_err());
            let o = Market::get(0).expect("offer survives");
            assert!(!o.withdrawn, "a privileged origin took down an offer");
        });
    }

    #[test]
    fn rejected_publish_burns_no_id_and_leaves_no_trace() {
        new_test_ext().execute_with(|| {
            // Fill the mask's cap (MaxPerMask = 2).
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"a")));
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(2), cat(b"b")));
            assert_eq!(Market::count(), 2);

            // The third is refused. The id counter must NOT advance and no ghost entry may be stored:
            // ids are the canonical order bound, so a burnt id would leave a permanent hole.
            assert_noop!(
                Market::publish(RuntimeOrigin::signed(1), h(3), cat(b"c")),
                Error::<Test>::TooManyActiveOffers
            );
            assert_eq!(Market::count(), 2, "a rejected publish consumed an id");
            assert!(Offers::<Test>::get(2).is_none(), "a rejected publish left a ghost offer");

            // Freeing a slot lets the mask publish again, and the new offer takes the NEXT id (append-only).
            assert_ok!(Market::withdraw(RuntimeOrigin::signed(1), 0));
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(3), cat(b"c")));
            assert_eq!(Market::count(), 3);
            assert_eq!(Market::get(2).expect("published").detail, h(3));
        });
    }

    #[test]
    fn withdraw_is_idempotent_and_cannot_be_replayed_to_free_extra_slots() {
        new_test_ext().execute_with(|| {
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"a")));
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(2), cat(b"b")));
            assert_ok!(Market::withdraw(RuntimeOrigin::signed(1), 0));

            // Replaying the same withdrawal must fail — otherwise it would keep shrinking the active
            // list and could be used to slip past the per-mask cap.
            assert_noop!(
                Market::withdraw(RuntimeOrigin::signed(1), 0),
                Error::<Test>::AlreadyWithdrawn
            );
            assert_eq!(Market::active_of(&1u64).len(), 1, "replay corrupted the active list");

            // The cap still binds: one live offer + one new publish = 2, a third is still refused.
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(3), cat(b"c")));
            assert_noop!(
                Market::publish(RuntimeOrigin::signed(1), h(4), cat(b"d")),
                Error::<Test>::TooManyActiveOffers
            );
        });
    }

    #[test]
    fn a_strangers_failed_withdrawal_touches_nothing() {
        new_test_ext().execute_with(|| {
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"a")));
            assert_ok!(Market::publish(RuntimeOrigin::signed(2), h(2), cat(b"b")));

            assert_noop!(
                Market::withdraw(RuntimeOrigin::signed(2), 0),
                Error::<Test>::NotYourOffer
            );
            // Neither the victim's offer nor the attacker's own active list may move.
            assert!(!Market::get(0).expect("offer 0").withdrawn);
            assert_eq!(Market::active_of(&1u64).len(), 1);
            assert_eq!(Market::active_of(&2u64).len(), 1);
        });
    }

    #[test]
    fn deal_closes_only_with_both_signatures() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"tools")));
            // a stranger cannot put words in the offerer's mouth
            assert_noop!(Market::propose_deal(RuntimeOrigin::signed(3), 0, 2), Error::<Test>::NotAPartyToOffer);
            assert_noop!(Market::propose_deal(RuntimeOrigin::signed(1), 0, 1), Error::<Test>::SelfDeal);
            // the buyer proposes to the seller
            assert_ok!(Market::propose_deal(RuntimeOrigin::signed(2), 0, 1));
            assert!(!crate::Deals::<Test>::get(0).unwrap().closed);
            // only the named counterparty confirms
            assert_noop!(Market::confirm_deal(RuntimeOrigin::signed(2), 0), Error::<Test>::NotYourDeal);
            assert_noop!(Market::confirm_deal(RuntimeOrigin::signed(3), 0), Error::<Test>::NotYourDeal);
            assert_ok!(Market::confirm_deal(RuntimeOrigin::signed(1), 0));
            assert!(crate::Deals::<Test>::get(0).unwrap().closed);
            assert_noop!(Market::confirm_deal(RuntimeOrigin::signed(1), 0), Error::<Test>::DealAlreadyClosed);
            // a closed deal cannot be cancelled away
            assert_noop!(Market::cancel_deal(RuntimeOrigin::signed(2), 0), Error::<Test>::DealAlreadyClosed);
        });
    }

    #[test]
    fn deals_respect_withdrawn_offers_and_cancel_is_proposer_only() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"")));
            assert_ok!(Market::propose_deal(RuntimeOrigin::signed(1), 0, 2)); // seller proposes
            assert_noop!(Market::cancel_deal(RuntimeOrigin::signed(2), 0), Error::<Test>::NotYourDeal);
            assert_ok!(Market::cancel_deal(RuntimeOrigin::signed(1), 0));
            assert!(crate::Deals::<Test>::get(0).is_none());
            assert_noop!(Market::propose_deal(RuntimeOrigin::signed(2), 9, 1), Error::<Test>::OfferNotFound);
            assert_ok!(Market::withdraw(RuntimeOrigin::signed(1), 0));
            assert_noop!(Market::propose_deal(RuntimeOrigin::signed(2), 0, 1), Error::<Test>::AlreadyWithdrawn);
        });
    }

    #[test]
    fn open_deal_proposals_are_capped_and_expire() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"")));
            assert_ok!(Market::propose_deal(RuntimeOrigin::signed(2), 0, 1));
            assert_ok!(Market::propose_deal(RuntimeOrigin::signed(2), 0, 1));
            assert_noop!(Market::propose_deal(RuntimeOrigin::signed(2), 0, 1), Error::<Test>::TooManyOpenDeals);
            // the window closes: no late confirmation, and anyone can clear it
            System::set_block_number(12);
            assert_noop!(Market::confirm_deal(RuntimeOrigin::signed(1), 0), Error::<Test>::DealExpired);
            assert_ok!(Market::reap_deal(RuntimeOrigin::signed(9), 0));
            assert_ok!(Market::propose_deal(RuntimeOrigin::signed(2), 0, 1));
            // a confirmed deal frees the proposer's slot too
            assert_ok!(Market::confirm_deal(RuntimeOrigin::signed(1), 2));
            assert_ok!(Market::propose_deal(RuntimeOrigin::signed(2), 0, 1));
        });
    }

    #[test]
    fn no_confirmation_over_an_offer_withdrawn_after_the_proposal() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            assert_ok!(Market::publish(RuntimeOrigin::signed(1), h(1), cat(b"")));
            assert_ok!(Market::propose_deal(RuntimeOrigin::signed(1), 0, 2));
            assert_ok!(Market::withdraw(RuntimeOrigin::signed(1), 0));
            assert_noop!(Market::confirm_deal(RuntimeOrigin::signed(2), 0), Error::<Test>::AlreadyWithdrawn);
        });
    }
}
