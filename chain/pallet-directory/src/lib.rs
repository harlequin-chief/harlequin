//! Harlequin mask directory — a censorship-resistant, **self-certifying** handle→account registry.
//!
//! A member's pseudonymous *mask* is published on-chain so others can discover and address it without any
//! central naming authority. The handle is **derived from the account's own public key**
//! (`handle = sha256(account)[..16]`), so:
//!
//! - **No squatting / no naming authority.** You cannot claim a handle you do not hold the key for — the
//!   handle IS a function of your key. There is no first-come land grab and nobody to petition for a name.
//! - **Self-certifying.** Anyone can verify `handle == derive(account)` offline; the chain only needs to
//!   publish *that the mask exists* (and resolve handle↔account for discovery).
//! - **Censorship-resistant.** Registration is a plain signed self-assertion (`ensure_signed`); no admin,
//!   no gate, no fee here. You register your own mask; nobody registers it for you and nobody can refuse it.
//!
//! **No session crypto here.** This pallet only binds a public handle to a public key. The chat's session
//! handshake / forward secrecy is a separate, **gated** concern (#637, built in a dedicated session); it is NOT
//! in this crate. Registration authorization is the member's existing login signature (the signed origin).
//!
//! The on-chain handle is the raw 16-byte truncated SHA-256 (the chain's canonical dep-free hash, reused
//! from `consensus-core`, the same primitive the beacon uses). The human-readable form (e.g. base32) is a
//! **client presentation** of these bytes; the client's derivation MUST match `Pallet::derive` exactly so
//! that `handle == derive(pubkey)` holds end to end. The chain is the canonical source of the derivation.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// A mask handle: the first 16 bytes of `sha256(account.encode())`. Self-certifying — derivable by anyone
/// from the account, so it cannot be squatted. Displayed client-side (e.g. base32); stored raw here.
pub type Handle = [u8; 16];

#[frame::pallet]
pub mod pallet {
    use super::Handle;
    use frame::prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// `account → handle`. Presence means the account has published a mask. Also the dedup guard: an account
    /// registers exactly one mask.
    #[pallet::storage]
    pub type HandleOf<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, Handle, OptionQuery>;

    /// `handle → account`. The resolution direction (discover/address a mask) and the collision guard.
    #[pallet::storage]
    pub type OwnerOf<T: Config> = StorageMap<_, Blake2_128Concat, Handle, T::AccountId, OptionQuery>;

    /// Number of published masks (directory size; cheap telemetry, no enumeration needed to count).
    #[pallet::storage]
    pub type MaskCount<T> = StorageValue<_, u64, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// `who` published the mask `handle` (`handle == derive(who)`).
        MaskRegistered { who: T::AccountId, handle: Handle },
        /// `who` withdrew its mask `handle` from the directory.
        MaskDeregistered { who: T::AccountId, handle: Handle },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The account already has a published mask (one mask per account).
        AlreadyRegistered,
        /// The account has no published mask to withdraw.
        NotRegistered,
        /// The derived handle is already taken — a SHA-256 collision (cryptographically negligible). Guarded
        /// so the registry can never bind one handle to two accounts.
        HandleCollision,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Publish the caller's mask. The handle is derived from the caller's account, so this is a pure
        /// self-assertion: no name is chosen, none can be squatted, and nobody can register on another's
        /// behalf. Idempotency: a second registration of an already-published account is rejected (use
        /// `deregister` first if re-publishing is ever needed).
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn register(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!HandleOf::<T>::contains_key(&who), Error::<T>::AlreadyRegistered);
            let handle = Self::derive(&who);
            ensure!(!OwnerOf::<T>::contains_key(handle), Error::<T>::HandleCollision);
            HandleOf::<T>::insert(&who, handle);
            OwnerOf::<T>::insert(handle, &who);
            MaskCount::<T>::mutate(|n| *n = n.saturating_add(1));
            Self::deposit_event(Event::MaskRegistered { who, handle });
            Ok(())
        }

        /// Withdraw the caller's mask from the directory. Only the holder can remove its own mask (signed
        /// origin); there is no admin removal — the directory cannot be censored by a third party.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn deregister(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let handle = HandleOf::<T>::take(&who).ok_or(Error::<T>::NotRegistered)?;
            OwnerOf::<T>::remove(handle);
            MaskCount::<T>::mutate(|n| *n = n.saturating_sub(1));
            Self::deposit_event(Event::MaskDeregistered { who, handle });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// The canonical mask-handle derivation: `sha256(account.encode())[..16]`. Reuses the chain's
        /// dep-free SHA-256 (the same primitive as the beacon) so the derivation is identical everywhere.
        /// The client MUST reproduce this exactly for `handle == derive(pubkey)` to hold off-chain.
        pub fn derive(who: &T::AccountId) -> Handle {
            let digest = consensus_core::sha256::sha256(&who.encode());
            let mut handle = [0u8; 16];
            handle.copy_from_slice(&digest[..16]);
            handle
        }

        /// Resolve a handle to the account that published it (discovery / addressing). `None` if unknown.
        pub fn resolve(handle: &Handle) -> Option<T::AccountId> {
            OwnerOf::<T>::get(handle)
        }

        /// The published handle of an account, if any.
        pub fn handle_of(who: &T::AccountId) -> Option<Handle> {
            HandleOf::<T>::get(who)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_directory;
    use crate::{Error, HandleOf, MaskCount, OwnerOf};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Directory: pallet_directory,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    impl pallet_directory::Config for Test {
        type RuntimeEvent = RuntimeEvent;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    #[test]
    fn register_binds_handle_to_account_self_certifying() {
        new_test_ext().execute_with(|| {
            assert_ok!(Directory::register(RuntimeOrigin::signed(1)));
            // the stored handle is exactly derive(account) — self-certifying, verifiable by anyone
            let h = Directory::handle_of(&1u64).expect("registered");
            assert_eq!(h, Directory::derive(&1u64));
            // resolves both ways
            assert_eq!(Directory::resolve(&h), Some(1u64));
            assert_eq!(MaskCount::<Test>::get(), 1);
        });
    }

    #[test]
    fn handle_is_deterministic_and_distinct_per_account() {
        new_test_ext().execute_with(|| {
            // deterministic: derive is a pure function of the account
            assert_eq!(Directory::derive(&7u64), Directory::derive(&7u64));
            // distinct accounts derive distinct handles
            assert_ne!(Directory::derive(&7u64), Directory::derive(&8u64));
        });
    }

    #[test]
    fn cannot_register_twice() {
        new_test_ext().execute_with(|| {
            assert_ok!(Directory::register(RuntimeOrigin::signed(1)));
            assert_noop!(
                Directory::register(RuntimeOrigin::signed(1)),
                Error::<Test>::AlreadyRegistered
            );
            assert_eq!(MaskCount::<Test>::get(), 1);
        });
    }

    #[test]
    fn no_squatting_handle_is_bound_to_its_owner_only() {
        new_test_ext().execute_with(|| {
            // account 2 registers and gets ITS handle; nobody else can hold account 2's handle because the
            // handle is derived from the key — there is no "choose a name" path to squat.
            assert_ok!(Directory::register(RuntimeOrigin::signed(2)));
            let h2 = Directory::handle_of(&2u64).unwrap();
            // account 3's mask is a different handle; account 3 can never publish under h2
            assert_ok!(Directory::register(RuntimeOrigin::signed(3)));
            assert_ne!(Directory::handle_of(&3u64).unwrap(), h2);
            assert_eq!(Directory::resolve(&h2), Some(2u64));
        });
    }

    #[test]
    fn deregister_only_by_holder_and_frees_the_entry() {
        new_test_ext().execute_with(|| {
            assert_ok!(Directory::register(RuntimeOrigin::signed(1)));
            let h = Directory::handle_of(&1u64).unwrap();
            // a non-holder cannot remove a mask it does not own (it simply has none to remove)
            assert_noop!(
                Directory::deregister(RuntimeOrigin::signed(99)),
                Error::<Test>::NotRegistered
            );
            // the holder withdraws; both directions are cleared
            assert_ok!(Directory::deregister(RuntimeOrigin::signed(1)));
            assert!(HandleOf::<Test>::get(1u64).is_none());
            assert!(OwnerOf::<Test>::get(h).is_none());
            assert_eq!(MaskCount::<Test>::get(), 0);
            // can re-publish after withdrawing
            assert_ok!(Directory::register(RuntimeOrigin::signed(1)));
            assert_eq!(Directory::handle_of(&1u64), Some(h));
        });
    }
}
