//! Harlequin forum index — an **append-only, ordered** index of posts on chain.
//!
//! The forum is decentralized: the post **body** lives off-chain in **IPFS** (content-addressed), and the
//! chain holds only a compact, ordered **index** entry per post — its author, the body's content hash (the
//! IPFS address), a monotonic order id, and an optional thread parent. This keeps the chain small while the
//! *ordering and authorship* — the parts that need consensus — are canonical and tamper-evident.
//!
//! **Moderation is by reputation, not authority.** A post is **hidden ONLY by a jury verdict**
//! (`pallet-justice`), routed here through a pluggable [`Config::ModerationOrigin`] — exactly the pattern the
//! other pallets use for their privileged-but-not-royal inputs (e.g. `ServiceOrigin`, `EvidenceOrigin`). There
//! is no admin delete and no author edit: the index is append-only, so the record of what was said — and that
//! a jury hid it — is itself non-repudiable. Hiding sets a flag; it never erases the entry (no memory-holing).
//!
//! **Anti-spam (pre-mainnet note).** This crate gates posting at `ensure_signed` only. Production should
//! additionally require a registered mask (`pallet-directory`) and either a small HLQ fee (`pallet-tokens`)
//! or a reputation floor; those couplings are deliberately left to the runtime wiring, not hard-coded here.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

/// The content hash of a post body (its IPFS address as a 32-byte multihash digest, e.g. CIDv1 raw+sha256).
/// The chain stores only this; the body is fetched from IPFS by the client and verified against it.
pub type BodyHash = [u8; 32];

#[frame::pallet]
pub mod pallet {
    use super::BodyHash;
    use frame::prelude::*;

    /// One forum index entry. The body is off-chain (IPFS at `body`); this is the consensus-ordered metadata.
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
    pub struct Post<T: Config> {
        /// Who authored it (the pseudonymous account / mask).
        pub author: T::AccountId,
        /// Content hash of the body (IPFS address). The chain never holds the text.
        pub body: BodyHash,
        /// Thread parent, if this is a reply; `None` for a root post. Must reference an existing post.
        pub parent: Option<u64>,
        /// Block at which it was indexed (ordering tiebreak / display).
        pub at: BlockNumberFor<T>,
        /// Hidden by a jury verdict. The entry remains (append-only); clients hide the body. Never erased.
        pub hidden: bool,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Origin allowed to hide a post — the **jury verdict** path (`pallet-justice`), pluggable like the
        /// other pallets' privileged origins. NOT a signed admin: moderation is by reputation, not authority.
        type ModerationOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Max thread nesting depth checked at post time (bounds reply chains / display cost). A reply's depth
        /// is its parent's depth + 1; roots are depth 0.
        #[pallet::constant]
        type MaxThreadDepth: Get<u32>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// The next post id to assign. Monotonic — the id IS the canonical global order (append-only).
    #[pallet::storage]
    pub type NextId<T> = StorageValue<_, u64, ValueQuery>;

    /// `id → post`. The ordered index. Ids are dense and increasing, so iterating ids is chronological order.
    #[pallet::storage]
    pub type Posts<T: Config> = StorageMap<_, Blake2_128Concat, u64, Post<T>, OptionQuery>;

    /// Depth of each post in its thread (root = 0), cached so a reply can be depth-checked in O(1).
    #[pallet::storage]
    pub type Depth<T> = StorageMap<_, Blake2_128Concat, u64, u32, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A post was indexed: `id` (its order), by `author`, body at `body`, replying to `parent`.
        Posted { id: u64, author: T::AccountId, body: BodyHash, parent: Option<u64> },
        /// A post was hidden by a jury verdict (the entry remains; only the body is hidden by clients).
        PostHidden { id: u64 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The referenced parent post does not exist.
        ParentNotFound,
        /// The reply would exceed `MaxThreadDepth`.
        ThreadTooDeep,
        /// No post with that id.
        PostNotFound,
        /// The post is already hidden.
        AlreadyHidden,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Index a post: publish its body content-hash (IPFS address) under the caller's authorship at the
        /// next order id. Append-only — there is no edit or delete. `parent` makes it a reply (must exist and
        /// stay within `MaxThreadDepth`).
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn post(origin: OriginFor<T>, body: BodyHash, parent: Option<u64>) -> DispatchResult {
            let author = ensure_signed(origin)?;

            let depth = match parent {
                None => 0u32,
                Some(pid) => {
                    ensure!(Posts::<T>::contains_key(pid), Error::<T>::ParentNotFound);
                    let d = Depth::<T>::get(pid).saturating_add(1);
                    ensure!(d <= T::MaxThreadDepth::get(), Error::<T>::ThreadTooDeep);
                    d
                }
            };

            let id = NextId::<T>::get();
            Posts::<T>::insert(
                id,
                Post::<T> {
                    author: author.clone(),
                    body,
                    parent,
                    at: frame_system::Pallet::<T>::block_number(),
                    hidden: false,
                },
            );
            Depth::<T>::insert(id, depth);
            NextId::<T>::put(id.saturating_add(1));
            Self::deposit_event(Event::Posted { id, author, body, parent });
            Ok(())
        }

        /// Hide a post. Gated by `ModerationOrigin` — i.e. a jury verdict, never a signed admin. The entry is
        /// kept (append-only, no memory-holing); the `hidden` flag tells clients not to render the body.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn hide_post(origin: OriginFor<T>, id: u64) -> DispatchResult {
            T::ModerationOrigin::ensure_origin(origin)?;
            Posts::<T>::try_mutate(id, |maybe| -> DispatchResult {
                let p = maybe.as_mut().ok_or(Error::<T>::PostNotFound)?;
                ensure!(!p.hidden, Error::<T>::AlreadyHidden);
                p.hidden = true;
                Ok(())
            })?;
            Self::deposit_event(Event::PostHidden { id });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Fetch a post by id (None if no such id).
        pub fn get(id: u64) -> Option<Post<T>> {
            Posts::<T>::get(id)
        }

        /// Total posts ever indexed (= next id), the canonical order bound.
        pub fn count() -> u64 {
            NextId::<T>::get()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_forum;
    use crate::{Error, Posts};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Forum: pallet_forum,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    impl pallet_forum::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        // In the mock, the moderation (jury verdict) origin stands in as root; the runtime wires it to the
        // real pallet-justice verdict path. Lets us assert "only the moderation origin can hide".
        type ModerationOrigin = EnsureRoot<Self::AccountId>;
        type MaxThreadDepth = ConstU32<3>;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    fn h(n: u8) -> [u8; 32] {
        [n; 32]
    }

    #[test]
    fn posts_are_ordered_append_only() {
        new_test_ext().execute_with(|| {
            assert_ok!(Forum::post(RuntimeOrigin::signed(1), h(1), None));
            assert_ok!(Forum::post(RuntimeOrigin::signed(2), h(2), None));
            // ids are the canonical monotonic order
            assert_eq!(Forum::get(0).unwrap().author, 1);
            assert_eq!(Forum::get(1).unwrap().author, 2);
            assert_eq!(Forum::get(0).unwrap().body, h(1));
            assert_eq!(Forum::count(), 2);
        });
    }

    #[test]
    fn replies_thread_and_respect_depth() {
        new_test_ext().execute_with(|| {
            assert_ok!(Forum::post(RuntimeOrigin::signed(1), h(1), None)); // id 0, depth 0
            assert_ok!(Forum::post(RuntimeOrigin::signed(2), h(2), Some(0))); // id 1, depth 1
            assert_ok!(Forum::post(RuntimeOrigin::signed(3), h(3), Some(1))); // id 2, depth 2
            assert_ok!(Forum::post(RuntimeOrigin::signed(4), h(4), Some(2))); // id 3, depth 3 (== max)
            // a reply at depth 4 exceeds MaxThreadDepth = 3
            assert_noop!(
                Forum::post(RuntimeOrigin::signed(5), h(5), Some(3)),
                Error::<Test>::ThreadTooDeep
            );
            assert_eq!(Forum::get(1).unwrap().parent, Some(0));
        });
    }

    #[test]
    fn reply_to_missing_parent_rejected() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                Forum::post(RuntimeOrigin::signed(1), h(1), Some(99)),
                Error::<Test>::ParentNotFound
            );
        });
    }

    #[test]
    fn hide_only_by_moderation_origin_keeps_entry() {
        new_test_ext().execute_with(|| {
            assert_ok!(Forum::post(RuntimeOrigin::signed(1), h(1), None));
            // a plain signed account (even the author) cannot hide — moderation is jury-only, not authority
            assert_noop!(Forum::hide_post(RuntimeOrigin::signed(1), 0), DispatchError::BadOrigin);
            // the moderation origin (jury verdict; root in the mock) hides it
            assert_ok!(Forum::hide_post(RuntimeOrigin::root(), 0));
            let p = Posts::<Test>::get(0).unwrap();
            assert!(p.hidden); // flagged...
            assert_eq!(p.author, 1); // ...but the entry (and its authorship) is NOT erased — no memory-holing
            // idempotency guard
            assert_noop!(Forum::hide_post(RuntimeOrigin::root(), 0), Error::<Test>::AlreadyHidden);
        });
    }

    #[test]
    fn hide_missing_post_rejected() {
        new_test_ext().execute_with(|| {
            assert_noop!(Forum::hide_post(RuntimeOrigin::root(), 0), Error::<Test>::PostNotFound);
        });
    }
}
