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
//! **No artificial inflation (Art. X, the maintainer's hard constraint).** New coin enters ONLY through the
//! pre-committed per-coin `EmissionCurve` at each era boundary (`on_initialize`), disinflationary with a
//! hard cap. SOV: hard cap (e.g. 54,000,000 — a deck of 54: 52 cards + 2 jokers, the joker *is* the
//! harlequin). HLQ: rule-bound emission + a partial **fee burn** sink so it tends to a use↔burn equilibrium,
//! never unbounded. There is no "mint N coins" call anywhere.
//!
//! ## Pre-mainnet notes (honest limitations)
//! - **Weights are placeholders, not benchmarked** (must be benchmarked before mainnet).
//! - The per-node anti-Sybil weight cap (§3.1) is applied here when building the reward vector; the service
//!   `(account, weight)` pairs come from a pluggable [`EraServiceSource`] (P-2/#838), wired in the runtime
//!   to pallet-participation's non-forgeable author/finality feed — NOT a Root-gated call, so emission
//!   mints on a king-less mainnet.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub use pallet::*;

/// The two coins. Fixed set (LORE / the maintainer): HLQ "La Bolsa" (daily use) and SOV "El Cofre"
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

    /// The non-forgeable source of verified per-era service (P-2/#838). The emission reward split reads the
    /// `(account, weight)` pairs from here at each era boundary and then clears the era. In the runtime this
    /// is wired to `pallet-participation`'s `EraServiceAccumulator` — fed ONLY by the block-author inherent
    /// (`note_participation`, whose finality votes are sr25519 + committee verified and whose author is
    /// bound to the signing vote key by P-1). Emission therefore needs NO privileged/Root origin, so it
    /// mints on a king-less mainnet (Root is compiled out). Replaces the dead `record_service` /
    /// `ServiceWeights` / `ServiceOrigin` path, which could never populate service on mainnet 0-Sudo.
    pub trait EraServiceSource<AccountId> {
        /// Accumulated verified service `(account, weight)` for `era`.
        fn era_service(era: u64) -> alloc::vec::Vec<(AccountId, u64)>;
        /// Clear `era`'s accumulator once the reward split has paid it (called right after the split).
        fn clear_era(era: u64);
    }

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

        /// Source of verified per-era service, read at each era boundary to build the reward split
        /// (P-2/#838). Wired to `pallet-participation` in the runtime — the non-forgeable author/vote
        /// inherent feed, NOT a Root-gated call. Confers NO power, only a claim on the reward pool.
        type ServiceSource: EraServiceSource<Self::AccountId>;

        /// Ceiling for FEELESS extrinsics per block, in basis points of the max block weight
        /// (SPEC-RELAUNCH §2 R1(i)). Default 1500 = 15%: a mask farm can never fill blocks for free.
        #[pallet::constant]
        type FeelessBlockBps: Get<u16>;

        /// Blocks per FEELESS budget epoch (the per-mask allowance resets each such epoch and is NOT
        /// cumulative — SPEC-RELAUNCH §2 R1(iii)). ~1 day of blocks.
        #[pallet::constant]
        type FeelessEpochBlocks: Get<BlockNumberFor<Self>>;
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

    /// FEELESS accounting (SPEC-RELAUNCH §2): budget-epoch index in which an account was first seen
    /// (its "age" clock — waiting is the only way to age, so farming masks buys nothing today).
    #[pallet::storage]
    pub type FeelessFirstSeen<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u32>;

    /// FEELESS uses spent by an account: `(budget_epoch_index, used)`. Resets by rollover (non-cumulative).
    #[pallet::storage]
    pub type FeelessUsed<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, (u32, u32), ValueQuery>;

    /// Ref-time weight consumed by FEELESS extrinsics in the CURRENT block (killed each `on_initialize`).
    /// Enforces the global per-block ceiling (`FeelessBlockBps`).
    #[pallet::storage]
    pub type FeelessWeightThisBlock<T> = StorageValue<_, u64, ValueQuery>;

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
            // T-HIGH1 fix (audit): reject a chain-spec that lists the same account twice. Without
            // this, a duplicate entry OVERWRITES the balance (`Accounts::insert`) but `Minted` ACCUMULATES
            // both amounts against the hard cap AND `inc_sufficients` fires twice (an unbalanced existence
            // reference with no matching `dec`) — a silent, irreversible cap corruption baked into the sealed
            // genesis. Genesis is hand-authored (no tooling), so this guard is load-bearing.
            let mut seen = alloc::collections::BTreeSet::new();
            for (who, _, _) in &self.balances {
                assert!(seen.insert(who.clone()), "duplicate account in genesis token balances");
            }
            for (who, hlq, sov) in &self.balances {
                Accounts::<T>::insert(who, Balances { hlq: *hlq, sov: *sov });
                // E4 fix: a genesis-funded account materialises too (existence reference), so it can pay
                // without a phantom native balance — and so the `dec` in `debit` on spend-to-zero is
                // balanced (one inc here, one dec there).
                if hlq.saturating_add(*sov) > 0 {
                    let _ = frame_system::Pallet::<T>::inc_sufficients(who);
                }
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
        fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
            // fresh block → fresh feeless ceiling (the global per-block cap is per-block state).
            FeelessWeightThisBlock::<T>::kill();
            Weight::from_parts(1_000_000_000, 0) // placeholder; benchmark + offchain-worker pre-mainnet
        }

        /// Emission runs in `on_finalize` (P-2/#838), NOT `on_initialize`. `frame_executive` runs ALL
        /// pallets' `on_initialize` (block start) before ANY `on_finalize` (block end). pallet-participation
        /// (index 12) folds the era's last epoch into its `EraServiceAccumulator` in ITS `on_initialize`, so
        /// by the time this runs the era is COMPLETE. The old `on_initialize` mint ran BEFORE that fold →
        /// it lost the final epoch (~1/183 on mainnet) and stranded its weight; this is exactly-once and
        /// loss-free. Depends on `EraLength` being an exact multiple of participation's `EpochLength` (P-5)
        /// so no epoch straddles the era boundary.
        fn on_finalize(n: BlockNumberFor<T>) {
            let len = T::EraLength::get();
            if !n.is_zero() && !len.is_zero() && (n % len).is_zero() {
                Self::run_epoch();
            }
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

        // `record_service` (call_index 1) REMOVED in P-2/#838: it was gated by `ServiceOrigin = EnsureRoot`,
        // and Root is compiled out of mainnet (king-less, K1) → it could NEVER populate service there, so
        // emission minted nothing. Service now comes from the non-forgeable participation feed via
        // `T::ServiceSource` (read in `run_epoch`). No second, unreachable path. call_index 0 (transfer)
        // is unchanged; no index is reused (1 is simply retired) so historical encodings never collide.
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
                let total_before = b.hlq.saturating_add(b.sov);
                let v = Self::get(coin, b).checked_add(amount).ok_or(Error::<T>::Overflow)?;
                Self::set(coin, b, v);
                // E4 fix (, the reviewer's stress catch): pallet-tokens is a SUFFICIENT ledger.
                // Receiving Harlequin coin MATERIALISES the account — it takes a `frame_system` existence
                // reference (sufficient), so its signed extrinsics validate. Without this, an account funded
                // only via a Tokens transfer keeps `providers==0 && sufficients==0`, and `HlqCheckNonce`
                // treats it as "not yet a mask" → InvalidTransaction::Payment on ANY paying call. On the
                // launch genesis (no native `pallet_balances` for anyone) that would brick the whole
                // citizen economy: earn HLQ, never able to spend it. One inc per 0→positive transition,
                // matched by the dec in `debit` on positive→0 — balanced, independent of the newborn
                // feeless bootstrap's own reference in `HlqCheckNonce`.
                if total_before == 0 && amount > 0 {
                    let _ = frame_system::Pallet::<T>::inc_sufficients(who);
                }
                Ok(())
            })
        }
        fn debit(coin: Coin, who: &T::AccountId, amount: u128) -> Result<(), Error<T>> {
            Accounts::<T>::try_mutate_exists(who, |maybe_b| {
                let b = maybe_b.get_or_insert_with(Balances::default);
                let cur = Self::get(coin, b);
                if cur < amount {
                    return Err(Error::<T>::InsufficientFunds);
                }
                Self::set(coin, b, cur - amount);
                // Fully spent (both coins zero): reap the ledger entry and release the existence reference,
                // so `frame_system` can reap the account when its last ref drops (mirror of `credit`).
                if b.hlq == 0 && b.sov == 0 {
                    *maybe_b = None;
                    let _ = frame_system::Pallet::<T>::dec_sufficients(who);
                }
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

        /// Fee-side primitives for the runtime's `OnChargeTransaction` adapter (SPEC-RELAUNCH §2 — fees
        /// are paid in HLQ, the daily-use coin, never in a phantom native balance). Withdraw-then-settle
        /// split so the adapter can refund the weight difference before the burn/author split.
        /// Secure the (maximum) fee up front. Fails ⇒ the extrinsic is invalid (payment).
        pub fn fee_withdraw(payer: &T::AccountId, amount: u128) -> Result<(), Error<T>> {
            if amount == 0 {
                return Ok(());
            }
            Self::debit(Coin::Hlq, payer, amount)
        }

        /// Return an over-withdrawn remainder to the payer (actual fee < secured fee).
        pub fn fee_refund(payer: &T::AccountId, amount: u128) {
            if amount > 0 {
                let _ = Self::credit(Coin::Hlq, payer, amount);
            }
        }

        /// Settle an already-withdrawn fee: `FeeBurnBps` is burned; the rest pays the block `author`
        /// (the node that did the work). No author known ⇒ the whole fee is BURNED — public and
        /// non-discretionary, never a pot anyone controls (Art. VI: no hidden hand).
        pub fn fee_settle(payer: &T::AccountId, author: Option<&T::AccountId>, fee: u128) {
            if fee == 0 {
                return;
            }
            let burn = tokens_core::burn_amount(fee, T::FeeBurnBps::get());
            let to_node = fee - burn;
            let (burned, node) = match author {
                Some(node) if to_node > 0 => {
                    let _ = Self::credit(Coin::Hlq, node, to_node);
                    (burn, node.clone())
                }
                _ => (fee, payer.clone()), // no author → burn everything (event still names the payer)
            };
            if burned > 0 {
                Burned::<T>::mutate(Coin::Hlq, |x| *x = x.saturating_add(burned));
            }
            Self::deposit_event(Event::FeeCharged {
                payer: payer.clone(),
                node,
                burned,
                to_node: fee - burned,
            });
        }

        /// FEELESS gate (SPEC-RELAUNCH §2, fix for audit 🔴#1's bootstrap chicken-and-egg): try to consume
        /// one feeless use for `who` at this extrinsic's `ref_time` weight. `allowance` (uses per budget
        /// epoch) is computed by the RUNTIME from age + reputation — this pallet only does the bookkeeping:
        ///   (i)   global per-block ceiling: feeless weight ≤ `FeelessBlockBps` of the max block weight;
        ///   (ii)  per-mask budget: `used < allowance` within the current budget epoch (rollover resets);
        ///   (iii) the age clock starts at first use (`FeelessFirstSeen`).
        /// Returns `true` if the use was granted AND recorded; `false` → the caller must charge a real fee.
        pub fn try_feeless(who: &T::AccountId, ref_time: u64, allowance: u32) -> bool {
            let len = T::FeelessEpochBlocks::get();
            if len.is_zero() || allowance == 0 {
                return false;
            }
            // (i) global ceiling, in bps of max block ref-time.
            let max_block = <T as frame_system::Config>::BlockWeights::get().max_block.ref_time();
            let bps = core::cmp::min(T::FeelessBlockBps::get(), 10_000) as u64;
            let ceiling = max_block / 10_000 * bps;
            let used_block = FeelessWeightThisBlock::<T>::get();
            if used_block.saturating_add(ref_time) > ceiling {
                return false;
            }
            // (ii) per-mask budget in the current budget epoch (non-cumulative).
            let idx: u32 =
                (frame_system::Pallet::<T>::block_number() / len).saturated_into::<u32>();
            let (epoch_idx, used) = FeelessUsed::<T>::get(who);
            let used = if epoch_idx == idx { used } else { 0 };
            if used >= allowance {
                return false;
            }
            // grant + record.
            FeelessUsed::<T>::insert(who, (idx, used.saturating_add(1)));
            if !FeelessFirstSeen::<T>::contains_key(who) {
                FeelessFirstSeen::<T>::insert(who, idx);
            }
            FeelessWeightThisBlock::<T>::put(used_block.saturating_add(ref_time));
            true
        }

        /// Read-only twin of `try_feeless` (for `can_withdraw_fee` validation): would the use be granted?
        /// Checks the same ceiling + budget but records NOTHING.
        pub fn can_feeless(who: &T::AccountId, ref_time: u64, allowance: u32) -> bool {
            let len = T::FeelessEpochBlocks::get();
            if len.is_zero() || allowance == 0 {
                return false;
            }
            let max_block = <T as frame_system::Config>::BlockWeights::get().max_block.ref_time();
            let bps = core::cmp::min(T::FeelessBlockBps::get(), 10_000) as u64;
            let ceiling = max_block / 10_000 * bps;
            if FeelessWeightThisBlock::<T>::get().saturating_add(ref_time) > ceiling {
                return false;
            }
            let idx: u32 =
                (frame_system::Pallet::<T>::block_number() / len).saturated_into::<u32>();
            let (epoch_idx, used) = FeelessUsed::<T>::get(who);
            let used = if epoch_idx == idx { used } else { 0 };
            used < allowance
        }

        /// Age of `who` in FEELESS budget epochs (0 if never seen). Input to the runtime's allowance curve:
        /// only waiting ages a mask, so minting fresh masks buys no budget today (SPEC-RELAUNCH §2 R1(ii)).
        pub fn feeless_age(who: &T::AccountId) -> u32 {
            let len = T::FeelessEpochBlocks::get();
            if len.is_zero() {
                return 0;
            }
            match FeelessFirstSeen::<T>::get(who) {
                None => 0,
                Some(first) => {
                    let idx: u32 =
                        (frame_system::Pallet::<T>::block_number() / len).saturated_into::<u32>();
                    idx.saturating_sub(first)
                }
            }
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

            // Service for THIS era comes from the non-forgeable participation feed (P-2/#838), read at the
            // era boundary in `on_finalize` (after participation's fold). `epoch` is the tokens era counter,
            // bumped once per era boundary from 0: on the k-th boundary (block k·EraLength) it reads as
            // `k-1`, which is exactly the era index participation keys its accumulator by
            // (`ended_epoch_start / EraLength`) for the era that just closed — so `era_service(epoch)`
            // returns the just-ended era. (Byte-exact review point: equivalence holds while `EraLength` is
            // constant and `Epoch` starts at 0 at genesis; P-5 keeps `EraLength` a multiple of EpochLength.)
            let mut who: alloc::vec::Vec<T::AccountId> = alloc::vec::Vec::new();
            let mut weights: alloc::vec::Vec<u64> = alloc::vec::Vec::new();
            for (acct, w) in T::ServiceSource::era_service(epoch) {
                if w > 0 {
                    who.push(acct);
                    weights.push(w);
                }
            }
            Self::cap_weights(&mut weights); // §3.1 anti-Sybil cap, BEFORE the split

            // The curve is a MAXIMUM emission, never forced (SPEC-RELAUNCH §2, audit 🔴#2): with no
            // verified service this era, NOTHING is minted — the tranche simply stays unminted under the
            // hard cap. The old order (mint, then look for receivers) vaporised the era's emission into
            // `Minted` with no one credited: phantom supply that permanently ate the cap.
            let (hlq_pool, sov_pool) = if who.is_empty() {
                (0, 0)
            } else {
                let hlq_pool = Self::mint_pool(Coin::Hlq, T::HlqCurve::get(), epoch);
                let sov_pool = Self::mint_pool(Coin::Sov, T::SovCurve::get(), epoch);
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
                (hlq_pool, sov_pool)
            };

            // Clear the era's accumulator (in the feed source) and advance.
            T::ServiceSource::clear_era(epoch);
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
    use crate::{Accounts, Burned, Coin, CurveParams, Epoch, Error, Minted};
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
        type ServiceSource = MockServiceSource; // P-2: tests seed service via `seed_service`
        type FeelessBlockBps = ConstU16<1500>; // 15% of block weight for feeless
        type FeelessEpochBlocks = ConstU64<10>; // short budget epoch for tests
    }

    // Mock service source (P-2/#838): tests seed verified `(account, weight)` here; `run_epoch` reads it as
    // the era's service and `clear_era` empties it — the unit-test stand-in for pallet-participation's
    // non-forgeable accumulator, replacing the removed Root-gated `record_service` extrinsic.
    std::thread_local! {
        static SERVICE: core::cell::RefCell<Vec<(u64, u64)>> = const { core::cell::RefCell::new(Vec::new()) };
    }
    pub struct MockServiceSource;
    impl pallet_tokens::EraServiceSource<u64> for MockServiceSource {
        fn era_service(_era: u64) -> Vec<(u64, u64)> {
            SERVICE.with(|s| s.borrow().clone())
        }
        fn clear_era(_era: u64) {
            SERVICE.with(|s| s.borrow_mut().clear());
        }
    }
    /// Seed (accumulate) verified service for `who` — the P-2 replacement for the old `record_service`.
    fn seed_service(who: u64, weight: u64) {
        SERVICE.with(|s| {
            let mut v = s.borrow_mut();
            match v.iter_mut().find(|(a, _)| *a == who) {
                Some(e) => e.1 = e.1.saturating_add(weight),
                None => v.push((who, weight)),
            }
        });
    }
    /// Current seeded service of `who` (0 if none) — replaces `ServiceWeights::get` in assertions.
    fn service_of(who: u64) -> u64 {
        SERVICE.with(|s| s.borrow().iter().find(|(a, _)| *a == who).map(|(_, w)| *w).unwrap_or(0))
    }
    /// Reset the mock feed — every ext starts with no seeded service (thread_local can outlive one test).
    fn reset_service() {
        SERVICE.with(|s| s.borrow_mut().clear());
    }

    fn new_test_ext() -> TestState {
        reset_service();
        frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
    }

    fn ext_with_balances(balances: Vec<(u64, u128, u128)>) -> TestState {
        reset_service();
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

    // --- E4: existence via the sufficient ledger (the reviewer's stress catch) ---

    #[test]
    fn receiving_coin_materialises_the_account_and_spending_reaps_it() {
        // A fresh mask funded ONLY via a Tokens transfer must gain a frame_system existence reference
        // (sufficients>0), else HlqCheckNonce rejects its paying extrinsics as "not yet a mask". Spending
        // to zero releases the reference and reaps the ledger entry.
        ext_with_balances(vec![(1, 100, 0)]).execute_with(|| {
            assert_eq!(frame_system::Account::<Test>::get(2).sufficients, 0);
            assert_ok!(Tokens::transfer(RuntimeOrigin::signed(1), Coin::Hlq, 2, 40));
            assert_eq!(frame_system::Account::<Test>::get(2).sufficients, 1);
            assert!(Accounts::<Test>::contains_key(2));
            // spend it all back
            assert_ok!(Tokens::transfer(RuntimeOrigin::signed(2), Coin::Hlq, 1, 40));
            assert!(!Accounts::<Test>::contains_key(2));
            assert_eq!(frame_system::Account::<Test>::get(2).sufficients, 0);
        });
    }

    #[test]
    fn genesis_funded_account_is_materialised() {
        // Founders seeded in genesis exist without any phantom native balance — one sufficient ref each.
        ext_with_balances(vec![(1, 100, 5)]).execute_with(|| {
            assert_eq!(frame_system::Account::<Test>::get(1).sufficients, 1);
        });
    }

    #[test]
    #[should_panic(expected = "duplicate account in genesis token balances")]
    fn genesis_rejects_duplicate_balances() {
        // T-HIGH1 (audit): a chain-spec listing the same account twice would silently corrupt
        // the Minted-vs-cap accounting (Minted accumulates BOTH amounts against the hard cap while the
        // balance keeps only the last) and fire inc_sufficients twice — an irreversible corruption baked
        // into the sealed genesis. The build must reject it rather than proceed.
        let _ = ext_with_balances(vec![(1, 100, 0), (1, 50, 0)]);
    }

    #[test]
    fn partial_spend_keeps_the_account_alive() {
        // Draining one coin to zero while the other stays positive must NOT reap or drop the reference.
        ext_with_balances(vec![(1, 100, 7)]).execute_with(|| {
            assert_ok!(Tokens::transfer(RuntimeOrigin::signed(1), Coin::Hlq, 2, 100));
            // account 1 still holds SOV → stays materialised
            assert_eq!(frame_system::Account::<Test>::get(1).sufficients, 1);
            assert!(Accounts::<Test>::contains_key(1));
            assert_eq!(Tokens::balance(&1, Coin::Sov), 7);
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

    // --- proof-of-service: emission reads the non-forgeable feed, NO Root/origin (P-2/#838) ---

    #[test]
    fn emission_reads_service_source_no_origin() {
        new_test_ext().execute_with(|| {
            // No extrinsic, no Root: service arrives via the feed (participation's inherent, mocked here).
            seed_service(1, 15);
            assert_eq!(service_of(1), 15);
            Tokens::run_epoch(); // the era boundary reads the feed and mints — no privileged call anywhere
            // the whole HLQ pool (1000) went to the single provider; feed cleared afterwards
            assert_eq!(Tokens::balance(&1, Coin::Hlq), 1000);
            assert!(Minted::<Test>::get(Coin::Hlq) > 0);
            assert_eq!(service_of(1), 0, "clear_era must empty the feed after the split");
        });
    }

    // --- era reward: mint by curve, split by service, advance, clear ---

    #[test]
    fn era_mints_and_splits_by_service() {
        new_test_ext().execute_with(|| {
            // two equal-service nodes
            seed_service(1, 5);
            seed_service(2, 5);
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
            assert_eq!(service_of(1), 0);
            assert_eq!(service_of(2), 0);
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
            seed_service(1, 1);
            seed_service(2, 1);
            seed_service(3, 1);
            seed_service(9, 100);
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
                seed_service(1, 1); // run_epoch's clear_era empties the feed each iteration
                Tokens::run_epoch();
            }
            // SOV cap is 150 — cumulative minted must stop there, never above
            assert_eq!(Minted::<Test>::get(Coin::Sov), 150);
            assert!(Tokens::balance(&1, Coin::Sov) <= 150);
        });
    }

    // --- no-king: the era fires by block hook (on_finalize, P-2), not by a privileged call ---

    #[test]
    fn on_finalize_fires_only_on_era_boundary() {
        new_test_ext().execute_with(|| {
            seed_service(1, 1);
            // non-boundary block: nothing happens
            Tokens::on_finalize(3);
            assert_eq!(Epoch::<Test>::get(), 0);
            assert_eq!(Minted::<Test>::get(Coin::Hlq), 0);
            // EraLength=5 boundary: the chain mints + distributes by itself, in on_finalize
            Tokens::on_finalize(5);
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

    // --- balance-read: the primitive the typed runtime API (`TokensApi::balance_of`) exposes ---

    #[test]
    fn balance_of_absent_account_is_zero() {
        // ValueQuery: an account that holds nothing — or was never seen — reads as 0, not an error.
        // The wallet-UI / node console rely on this never panicking on an unknown id.
        new_test_ext().execute_with(|| {
            assert_eq!(Tokens::balance(&42, Coin::Hlq), 0);
            assert_eq!(Tokens::balance(&42, Coin::Sov), 0);
        });
    }

    // --- 🔴#2 fix: the curve is a MAXIMUM — an era with no verified service mints NOTHING ---

    #[test]
    fn era_without_receivers_mints_nothing() {
        new_test_ext().execute_with(|| {
            // era 0: nobody recorded service → no mint, cap untouched, era still advances
            Tokens::run_epoch();
            assert_eq!(Minted::<Test>::get(Coin::Hlq), 0, "vaporised emission into Minted");
            assert_eq!(Minted::<Test>::get(Coin::Sov), 0);
            assert_eq!(Epoch::<Test>::get(), 1);
            // next era WITH service mints and distributes normally — nothing was lost
            seed_service(1, 1);
            Tokens::run_epoch();
            assert!(Tokens::balance(&1, Coin::Hlq) > 0);
            assert_eq!(Minted::<Test>::get(Coin::Hlq), Tokens::balance(&1, Coin::Hlq));
        });
    }

    // --- feeless gate (SPEC-RELAUNCH §2): budget, rollover, global ceiling, age clock ---

    #[test]
    fn feeless_budget_and_rollover() {
        new_test_ext().execute_with(|| {
            frame_system::Pallet::<Test>::set_block_number(1);
            Tokens::on_initialize(1);
            // allowance 2: two uses granted, third refused
            assert!(Tokens::try_feeless(&1, 10, 2));
            assert!(Tokens::try_feeless(&1, 10, 2));
            assert!(!Tokens::try_feeless(&1, 10, 2), "budget must exhaust");
            // allowance 0 never grants
            assert!(!Tokens::try_feeless(&2, 10, 0));
            // next budget epoch (FeelessEpochBlocks=10): budget resets, does NOT accumulate
            frame_system::Pallet::<Test>::set_block_number(11);
            Tokens::on_initialize(11);
            assert!(Tokens::try_feeless(&1, 10, 2));
            assert!(Tokens::try_feeless(&1, 10, 2));
            assert!(!Tokens::try_feeless(&1, 10, 2), "rollover must not accumulate");
        });
    }

    #[test]
    fn feeless_global_block_ceiling() {
        new_test_ext().execute_with(|| {
            frame_system::Pallet::<Test>::set_block_number(1);
            Tokens::on_initialize(1);
            let bw: frame_system::limits::BlockWeights =
                <Test as frame_system::Config>::BlockWeights::get();
            let max = bw.max_block.ref_time();
            let ceiling = max / 10_000 * 1500; // 15%
            // one huge feeless tx fills the whole block ceiling…
            assert!(Tokens::try_feeless(&1, ceiling, 10));
            // …then NOBODY else rides free this block, even with budget left
            assert!(!Tokens::try_feeless(&2, 1, 10), "global ceiling must bind");
            // fresh block → ceiling resets
            Tokens::on_initialize(2);
            assert!(Tokens::try_feeless(&2, 1, 10));
        });
    }

    #[test]
    fn feeless_age_clock_starts_at_first_use() {
        new_test_ext().execute_with(|| {
            frame_system::Pallet::<Test>::set_block_number(1);
            assert_eq!(Tokens::feeless_age(&1), 0, "never seen = 0");
            assert!(Tokens::try_feeless(&1, 1, 1));
            // 5 budget epochs later (10 blocks each) the mask has aged 5 — only waiting ages it
            frame_system::Pallet::<Test>::set_block_number(51);
            assert_eq!(Tokens::feeless_age(&1), 5);
            // a mask minted "now" starts at 0 regardless of how many exist
            assert!(Tokens::try_feeless(&2, 1, 1));
            assert_eq!(Tokens::feeless_age(&2), 0);
        });
    }

    #[test]
    fn balance_of_reads_each_coin_independently() {
        // The two coins are separate ledgers: a read of one must never bleed into the other.
        ext_with_balances(vec![(1, 500, 200)]).execute_with(|| {
            assert_eq!(Tokens::balance(&1, Coin::Hlq), 500);
            assert_eq!(Tokens::balance(&1, Coin::Sov), 200);
            // moving HLQ leaves SOV untouched
            assert_ok!(Tokens::transfer(RuntimeOrigin::signed(1), Coin::Hlq, 2, 100));
            assert_eq!(Tokens::balance(&1, Coin::Hlq), 400);
            assert_eq!(Tokens::balance(&1, Coin::Sov), 200);
            assert_eq!(Tokens::balance(&2, Coin::Hlq), 100);
            assert_eq!(Tokens::balance(&2, Coin::Sov), 0);
        });
    }
}
