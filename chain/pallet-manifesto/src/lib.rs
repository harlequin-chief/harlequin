//! Harlequin manifesto pallet — **Art. XII, Permanence**.
//!
//! The manifesto is the constitution of the society. This pallet seals it into the chain at **genesis
//! (block 0)**: the canonical text and its SHA-256 are written once, in `genesis_build`, and there is
//! **no extrinsic that can ever change them**. Permanence is therefore not a promise — it is a property
//! of the chain. Anyone can fetch the text, recompute the hash with the same hand-rolled SHA-256 the
//! consensus uses (`manifesto-core`), and check it against `ManifestoHash`; the State, or a future
//! captured majority, cannot quietly rewrite the founding pact.
//!
//! The **final text enters only at the freeze**, after the adversarial audit (a placeholder is sealed
//! until then; see `project_manifesto_genesis_freeze`). The mechanism does not depend on the content.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

#[frame::pallet]
pub mod pallet {
    use super::*;
    use frame::prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// Maximum manifesto length in bytes (a constitution, not a database).
        #[pallet::constant]
        type MaxManifestoLen: Get<u32>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// The canonical manifesto bytes, sealed at genesis. Read-only forever (no mutator extrinsic).
    #[pallet::storage]
    pub type ManifestoText<T: Config> =
        StorageValue<_, BoundedVec<u8, <T as Config>::MaxManifestoLen>, OptionQuery>;

    /// SHA-256 of the canonical manifesto, sealed at genesis. The immutable fingerprint (Art. XII).
    #[pallet::storage]
    pub type ManifestoHash<T: Config> = StorageValue<_, [u8; 32], OptionQuery>;

    /// Genesis: the founding manifesto bytes. Empty = nothing sealed (a placeholder is used until the
    /// freeze). NB: there is deliberately **no call** to set or change this — permanence by construction.
    #[pallet::genesis_config]
    #[derive(frame::prelude::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub text: alloc::vec::Vec<u8>,
        #[serde(skip)]
        pub _marker: core::marker::PhantomData<T>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            if self.text.is_empty() {
                return;
            }
            // Seal: store the text (bounded) and its SHA-256 — the same hash the whole chain uses.
            let seal = manifesto_core::ManifestoSeal::seal(&self.text);
            let bounded: BoundedVec<u8, <T as Config>::MaxManifestoLen> = self
                .text
                .clone()
                .try_into()
                .expect("genesis manifesto exceeds MaxManifestoLen");
            ManifestoText::<T>::put(bounded);
            ManifestoHash::<T>::put(seal.hash);
        }
    }

    impl<T: Config> Pallet<T> {
        /// The sealed manifesto hash, if any (the integrity anchor any node can check against).
        pub fn manifesto_hash() -> Option<[u8; 32]> {
            ManifestoHash::<T>::get()
        }

        /// True iff `text` is exactly the manifesto sealed at genesis.
        pub fn verify(text: &[u8]) -> bool {
            match ManifestoHash::<T>::get() {
                Some(h) => manifesto_core::ManifestoSeal::from_hash(h).verify(text),
                None => false,
            }
        }
    }
}
