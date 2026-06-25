//! Harlequin tokens pallet — two coins (HLQ, SOV), thin FRAME wrapper over `tokens-core`.
//!
//! The pallet holds **balances** in storage; the **economic rules** live in the validated, dependency-free
//! `tokens-core` (15/15, no_std): the disinflationary [`EmissionCurve`] (rational decay, hard cap, the ONLY
//! source of new coin — no discretionary mint), the fee `burn_amount`, and `split_service_reward`. See
//! `../PALLET-TOKENS-DESIGN.md`.
//!
//! **Invariant §0 — money ≠ power (Art. VI).** This pallet moves and mints *money*; it confers NO voting
//! weight, committee seat, jury seat or reputation. It does not read or write reputation. Who governs is
//! `pallet-reputation`, an orthogonal axis. The two reward roles stay separate (SPEC §3.5): governing is by
//! reputation (elsewhere); maintaining the network is paid here by **proof-of-service**, and that pay grants
//! no vote.
//!
//! **No artificial inflation (Art. X, a hard design constraint).** New coin enters ONLY through the
//! pre-committed per-coin `EmissionCurve` at each era boundary (`on_initialize`), disinflationary with a
//! hard cap. SOV: hard cap (e.g. 54,000,000 — a deck of 54: 52 cards + 2 jokers, the joker *is* the
//! harlequin). HLQ: rule-bound emission + a partial **fee burn** sink so it tends to a use↔burn equilibrium,
//! never unbounded. There is no "mint N coins" call anywhere.
//!
//! ## Pre-mainnet notes (honest limitations)
//! - **Weights are placeholders, not benchmarked** (must be benchmarked before mainnet).
//! - The per-node anti-Sybil weight cap (§3.1) is applied here when building the reward vector; the
//!   `ServiceWeights` are recorded by a pluggable `ServiceOrigin` (the consensus/finality reporter), which a
//!   future increment wires to on-chain block authorship + finality (reusing the Snowball gadget).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// The two coins. Fixed set (the two coin names): HLQ "La Bolsa" (daily use) and SOV "El Cofre"
/// (Soberano, reserve).
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
pub enum Coin {
    /// HLQ — daily-use medium of exchange (SPEC §3.2).
    Hlq,
    /// SOV — Soberano, scarce store of value (SPEC §3.1).
    Sov,
}

impl Coin {
    /// Map to the `tokens-core` coin (kept identical in meaning).
    pub fn core(&self) -> tokens_core::Coin {
        match self {
            Coin::Hlq => tokens_core::Coin::Hlq,
            Coin::Sov => tokens_core::Coin::Sov,
        }
    }
}

/// A SCALE-encodable mirror of `tokens_core::EmissionCurve`, so the schedule can be a runtime constant
/// (`#[pallet::constant]`). Converts to the core type for the pure math. Numbers are runtime config
/// `[PARÁMETRO]` — never hard-coded here (SPEC §3.5 "nobody decrees the numbers").
#[derive(
    codec::Encode,
    codec::Decode,
    codec::MaxEncodedLen,
    scale_info::TypeInfo,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Debug,
)]
pub struct CurveParams {
    pub initial: u128,
    pub decay_num: u32,
    pub decay_den: u32,
    pub era_length: u64,
    pub cap: u128,
}

impl CurveParams {
    fn core(&self) -> tokens_core::EmissionCurve {
        tokens_core::EmissionCurve {
            initial: self.initial,
            decay_num: self.decay_num,
            decay_den: self.decay_den,
            era_length: self.era_length,
            cap: self.cap,
        }
    }
}

/// Per-account balances of both coins. Absent ⇒ zero of both.
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
    Debug,
    Default,
)]
pub struct Balances {
    pub hlq: u128,
    pub sov: u128,
}

#[frame::pallet]
pub mod pallet {
    use super::{Balances, Coin, CurveParams};
    use frame::prelude::*;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// The aggregate event type of the runtime.
        type RuntimeEvent: From<Event<Self>>
            + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// HLQ emission schedule (daily-use coin). Rule-bound; numbers decided by modelling, not decree.
        #[pallet::constant]
        type HlqCurve: Get<CurveParams>;

        /// SOV emission schedule (reserve). Hard cap (e.g. 54,000,000), disinflationary.
        #[pallet::constant]
        type SovCurve: Get<CurveParams>;

        /// Epochs (here = blocks) per emission era boundary at which `on_initialize` mints + distributes.
        #[pallet::constant]
        type EraLength: Get<BlockNumberFor<Self>>;

        /// Fraction of an HLQ fee that is BURNED (basis points, clamped to 10000). The rest goes to the
        /// processing node. TOKENOMICS §2: 5000 (50/50). A tunable dial (sane range 3000–7000).
        #[pallet::constant]
        type FeeBurnBps: Get<u16>;

        /// `k` in the per-node anti-Sybil reward cap `weight_i = min(weight_i, ceil(median·k))` (§3.1).
        /// Flattens the high tail so no operator (or Sybil) drains the pool. `k ≈ 2`.
        #[pallet::constant]
        type ServiceCapK: Get<u32>;

        /// Origin that records verified honest service (uptime + producing/finalising in turn, read from
        /// chain). Pluggable like `EvidenceOrigin` in pallet-reputation — the pallet trusts it to have
        /// verified the work; it confers NO power, only a claim on the reward pool.
        type ServiceOrigin: EnsureOrigin<Self::RuntimeOrigin>;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Balances of both coins per account.
    #[pallet::storage]
    pub type Accounts<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, Balances, ValueQuery>;

    /// Cumulative minted per coin (enforces the hard cap). Only `run_epoch` increases it.
    #[pallet::storage]
    pub type Minted<T: Config> = StorageMap<_, Blake2_128Concat, Coin, u128, ValueQuery>;

    /// Cumulative burned per coin (the deflationary fee sink). Circulating = minted − burned.
    #[pallet::storage]
    pub type Burned<T: Config> = StorageMap<_, Blake2_128Concat, Coin, u128, ValueQuery>;

    /// Current emission epoch (era counter), bumped each era boundary.
    #[pallet::storage]
    pub type Epoch<T> = StorageValue<_, u64, ValueQuery>;

    /// Verified honest-service weight accrued by each node THIS era (proof-of-service, §3.5(2)). Reset each
    /// era after the reward split. NOT reputation, NOT stake — only measured service.
    #[pallet::storage]
    pub type ServiceWeights<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u64, ValueQuery>;

    /// Genesis: a modest, declared founder allocation (SPEC §5, fair launch — no premine/ICO). The bulk of
    /// SOV is earned post-genesis by running the network. Reputation is NOT seeded here (money ≠ power).
    #[pallet::genesis_config]
    #[derive(frame::prelude::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        /// `(account, hlq, sov)` balances granted at genesis.
        pub balances: alloc::vec::Vec<(T::AccountId, u128, u128)>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            for (who, hlq, sov) in &self.balances {
                Accounts::<T>::insert(who, Balances { hlq: *hlq, sov: *sov });
                // Founder allocation counts as already-minted against the cap (transparent, not free supply).
                if *hlq > 0 {
                    Minted::<T>::mutate(Coin::Hlq, |m| *m = m.saturating_add(*hlq));
                }
                if *sov > 0 {
                    Minted::<T>::mutate(Coin::Sov, |m| *m = m.saturating_add(*sov));
                }
            }
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// `amount` of `coin` moved from `from` to `to`.
        Transferred { coin: Coin, from: T::AccountId, to: T::AccountId, amount: u128 },
        /// An HLQ fee: `burned` removed from supply, `to_node` paid to the processing node.
        FeeCharged { payer: T::AccountId, node: T::AccountId, burned: u128, to_node: u128 },
        /// `who` accrued `weight` of verified service this era.
        ServiceRecorded { who: T::AccountId, weight: u64 },
        /// A new era: `hlq`/`sov` emitted into the reward pool and split among `nodes` service providers.
        EraReward { epoch: u64, hlq: u128, sov: u128, nodes: u32 },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Not enough balance of the coin.
        InsufficientFunds,
        /// Arithmetic overflow (supply is cap-bounded; this signals misuse).
        Overflow,
        /// Zero amount or sending to oneself.
        InvalidTransfer,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// At every era boundary the chain mints the scheduled emission for both coins and distributes it to
        /// the nodes by verified service — by ITSELF, no privileged trigger (no king). Deterministic.
        fn on_initialize(n: BlockNumberFor<T>) -> Weight {
            let len = T::EraLength::get();
            if !n.is_zero() && !len.is_zero() && (n % len).is_zero() {
                Self::run_epoch();
            }
            Weight::from_parts(1_000_000_000, 0) // placeholder; benchmark + offchain-worker pre-mainnet
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Move `amount` of `coin` to `to`. Checked: rejects insufficient funds, overflow, self/zero.
        /// Conserves supply (mints nothing). Money only — confers no power.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn transfer(
            origin: OriginFor<T>,
            coin: Coin,
            to: T::AccountId,
            amount: u128,
        ) -> DispatchResult {
            let from = ensure_signed(origin)?;
            Self::do_transfer(coin, &from, &to, amount)?;
            Self::deposit_event(Event::Transferred { coin, from, to, amount });
            Ok(())
        }

        /// Record verified honest service for `who` this era (proof-of-service input to the reward split).
        /// Gated by `ServiceOrigin` — the pluggable, trusted reporter of on-chain participation. Adds to the
        /// running weight; confers NO power (Art. VI).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn record_service(origin: OriginFor<T>, who: T::AccountId, weight: u64) -> DispatchResult {
            T::ServiceOrigin::ensure_origin(origin)?;
            let total = ServiceWeights::<T>::mutate(&who, |w| {
                *w = w.saturating_add(weight);
                *w
            });
            Self::deposit_event(Event::ServiceRecorded { who, weight: total });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        fn get(coin: Coin, b: &Balances) -> u128 {
            match coin {
                Coin::Hlq => b.hlq,
                Coin::Sov => b.sov,
            }
        }
        fn set(coin: Coin, b: &mut Balances, v: u128) {
            match coin {
                Coin::Hlq => b.hlq = v,
                Coin::Sov => b.sov = v,
            }
        }

        /// Balance of `who` in `coin`.
        pub fn balance(who: &T::AccountId, coin: Coin) -> u128 {
            Self::get(coin, &Accounts::<T>::get(who))
        }

        /// Circulating supply of `coin` = minted − burned.
        pub fn circulating(coin: Coin) -> u128 {
            Minted::<T>::get(coin).saturating_sub(Burned::<T>::get(coin))
        }

        fn credit(coin: Coin, who: &T::AccountId, amount: u128) -> Result<(), Error<T>> {
            Accounts::<T>::try_mutate(who, |b| {
                let v = Self::get(coin, b).checked_add(amount).ok_or(Error::<T>::Overflow)?;
                Self::set(coin, b, v);
                Ok(())
            })
        }
        fn debit(coin: Coin, who: &T::AccountId, amount: u128) -> Result<(), Error<T>> {
            Accounts::<T>::try_mutate(who, |b| {
                let cur = Self::get(coin, b);
                if cur < amount {
                    return Err(Error::<T>::InsufficientFunds);
                }
                Self::set(coin, b, cur - amount);
                Ok(())
            })
        }

        /// Checked, atomic transfer (the rule mirrored from `tokens-core::Ledger::transfer`).
        pub fn do_transfer(
            coin: Coin,
            from: &T::AccountId,
            to: &T::AccountId,
            amount: u128,
        ) -> Result<(), Error<T>> {
            if amount == 0 || from == to {
                return Err(Error::<T>::InvalidTransfer);
            }
            Self::debit(coin, from, amount)?;
            if let Err(e) = Self::credit(coin, to, amount) {
                let _ = Self::credit(coin, from, amount); // restore
                return Err(e);
            }
            Ok(())
        }

        /// Charge an HLQ fee with the configured burn split (TOKENOMICS §2). `FeeBurnBps` of the fee is
        /// burned (leaves circulation), the rest pays the processing `node`. Mints nothing. Called by the
        /// runtime's transaction-payment hook, not by users.
        pub fn charge_fee(payer: &T::AccountId, node: &T::AccountId, fee: u128) -> Result<(), Error<T>> {
            if fee == 0 {
                return Err(Error::<T>::InvalidTransfer);
            }
            let burn = tokens_core::burn_amount(fee, T::FeeBurnBps::get());
            let to_node = fee - burn;
            Self::debit(Coin::Hlq, payer, fee)?;
            if to_node > 0 && payer != node {
                if let Err(e) = Self::credit(Coin::Hlq, node, to_node) {
                    let _ = Self::credit(Coin::Hlq, payer, fee);
                    return Err(e);
                }
            } else if payer == node {
                let _ = Self::credit(Coin::Hlq, payer, to_node);
            }
            if burn > 0 {
                Burned::<T>::mutate(Coin::Hlq, |x| *x = x.saturating_add(burn));
            }
            Self::deposit_event(Event::FeeCharged {
                payer: payer.clone(),
                node: node.clone(),
                burned: burn,
                to_node,
            });
            Ok(())
        }

        /// Mint a coin's scheduled emission for `epoch` into a pool (respecting the hard cap), returning the
        /// amount. Reuses `tokens-core` so the pallet enforces the same rule as the host engine.
        fn mint_pool(coin: Coin, params: CurveParams, epoch: u64) -> u128 {
            let minted = Minted::<T>::get(coin);
            let amount = params.core().mint_amount(epoch, minted);
            if amount > 0 {
                Minted::<T>::mutate(coin, |m| *m = m.saturating_add(amount));
            }
            amount
        }

        /// One era: mint HLQ + SOV emission, then split each pool among nodes by capped service weight.
        pub(crate) fn run_epoch() {
            let epoch = Epoch::<T>::get();

            // Collect this era's service weights in a fixed (account-ordered) vector.
            let mut who: alloc::vec::Vec<T::AccountId> = alloc::vec::Vec::new();
            let mut weights: alloc::vec::Vec<u64> = alloc::vec::Vec::new();
            for (acct, w) in ServiceWeights::<T>::iter() {
                if w > 0 {
                    who.push(acct);
                    weights.push(w);
                }
            }
            Self::cap_weights(&mut weights); // §3.1 anti-Sybil cap, BEFORE the split

            let hlq_pool = Self::mint_pool(Coin::Hlq, T::HlqCurve::get(), epoch);
            let sov_pool = Self::mint_pool(Coin::Sov, T::SovCurve::get(), epoch);

            if !who.is_empty() {
                let hlq_shares = tokens_core::split_service_reward(hlq_pool, &weights);
                let sov_shares = tokens_core::split_service_reward(sov_pool, &weights);
                for (i, acct) in who.iter().enumerate() {
                    if hlq_shares[i] > 0 {
                        let _ = Self::credit(Coin::Hlq, acct, hlq_shares[i]);
                    }
                    if sov_shares[i] > 0 {
                        let _ = Self::credit(Coin::Sov, acct, sov_shares[i]);
                    }
                }
            }

            // Clear the era's weights and advance.
            let _ = ServiceWeights::<T>::clear(u32::MAX, None);
            Epoch::<T>::put(epoch.saturating_add(1));
            Self::deposit_event(Event::EraReward {
                epoch,
                hlq: hlq_pool,
                sov: sov_pool,
                nodes: who.len() as u32,
            });
        }

        /// Anti-Sybil per-node cap (§3.1): `weight_i = min(weight_i, ceil(median·k))`. Flattens the high
        /// tail so no operator drains the pool. Mutates the weight vector in place before the split.
        pub(crate) fn cap_weights(weights: &mut [u64]) {
            if weights.is_empty() {
                return;
            }
            let mut sorted: alloc::vec::Vec<u64> = weights.to_vec();
            sorted.sort_unstable();
            let median = sorted[sorted.len() / 2];
            let k = T::ServiceCapK::get() as u128;
            // ceil(median * k): median*k is small (weights are bounded), u128 is safe.
            let cap = (median as u128).saturating_mul(k);
            let cap = if cap > u64::MAX as u128 { u64::MAX } else { cap as u64 };
            for w in weights.iter_mut() {
                if *w > cap {
                    *w = cap;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate as pallet_tokens;
    use crate::{Burned, Coin, CurveParams, Epoch, Error, Minted, ServiceWeights};
    use frame::testing_prelude::*;

    construct_runtime! {
        pub enum Test {
            System: frame_system,
            Tokens: pallet_tokens,
        }
    }

    #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
    impl frame_system::Config for Test {
        type Block = MockBlock<Test>;
    }

    parameter_types! {
        // HLQ: flat 1000/era, hard cap 10_000 (reached after 10 eras) — easy round numbers to assert.
        pub const TestHlqCurve: CurveParams =
            CurveParams { initial: 1000, decay_num: 1, decay_den: 1, era_length: 1, cap: 10_000 };
        // SOV: 100/era halving (1/2) every era, hard cap 150 — so the cap bites on era 1 (100 then 50).
        pub const TestSovCurve: CurveParams =
            CurveParams { initial: 100, decay_num: 1, decay_den: 2, era_length: 1, cap: 150 };
    }

    impl pallet_tokens::Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type HlqCurve = TestHlqCurve;
        type SovCurve = TestSovCurve;
        type EraLength = ConstU64<5>; // run_epoch fires at blocks 5,10,15,...
        type FeeBurnBps = ConstU16<5000>; // 50/50 burn split
        type ServiceCapK = ConstU32<2>; // anti-Sybil cap = ceil(median·2)
        type ServiceOrigin = EnsureRoot<Self::AccountId>;
    }

    fn new_test_ext() -> TestState {
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    fn ext_with_balances(balances: Vec<(u64, u128, u128)>) -> TestState {
        RuntimeGenesisConfig {
            system: Default::default(),
            tokens: pallet_tokens::GenesisConfig { balances },
        }
        .build_storage()
        .unwrap()
        .into()
    }

    // --- transfers: checked, supply-conserving, money-only ---

    #[test]
    fn transfer_moves_funds_and_conserves_supply() {
        ext_with_balances(vec![(1, 100, 0)]).execute_with(|| {
            assert_ok!(Tokens::transfer(RuntimeOrigin::signed(1), Coin::Hlq, 2, 40));
            assert_eq!(Tokens::balance(&1, Coin::Hlq), 60);
            assert_eq!(Tokens::balance(&2, Coin::Hlq), 40);
            // transfer mints nothing: total stays 100
            assert_eq!(
                Tokens::balance(&1, Coin::Hlq) + Tokens::balance(&2, Coin::Hlq),
                100
            );
        });
    }

    #[test]
    fn transfer_rejects_insufficient_self_and_zero() {
        ext_with_balances(vec![(1, 10, 0)]).execute_with(|| {
            assert_noop!(
                Tokens::transfer(RuntimeOrigin::signed(1), Coin::Hlq, 2, 50),
                Error::<Test>::InsufficientFunds
            );
            assert_noop!(
                Tokens::transfer(RuntimeOrigin::signed(1), Coin::Hlq, 1, 5),
                Error::<Test>::InvalidTransfer
            );
            assert_noop!(
                Tokens::transfer(RuntimeOrigin::signed(1), Coin::Hlq, 2, 0),
                Error::<Test>::InvalidTransfer
            );
        });
    }

    // --- fee burn split (TOKENOMICS §2: 50/50) ---

    #[test]
    fn fee_burns_half_and_pays_node() {
        ext_with_balances(vec![(1, 1000, 0)]).execute_with(|| {
            // founder genesis counts as minted; record baseline
            let minted0 = Minted::<Test>::get(Coin::Hlq);
            assert_ok!(Tokens::charge_fee(&1, &9, 100));
            // 50% burned, 50% to the node
            assert_eq!(Tokens::balance(&1, Coin::Hlq), 900);
            assert_eq!(Tokens::balance(&9, Coin::Hlq), 50);
            assert_eq!(Burned::<Test>::get(Coin::Hlq), 50);
            // burning removes from circulation; minting unchanged
            assert_eq!(Minted::<Test>::get(Coin::Hlq), minted0);
            assert_eq!(Tokens::circulating(Coin::Hlq), minted0 - 50);
        });
    }

    // --- proof-of-service recording: gated, confers no power ---

    #[test]
    fn record_service_requires_origin() {
        new_test_ext().execute_with(|| {
            // a plain signed account cannot self-assert service
            assert_noop!(
                Tokens::record_service(RuntimeOrigin::signed(1), 1, 10),
                DispatchError::BadOrigin
            );
            // the trusted ServiceOrigin (root in the mock) can, and weight accrues
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 1, 10));
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 1, 5));
            assert_eq!(ServiceWeights::<Test>::get(1u64), 15);
        });
    }

    // --- era reward: mint by curve, split by service, advance, clear ---

    #[test]
    fn era_mints_and_splits_by_service() {
        new_test_ext().execute_with(|| {
            // two equal-service nodes
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 1, 5));
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 2, 5));
            Tokens::run_epoch();
            // HLQ pool this era = 1000 (flat), split 50/50
            assert_eq!(Tokens::balance(&1, Coin::Hlq), 500);
            assert_eq!(Tokens::balance(&2, Coin::Hlq), 500);
            // SOV pool this era = 100, split 50/50
            assert_eq!(Tokens::balance(&1, Coin::Sov), 50);
            assert_eq!(Tokens::balance(&2, Coin::Sov), 50);
            // minted tracked; epoch advanced; weights cleared after the split
            assert_eq!(Minted::<Test>::get(Coin::Hlq), 1000);
            assert_eq!(Minted::<Test>::get(Coin::Sov), 100);
            assert_eq!(Epoch::<Test>::get(), 1);
            assert_eq!(ServiceWeights::<Test>::get(1u64), 0);
            assert_eq!(ServiceWeights::<Test>::get(2u64), 0);
        });
    }

    // --- anti-Sybil cap §3.1: ceil(median·k) applied BEFORE the split ---

    #[test]
    fn cap_weights_flattens_the_tail() {
        // sorted [1,1,1,100], median (index len/2=2) = 1, k=2 -> cap 2; whale 100 -> 2.
        let mut w = vec![1u64, 1, 100, 1];
        Tokens::cap_weights(&mut w);
        assert_eq!(w, vec![1, 1, 2, 1]);
    }

    #[test]
    fn whale_cannot_drain_the_pool() {
        new_test_ext().execute_with(|| {
            // three honest small nodes + one Sybil whale claiming 100x the service
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 1, 1));
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 2, 1));
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 3, 1));
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 9, 100));
            Tokens::run_epoch();
            let small = Tokens::balance(&1, Coin::Hlq);
            let whale = Tokens::balance(&9, Coin::Hlq);
            // capped to 2 vs small 1 -> whale gets ~2x a small node, NOT 100x
            assert!(whale <= small * 3, "whale {whale} small {small}: cap failed");
            assert!(small > 0);
        });
    }

    // --- hard cap: cumulative emission never exceeds the curve cap ---

    #[test]
    fn sov_hard_cap_is_never_exceeded() {
        new_test_ext().execute_with(|| {
            for _ in 0..20 {
                assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 1, 1));
                Tokens::run_epoch();
            }
            // SOV cap is 150 — cumulative minted must stop there, never above
            assert_eq!(Minted::<Test>::get(Coin::Sov), 150);
            assert!(Tokens::balance(&1, Coin::Sov) <= 150);
        });
    }

    // --- no-king: the era fires by block hook, not by a privileged call ---

    #[test]
    fn on_initialize_fires_only_on_era_boundary() {
        new_test_ext().execute_with(|| {
            assert_ok!(Tokens::record_service(RuntimeOrigin::root(), 1, 1));
            // non-boundary block: nothing happens
            Tokens::on_initialize(3);
            assert_eq!(Epoch::<Test>::get(), 0);
            assert_eq!(Minted::<Test>::get(Coin::Hlq), 0);
            // EraLength=5 boundary: the chain mints + distributes by itself
            Tokens::on_initialize(5);
            assert_eq!(Epoch::<Test>::get(), 1);
            assert_eq!(Tokens::balance(&1, Coin::Hlq), 1000);
        });
    }

    // --- genesis founder allocation is transparent: counts as already-minted ---

    #[test]
    fn genesis_founder_counts_against_the_cap() {
        ext_with_balances(vec![(1, 500, 200)]).execute_with(|| {
            assert_eq!(Tokens::balance(&1, Coin::Hlq), 500);
            assert_eq!(Tokens::balance(&1, Coin::Sov), 200);
            // declared, not free supply: minted == the allocation, circulating matches
            assert_eq!(Minted::<Test>::get(Coin::Hlq), 500);
            assert_eq!(Minted::<Test>::get(Coin::Sov), 200);
            assert_eq!(Tokens::circulating(Coin::Hlq), 500);
            assert_eq!(Tokens::circulating(Coin::Sov), 200);
        });
    }
}
