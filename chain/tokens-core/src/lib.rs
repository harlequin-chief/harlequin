//! Harlequin economic core — **two coins**, dependency-free, deterministic, integer-only.
//!
//! Spec anchor: **SPEC §3** (instrumentos económicos). The two coins (the two coin names):
//!   - **HLQ (Harlequin)** — daily-use medium of exchange. Liquid, low-friction (SPEC §3.2, option C: no
//!     peg promise — value is *moved* in HLQ, *stored* in SOV).
//!   - **SOV (Soberano)** — scarce store of value. Cap-fixed, disinflationary emission with a hard cut,
//!     no discretionary printing (SPEC §3.1, Bitcoin-style).
//!
//! **Load-bearing principle — money is NOT power (Art. VI).** This crate moves and mints *money*; it
//! confers **no** voting weight. Who governs is decided by **reputation** (`reputation-core`, §1), never by
//! balance. Two reward roles stay separate (SPEC §3.5): *governing* (consensus/committee) is by reputation
//! and lives elsewhere; *maintaining* (infrastructure) is paid here by **proof-of-service**, and that pay
//! grants no vote. Nothing in this crate reads or writes reputation.
//!
//! **No discretionary mint.** New coin enters circulation ONLY through a pre-committed
//! [`EmissionCurve`] — a public, non-discretionary rule (Art. X "sound money"). There is no "create
//! N coins" call. The curve is disinflationary with a hard cap; once the cap is reached, emission is 0
//! forever. The *numbers* (initial reward, halving period, cap) are **[PARÁMETRO]** — supplied by the
//! caller / runtime config, never decided inside this code (SPEC §3.5: "nobody decrees the numbers").
//!
//! **Deterministic.** All math is integer (`u128`) with checked arithmetic; `BTreeMap` gives a fixed
//! iteration order. The result is bit-identical across architectures — a prerequisite for consensus.
//!
//! `no_std`-ready: `default-features = false` ⇒ `no_std` (alloc only), what the runtime/pallet links.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

/// A pseudonymous account = the same key that carries reputation and signs (SPEC §3.6). The physical
/// identity is never modelled (Art. VII).
pub type Account = String;

/// Smallest indivisible unit of a coin. HLQ is divisible for daily use; SOV is the reserve. The number of
/// decimals is a runtime presentation concern, not modelled here.
pub type Amount = u128;

/// The two coins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coin {
    /// HLQ — daily-use medium of exchange (SPEC §3.2).
    Hlq,
    /// SOV — Soberano, scarce store of value (SPEC §3.1).
    Sov,
}

/// Economic errors. Every fallible operation returns one of these rather than panicking — a runtime must
/// never trap on user input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The source account does not hold enough of the coin.
    InsufficientFunds,
    /// An arithmetic operation would overflow `u128` (supply is bounded by the cap, so this signals misuse).
    Overflow,
    /// Sending to oneself, or a zero-amount transfer — rejected as a no-op to keep the ledger clean.
    InvalidTransfer,
}

/// A pre-committed, disinflationary emission schedule with a hard cap. Public and non-discretionary
/// (Art. X). Emission proceeds in **eras** of `era_length` epochs; each era the per-epoch reward is scaled
/// by the rational decay ratio `decay_num / decay_den` (so `1/2` = a Bitcoin halving, `3/4` = a flatter
/// 0.75 decay). It only ever *decreases*, and total minted can never exceed `cap` (hard cut). Numbers are
/// caller-supplied `[PARÁMETRO]`.
///
/// Why a rational ratio and not just halving: a pure halving (r=1/2) front-loads emission — >50% of the cap
/// in the first era — so early node operators capture nearly everything and later ones get crumbs, which
/// fights "earned by service over time" (SPEC §3.5). A flatter ratio (r≈3/4) spreads emission across
/// operator generations (long tail), anti early-capture (per the emission model).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmissionCurve {
    /// Per-epoch reward in era 0 (before any decay).
    pub initial: Amount,
    /// Decay numerator. Must satisfy `0 < decay_num <= decay_den` (non-increasing). `num == den` ⇒ flat.
    pub decay_num: u32,
    /// Decay denominator (`> 0`). With `num/den`: `1/2` halving, `3/4` ≈ 0.75 (recommended), `1/1` flat.
    pub decay_den: u32,
    /// Epochs per era. `0` ⇒ flat (never decays, until the cap).
    pub era_length: u64,
    /// Absolute hard cap on cumulative emission. Once reached, emission is 0 forever.
    pub cap: Amount,
}

impl EmissionCurve {
    /// The scheduled (pre-cap) reward for an epoch: `initial * (decay_num/decay_den)^(epoch/era_length)`,
    /// computed iteratively in integers (each era multiplies by `num` then divides by `den`), monotonically
    /// non-increasing and reaching 0 in the limit. The cap is applied separately in
    /// [`Ledger::mint_emission`] against the amount already minted.
    ///
    /// Assumes sane parameters (`0 < decay_num <= decay_den`, `decay_den > 0`, `initial` well below
    /// `u128::MAX`); a malformed curve degrades to a flat or zero schedule rather than panicking.
    pub fn scheduled_reward(&self, epoch: u64) -> Amount {
        if self.era_length == 0 || self.decay_num >= self.decay_den {
            return self.initial; // flat (or malformed num>=den treated as flat)
        }
        if self.decay_num == 0 || self.decay_den == 0 {
            return 0;
        }
        let eras = epoch / self.era_length;
        let (num, den) = (self.decay_num as u128, self.decay_den as u128);
        let mut r = self.initial;
        let mut e = 0u64;
        while e < eras && r > 0 {
            // num <= den ⇒ result <= r (non-increasing); saturating_mul guards the rare large-r case.
            r = r.saturating_mul(num) / den;
            e += 1;
        }
        r
    }

    /// The amount actually mintable this epoch given how much has already been minted: the scheduled
    /// reward, capped so cumulative emission never exceeds `cap` (the hard cut). Pure — the runtime reuses
    /// this so the pallet enforces the same rule as the host `Ledger`. This is the ONLY source of new coin.
    pub fn mint_amount(&self, epoch: u64, already_minted: Amount) -> Amount {
        let room = self.cap.saturating_sub(already_minted);
        core::cmp::min(self.scheduled_reward(epoch), room)
    }
}

/// Per-coin supply state: the curve, how much has been minted (to enforce the cap) and how much has been
/// burned (fee sink). Circulating supply = `minted - burned`.
#[derive(Clone, Copy, Debug)]
struct Supply {
    curve: EmissionCurve,
    minted: Amount,
    burned: Amount,
}

/// The ledger: balances of both coins per account, plus each coin's supply state. Pure data + checked
/// operations; no I/O, no time, no randomness — a runtime drives it.
#[derive(Clone, Debug, Default)]
pub struct Ledger {
    // (hlq, sov) per account. Absent account ⇒ zero of both.
    balances: BTreeMap<Account, (Amount, Amount)>,
    hlq: Option<Supply>,
    sov: Option<Supply>,
}

impl Ledger {
    /// A fresh ledger with the two emission curves fixed at genesis. After this, the only way new coin
    /// appears is [`mint_emission`](Self::mint_emission) following these curves — there is no other mint.
    pub fn new(hlq_curve: EmissionCurve, sov_curve: EmissionCurve) -> Self {
        Ledger {
            balances: BTreeMap::new(),
            hlq: Some(Supply { curve: hlq_curve, minted: 0, burned: 0 }),
            sov: Some(Supply { curve: sov_curve, minted: 0, burned: 0 }),
        }
    }

    fn slot(&self, coin: Coin, bal: &(Amount, Amount)) -> Amount {
        match coin {
            Coin::Hlq => bal.0,
            Coin::Sov => bal.1,
        }
    }

    /// Balance of `account` in `coin` (0 if the account is unknown).
    pub fn balance(&self, account: &str, coin: Coin) -> Amount {
        self.balances.get(account).map(|b| self.slot(coin, b)).unwrap_or(0)
    }

    /// Total coin in circulation = minted minus burned (the fee sink, SPEC §3.5: HLQ fees partially burned).
    pub fn total_supply(&self, coin: Coin) -> Amount {
        let s = self.supply(coin);
        s.minted.saturating_sub(s.burned)
    }

    /// Cumulative burned for `coin` (the deflationary fee sink).
    pub fn total_burned(&self, coin: Coin) -> Amount {
        self.supply(coin).burned
    }

    fn supply(&self, coin: Coin) -> &Supply {
        match coin {
            Coin::Hlq => self.hlq.as_ref().unwrap(),
            Coin::Sov => self.sov.as_ref().unwrap(),
        }
    }
    fn supply_mut(&mut self, coin: Coin) -> &mut Supply {
        match coin {
            Coin::Hlq => self.hlq.as_mut().unwrap(),
            Coin::Sov => self.sov.as_mut().unwrap(),
        }
    }

    fn credit(&mut self, account: &str, coin: Coin, amount: Amount) -> Result<(), Error> {
        let entry = self.balances.entry(account.into()).or_insert((0, 0));
        let slot = match coin {
            Coin::Hlq => &mut entry.0,
            Coin::Sov => &mut entry.1,
        };
        *slot = slot.checked_add(amount).ok_or(Error::Overflow)?;
        Ok(())
    }

    fn debit(&mut self, account: &str, coin: Coin, amount: Amount) -> Result<(), Error> {
        let entry = self.balances.get_mut(account).ok_or(Error::InsufficientFunds)?;
        let slot = match coin {
            Coin::Hlq => &mut entry.0,
            Coin::Sov => &mut entry.1,
        };
        if *slot < amount {
            return Err(Error::InsufficientFunds);
        }
        *slot -= amount;
        Ok(())
    }

    /// Move `amount` of `coin` from `from` to `to`. Checked and conservative: it never creates or destroys
    /// coin (total supply is unchanged), rejects insufficient funds, overflow, self-transfers and zero.
    pub fn transfer(&mut self, from: &str, to: &str, coin: Coin, amount: Amount) -> Result<(), Error> {
        if amount == 0 || from == to {
            return Err(Error::InvalidTransfer);
        }
        self.debit(from, coin, amount)?;
        // credit can only fail on overflow; restore the debit so the call is atomic.
        if let Err(e) = self.credit(to, coin, amount) {
            let _ = self.credit(from, coin, amount);
            return Err(e);
        }
        Ok(())
    }

    /// A transaction fee in **HLQ**, paid by `payer` to the `node` that processes it. This is the natural,
    /// inflation-free reward (SPEC §3.5a): it moves existing coin, it does **not** mint. No new supply.
    pub fn charge_fee(&mut self, payer: &str, node: &str, amount: Amount) -> Result<(), Error> {
        self.transfer(payer, node, Coin::Hlq, amount)
    }

    /// A transaction fee in HLQ with a **burn split** (SPEC §3.5 / TOKENOMICS §2): `burn_bps` basis points
    /// of the fee are burned (removed from circulating supply — the deflationary sink), the rest goes to the
    /// `node`. `burn_bps` is clamped to 10000 (100%). Burning never mints; circulating supply falls by the
    /// burned amount. Atomic: on any failure the payer is made whole.
    pub fn charge_fee_with_burn(
        &mut self,
        payer: &str,
        node: &str,
        fee: Amount,
        burn_bps: u16,
    ) -> Result<(), Error> {
        if fee == 0 {
            return Err(Error::InvalidTransfer);
        }
        let burn = burn_amount(fee, burn_bps); // floor(fee*burn_bps/10000), clamped
        let to_node = fee - burn;
        self.debit(payer, Coin::Hlq, fee)?;
        if to_node > 0 && payer != node {
            if let Err(e) = self.credit(node, Coin::Hlq, to_node) {
                let _ = self.credit(payer, Coin::Hlq, fee); // restore
                return Err(e);
            }
        } else if payer == node {
            // paying yourself: only the burn actually leaves the payer.
            let _ = self.credit(payer, Coin::Hlq, to_node);
        }
        if burn > 0 {
            let s = self.supply_mut(Coin::Hlq);
            s.burned = s.burned.saturating_add(burn);
        }
        Ok(())
    }

    /// Mint this epoch's emission for `coin` to `beneficiary`, following the pre-committed curve and the
    /// hard cap. Returns the amount actually minted (which is `min(scheduled_reward(epoch), cap - minted)`,
    /// i.e. 0 once the cap is reached). This is the ONLY path that increases supply, and it is
    /// rule-bound — there is no discretionary mint anywhere (SPEC §3.5, Art. X).
    pub fn mint_emission(&mut self, coin: Coin, epoch: u64, beneficiary: &str) -> Result<Amount, Error> {
        let (amount, minted) = {
            let s = self.supply(coin);
            (s.curve.mint_amount(epoch, s.minted), s.minted)
        };
        if amount == 0 {
            return Ok(0);
        }
        self.credit(beneficiary, coin, amount)?;
        self.supply_mut(coin).minted = minted.checked_add(amount).ok_or(Error::Overflow)?;
        Ok(amount)
    }
}

/// Split an infrastructure-reward `pool` in proportion to per-node **verified service** weights
/// (proof-of-service, SPEC §3.5(2)). **Account-agnostic and index-aligned**: the caller passes weights in
/// some fixed order (e.g. iterating a `BTreeMap<AccountId,_>` for determinism) and gets shares back in the
/// SAME order, so any account type maps by position. Pure and deterministic: proportional floor shares with
/// the integer remainder handed to the highest-weight index.
///
/// The caller MUST have already (a) verified the work and (b) applied the per-node anti-Sybil **cap**
/// (TOKENOMICS §3.1: `weight_i = min(weight_i, ceil(median·k))`, k≈2) when building this vector — this
/// function is a pure divider over already-bounded weights and deliberately does not cap. This pay grants
/// **no voting power** (Art. VI).
pub fn split_service_reward(pool: Amount, weights: &[u64]) -> Vec<Amount> {
    let total: u128 = weights.iter().map(|w| *w as u128).sum();
    if total == 0 || pool == 0 {
        return alloc::vec![0; weights.len()];
    }
    let mut out: Vec<Amount> = Vec::with_capacity(weights.len());
    let mut distributed: Amount = 0;
    for w in weights {
        let share = pool.saturating_mul(*w as u128) / total;
        distributed = distributed.saturating_add(share);
        out.push(share);
    }
    let remainder = pool.saturating_sub(distributed);
    if remainder > 0 {
        if let Some(idx) = max_weight_index(weights) {
            out[idx] = out[idx].saturating_add(remainder);
        }
    }
    out
}

fn max_weight_index(weights: &[u64]) -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    for (i, w) in weights.iter().enumerate() {
        match best {
            Some((_, bw)) if *w <= bw => {}
            _ => best = Some((i, *w)),
        }
    }
    best.map(|(i, _)| i)
}

/// `floor(fee * burn_bps / 10000)`, overflow-safe, with `burn_bps` clamped to 10000 (100%). The burned
/// part of a fee (TOKENOMICS §2 sink); the rest goes to the node. Pure helper reused by the runtime.
pub fn burn_amount(fee: Amount, burn_bps: u16) -> Amount {
    let bps = if burn_bps > 10_000 { 10_000u128 } else { burn_bps as u128 };
    fee / 10_000 * bps + (fee % 10_000) * bps / 10_000
}

#[cfg(test)]
mod tests {
    use super::*;

    // A halving curve (r = 1/2) — the era_length is the epochs between halvings.
    fn curve(initial: Amount, period: u64, cap: Amount) -> EmissionCurve {
        EmissionCurve { initial, decay_num: 1, decay_den: 2, era_length: period, cap }
    }
    fn ledger() -> Ledger {
        // HLQ: large initial, fast halving, big cap. SOV: smaller, slower, hard cap (Bitcoin-like).
        Ledger::new(curve(1000, 2, 1_000_000), curve(50, 4, 2_000))
    }

    #[test]
    fn transfer_moves_and_conserves() {
        let mut l = ledger();
        l.mint_emission(Coin::Hlq, 0, "alice").unwrap();
        let before = l.total_supply(Coin::Hlq);
        l.transfer("alice", "bob", Coin::Hlq, 300).unwrap();
        assert_eq!(l.balance("bob", Coin::Hlq), 300);
        assert_eq!(l.balance("alice", Coin::Hlq), 700);
        assert_eq!(l.total_supply(Coin::Hlq), before); // transfer never changes supply
    }

    #[test]
    fn transfer_rejects_insufficient_self_and_zero() {
        let mut l = ledger();
        l.mint_emission(Coin::Hlq, 0, "alice").unwrap();
        assert_eq!(l.transfer("alice", "bob", Coin::Hlq, 99_999), Err(Error::InsufficientFunds));
        assert_eq!(l.transfer("alice", "alice", Coin::Hlq, 10), Err(Error::InvalidTransfer));
        assert_eq!(l.transfer("alice", "bob", Coin::Hlq, 0), Err(Error::InvalidTransfer));
        assert_eq!(l.transfer("ghost", "bob", Coin::Hlq, 1), Err(Error::InsufficientFunds));
    }

    #[test]
    fn coins_are_independent() {
        let mut l = ledger();
        l.mint_emission(Coin::Hlq, 0, "alice").unwrap();
        l.mint_emission(Coin::Sov, 0, "alice").unwrap();
        assert_eq!(l.balance("alice", Coin::Hlq), 1000);
        assert_eq!(l.balance("alice", Coin::Sov), 50);
        l.transfer("alice", "bob", Coin::Sov, 50).unwrap();
        assert_eq!(l.balance("alice", Coin::Sov), 0);
        assert_eq!(l.balance("alice", Coin::Hlq), 1000); // untouched
    }

    #[test]
    fn emission_is_monotonic_non_increasing() {
        let c = curve(1000, 2, u128::MAX);
        let mut prev = c.scheduled_reward(0);
        for e in 1..40u64 {
            let r = c.scheduled_reward(e);
            assert!(r <= prev, "reward must never rise: epoch {} {} > {}", e, r, prev);
            prev = r;
        }
        assert_eq!(c.scheduled_reward(0), 1000);
        assert_eq!(c.scheduled_reward(2), 500); // first halving at epoch 2
        assert_eq!(c.scheduled_reward(4), 250);
        assert_eq!(c.scheduled_reward(1000), 0); // shifted to nothing
    }

    #[test]
    fn rational_decay_is_flatter_than_halving() {
        // Design recommendation: r = 3/4 spreads emission across operator generations vs the front-loaded
        // halving. Same initial + era_length; the 0.75 curve must decay slower (higher reward each later era).
        let halving = EmissionCurve { initial: 1000, decay_num: 1, decay_den: 2, era_length: 1, cap: u128::MAX };
        let flat75 = EmissionCurve { initial: 1000, decay_num: 3, decay_den: 4, era_length: 1, cap: u128::MAX };
        assert_eq!(flat75.scheduled_reward(0), 1000);
        assert_eq!(flat75.scheduled_reward(1), 750); // 1000 * 3/4
        assert_eq!(flat75.scheduled_reward(2), 562); // 750 * 3/4 (floor)
        // After several eras the flat curve still pays more than the halving (long tail, anti early-capture).
        for e in 1..12u64 {
            assert!(flat75.scheduled_reward(e) >= halving.scheduled_reward(e));
        }
        assert!(flat75.scheduled_reward(5) > halving.scheduled_reward(5));
        // Still monotonically non-increasing.
        let mut prev = flat75.scheduled_reward(0);
        for e in 1..30u64 {
            let r = flat75.scheduled_reward(e);
            assert!(r <= prev);
            prev = r;
        }
    }

    #[test]
    fn flat_curve_when_era_zero_or_num_eq_den() {
        let flat = EmissionCurve { initial: 500, decay_num: 1, decay_den: 1, era_length: 1, cap: u128::MAX };
        assert_eq!(flat.scheduled_reward(0), 500);
        assert_eq!(flat.scheduled_reward(100), 500); // num==den ⇒ never decays
        let flat2 = EmissionCurve { initial: 500, decay_num: 1, decay_den: 2, era_length: 0, cap: u128::MAX };
        assert_eq!(flat2.scheduled_reward(100), 500); // era_length 0 ⇒ flat
    }

    #[test]
    fn fee_is_a_transfer_no_mint() {
        let mut l = ledger();
        l.mint_emission(Coin::Hlq, 0, "user").unwrap();
        let supply = l.total_supply(Coin::Hlq);
        l.charge_fee("user", "node", 10).unwrap();
        assert_eq!(l.balance("node", Coin::Hlq), 10);
        assert_eq!(l.total_supply(Coin::Hlq), supply); // fees mint nothing
    }

    #[test]
    fn fee_burn_splits_and_deflates() {
        let mut l = ledger();
        l.mint_emission(Coin::Hlq, 0, "user").unwrap(); // 1000
        let before = l.total_supply(Coin::Hlq);
        // 50/50 burn on a 100 fee: node gets 50, 50 burned (circulating supply drops by 50).
        l.charge_fee_with_burn("user", "node", 100, 5000).unwrap();
        assert_eq!(l.balance("node", Coin::Hlq), 50);
        assert_eq!(l.balance("user", Coin::Hlq), 900);
        assert_eq!(l.total_burned(Coin::Hlq), 50);
        assert_eq!(l.total_supply(Coin::Hlq), before - 50); // deflationary
        // 100% burn: node gets nothing, all 20 leaves circulation.
        l.charge_fee_with_burn("user", "node", 20, 10000).unwrap();
        assert_eq!(l.balance("node", Coin::Hlq), 50);
        assert_eq!(l.total_burned(Coin::Hlq), 70);
        // burn_bps clamped; zero fee rejected.
        assert_eq!(l.charge_fee_with_burn("user", "node", 0, 5000), Err(Error::InvalidTransfer));
    }

    #[test]
    fn emission_respects_hard_cap() {
        // SOV cap 2000; with initial 50 halving every 4 epochs, cumulative converges below cap but the cap
        // is the hard ceiling. Use a curve that would blow past the cap to prove the cut.
        let mut l = Ledger::new(curve(1, 1, 1), curve(10_000, 1, 2_000));
        let mut total = 0;
        for e in 0..10u64 {
            total += l.mint_emission(Coin::Sov, e, "node").unwrap();
        }
        assert_eq!(total, 2000); // never exceeds cap
        assert_eq!(l.total_supply(Coin::Sov), 2000);
        // Past the cap, emission is 0 forever.
        assert_eq!(l.mint_emission(Coin::Sov, 11, "node").unwrap(), 0);
    }

    #[test]
    fn only_emission_increases_supply() {
        let mut l = ledger();
        // No public mint function exists beyond mint_emission; a transfer of unfunded coin fails.
        assert_eq!(l.transfer("nobody", "alice", Coin::Hlq, 1), Err(Error::InsufficientFunds));
        assert_eq!(l.total_supply(Coin::Hlq), 0);
        let minted = l.mint_emission(Coin::Hlq, 0, "node").unwrap();
        assert_eq!(l.total_supply(Coin::Hlq), minted);
    }

    #[test]
    fn service_split_is_proportional_and_exact() {
        let shares = split_service_reward(100, &[1, 1, 2]); // index-aligned shares
        let sum: Amount = shares.iter().sum();
        assert_eq!(sum, 100); // exact: remainder handed out, nothing lost
        assert_eq!(shares[2], 50); // half the weight ⇒ 50
        assert_eq!(shares[0], 25);
        assert_eq!(shares[1], 25);
    }

    #[test]
    fn service_split_remainder_to_top_weight() {
        let shares = split_service_reward(10, &[1, 2]); // 10*1/3=3, 10*2/3=6, remainder 1 → to index 1
        assert_eq!(shares[0], 3);
        assert_eq!(shares[1], 7);
        assert_eq!(shares[0] + shares[1], 10);
    }

    #[test]
    fn service_split_zero_pool_or_weights() {
        assert!(split_service_reward(100, &[0, 0]).iter().all(|s| *s == 0));
        assert!(split_service_reward(0, &[5]).iter().all(|s| *s == 0));
        assert_eq!(split_service_reward(100, &[]).len(), 0);
    }

    #[test]
    fn burn_amount_floor_and_clamp() {
        assert_eq!(burn_amount(100, 5000), 50);
        assert_eq!(burn_amount(100, 10_000), 100);
        assert_eq!(burn_amount(100, 20_000), 100); // clamped to 100%
        assert_eq!(burn_amount(1, 5000), 0); // floor
        assert_eq!(burn_amount(0, 5000), 0);
    }

    #[test]
    fn mint_amount_matches_ledger() {
        let c = curve(1000, 2, 1500);
        assert_eq!(c.mint_amount(0, 0), 1000);
        assert_eq!(c.mint_amount(0, 1000), 500); // capped to remaining room
        assert_eq!(c.mint_amount(0, 1500), 0); // cap reached
    }
}
